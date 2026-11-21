//! #140: GitLab Job Hook ingestion and busy tracking, end to end through the
//! real routers: `crate::public_routes()` (which must serve
//! `POST /api/v1/webhooks/gitlab` without a session) merged with
//! `crate::session_routes()` (for `POST /api/v1/webhooks/{delivery_id}/retry`),
//! with `FakeGitLab` injected through `AppState::with_gitlab`.
//!
//! Every test builds its own migrated temporary database, seeds GitLab
//! connections (their access token and webhook secret sealed in
//! `runtime_secrets`, as #138 stores them), a pool and GitLab runners, then
//! sends Job Hook deliveries shaped like GitLab's documented payload
//! (`doc/user/project/integrations/webhook_events.md`, "Job events").
//!
//! The rules pinned here are `docs/handoffs/140-brief.md` as amended through
//! architecture review round 4, including D18's status rule (the shared
//! writer itself is pinned by `gridops-core`'s `gitlab/jobs_tests.rs`).
//!
//! `FakeGitLab` keys its state by base URL and never sees the token, so "the
//! matched connection's credentials" is checked as "the `job` lookup went to
//! that connection's GitLab instance". The connections here use host-only
//! base URLs, so the trailing-slash form `Url` gives them is unambiguous.

use std::{path::PathBuf, sync::Arc};

use anyhow::{Context as _, Result, anyhow};
use axum::{
    Router,
    body::Body,
    http::{Method, Request, StatusCode, header},
};
use gridops_core::{
    BitbucketClient, Config, GitHubClient, Vault,
    fleet::ExternalId,
    gitlab::{
        GitLabError, Job, JobStatus,
        fake::{FakeGitLab, FakeMethod},
    },
};
use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};
use sqlx::SqlitePool;
use tower::ServiceExt as _;
use url::Url;

use crate::state::AppState;

// The spec checker #138's tests also include: `resources::gitlab_tests` is a
// private module, so this one includes the file again.
#[allow(clippy::duplicate_mod)]
#[path = "../../tests/support/openapi_schema.rs"]
mod openapi_schema;

const PUBLIC: &str = "https://gridops.example";
const WEBHOOK: &str = "/api/v1/webhooks/gitlab";

const GITLAB_A: &str = "https://gitlab-a.example.test";
const GITLAB_B: &str = "https://gitlab-b.example.test";

/// A project connection for project 11 (`team/app`) on instance A.
const CONN_A: &str = "d1000000-0000-4000-8000-00000000000a";
const SECRET_A: &str = "webhook-secret-a";
/// A second connection: on instance B, or a group connection, per test.
const CONN_B: &str = "d1000000-0000-4000-8000-00000000000b";
const SECRET_B: &str = "webhook-secret-b";
/// A third connection, for the multiple-candidate tests.
const CONN_C: &str = "d1000000-0000-4000-8000-00000000000c";
const SECRET_C: &str = "webhook-secret-c";

const PROJECT: u64 = 11;
const PROJECT_PATH: &str = "team/app";
const PIPELINE: u64 = 7;
const POOL: &str = "pool-gitlab";

/// GitLab runner 55: the one the jobs below run on.
const RUNNER_ID: u64 = 55;

/// A status time (epoch ms) after every seeded time and before the test runs.
const T: i64 = 1_795_000_000_000;

struct Harness {
    router: Router,
    vault: Vault,
    gitlab: FakeGitLab,
    database: SqlitePool,
    path: PathBuf,
    admin: String,
}

impl Drop for Harness {
    fn drop(&mut self) {
        for suffix in ["", "-shm", "-wal"] {
            let _ = std::fs::remove_file(format!("{}{suffix}", self.path.display()));
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
struct JobRow {
    connection_id: String,
    project_id: i64,
    job_id: i64,
    pipeline_id: i64,
    status: String,
    tag_list: String,
    status_changed_at: i64,
    retries_count: i64,
    gitlab_runner_id: Option<i64>,
    pool_id: Option<String>,
    updated_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
struct RunnerRow {
    id: String,
    status: String,
    busy: bool,
    current_job_id: Option<i64>,
    last_job_id: Option<i64>,
    github_runner_id: Option<i64>,
    last_heartbeat_at: Option<i64>,
    updated_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
struct EventRow {
    runner_id: Option<String>,
    pool_id: Option<String>,
    level: String,
    event: String,
    message: String,
    metadata: String,
}

#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
struct DeliveryRow {
    id: String,
    event: String,
    action: Option<String>,
    hook_id: Option<i64>,
    installation_id: Option<i64>,
    repository_id: Option<i64>,
    signature_valid: bool,
    status: String,
}

fn url(value: &str) -> Result<Url> {
    Url::parse(value).map_err(|error| anyhow!("{value} is not a URL: {error}"))
}

fn ext(value: u64) -> Result<ExternalId> {
    ExternalId::new(value).map_err(|error| anyhow!("{error}"))
}

/// `ms` as GitLab writes `build_status_changed_at`: RFC 3339, milliseconds, `Z`.
fn rfc3339(ms: i64) -> String {
    chrono::DateTime::from_timestamp_millis(ms)
        .map(|time| time.to_rfc3339_opts(chrono::SecondsFormat::Millis, true))
        .unwrap_or_default()
}

/// A Job Hook payload for job `job_id` of project 11 (`team/app`) in
/// pipeline 7, as GitLab documents it: no runner, and `status` since `at`
/// (`None` writes `build_status_changed_at: null`).
fn job_event(job_id: u64, status: &str, at: Option<i64>) -> Value {
    json!({
        "object_kind": "build",
        "ref": "main",
        "tag": false,
        "retries_count": 0,
        "build_id": job_id,
        "build_name": "rspec",
        "build_stage": "test",
        "build_status": status,
        "build_status_changed_at": at.map(rfc3339),
        "build_created_at": rfc3339(T - 60_000),
        "pipeline_id": PIPELINE,
        "runner": null,
        "project_id": PROJECT,
        "project_name": "team / app",
        "project": {
            "id": PROJECT,
            "name": "app",
            "web_url": "https://gitlab-a.example.test/team/app",
            "path_with_namespace": PROJECT_PATH,
        },
    })
}

/// The same event, run by GitLab runner `runner_id`.
fn on_runner(mut event: Value, runner_id: u64) -> Value {
    event["runner"] = json!({
        "id": runner_id,
        "description": "gridops runner",
        "runner_type": "project_type",
        "active": true,
        "is_shared": false,
        "tags": ["docker"],
    });
    event
}

/// The same event for another project.
fn in_project(mut event: Value, project_id: u64, path: &str) -> Value {
    event["project_id"] = json!(project_id);
    event["project"]["id"] = json!(project_id);
    event["project"]["path_with_namespace"] = json!(path);
    event
}

fn tags(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| (*value).to_owned()).collect()
}

fn error_text(body: &Value) -> String {
    body["error"].as_str().unwrap_or_default().to_owned()
}

async fn harness() -> Result<Harness> {
    let path = std::env::temp_dir().join(format!(
        "gridops-gitlab-webhook-{}.sqlite",
        uuid::Uuid::new_v4()
    ));
    let database = gridops_core::connect_database_path(&path).await?;
    // The manager is unreachable: a test that reaches it fails fast.
    let config = Config::from_env()?
        .with_base_url(url(PUBLIC)?)
        .with_manager(url("http://127.0.0.1:9")?, "test-manager-credential");
    let vault = Vault::new([9_u8; 32], b"gitlab-webhook-test-session".to_vec());
    let github = GitHubClient::new(config.clone())?;
    let bitbucket = BitbucketClient::new()?;
    let gitlab = FakeGitLab::default();
    let state = AppState::new(config, database.clone(), vault.clone(), github, bitbucket)?
        .with_gitlab(Arc::new(gitlab.clone()));
    let router = crate::public_routes()
        .merge(crate::session_routes())
        .with_state(state);
    sqlx::raw_sql(
        r#"
        INSERT INTO installations (id,account_id,account_login,account_type,target_type,repository_selection,created_at,updated_at)
          VALUES (42,42,'fixture','Organization','Organization','all',1,1);
        INSERT INTO runner_pools (id,installation_id,name,scope,labels,image,created_at,updated_at)
          VALUES ('pool-gitlab',42,'gitlab','gitlab','[]','runner:latest',1,1);
        "#,
    )
    .execute(&database)
    .await?;
    let mut harness = Harness {
        router,
        vault,
        gitlab,
        database,
        path,
        admin: String::new(),
    };
    harness.admin = harness.sign_in_admin().await?;
    Ok(harness)
}

impl Harness {
    /// Seeds a system administrator with a session; returns the cookie.
    async fn sign_in_admin(&self) -> Result<String> {
        let now = gridops_core::now_millis();
        sqlx::query("INSERT INTO users (id,github_id,login,role,access_token,last_login_at,created_at,updated_at) VALUES ('admin-1',1,'admin-1','admin','',?,?,?)")
            .bind(now).bind(now).bind(now)
            .execute(&self.database).await?;
        let token = "session-admin-1".to_owned();
        sqlx::query("INSERT INTO sessions (id,token_hash,user_id,expires_at,last_seen_at,created_at) VALUES ('session-row-admin-1',?,'admin-1',?,?,?)")
            .bind(gridops_core::crypto::hash_token(&token))
            .bind(now + 3_600_000)
            .bind(now)
            .bind(now)
            .execute(&self.database)
            .await?;
        Ok(format!(
            "gridops_session={token}.{}",
            self.vault.sign(&token)?
        ))
    }

    /// Seeds a GitLab connection and its two sealed secrets, as #138 stores
    /// them. `kind` is `project` or `group`.
    #[allow(clippy::too_many_arguments)]
    async fn connection(
        &self,
        id: &str,
        base_url: &str,
        kind: &str,
        scope_id: i64,
        scope_path: &str,
        webhook_secret: &str,
        created_at: i64,
    ) -> Result<()> {
        let token_key = format!("gitlab.{id}.token");
        let secret_key = format!("gitlab.{id}.webhook");
        for (key, value) in [
            (&token_key, "glpat-fixture-token-0123456789"),
            (&secret_key, webhook_secret),
        ] {
            sqlx::query(
                "INSERT INTO runtime_secrets (key,value,updated_by,updated_at) VALUES (?,?,NULL,1)",
            )
            .bind(key)
            .bind(self.vault.seal(value)?)
            .execute(&self.database)
            .await?;
        }
        sqlx::query("INSERT INTO gitlab_connections (id,name,base_url,scope_kind,scope_id,scope_path,access_token_key,webhook_secret_key,created_by,created_at,updated_at) VALUES (?,?,?,?,?,?,?,?,NULL,?,?)")
            .bind(id)
            .bind(format!("connection {id}"))
            .bind(base_url)
            .bind(kind)
            .bind(scope_id)
            .bind(scope_path)
            .bind(&token_key)
            .bind(&secret_key)
            .bind(created_at)
            .bind(created_at)
            .execute(&self.database)
            .await?;
        Ok(())
    }

    /// The usual setup: connection A for project 11 on instance A.
    async fn connection_a(&self) -> Result<()> {
        self.connection(CONN_A, GITLAB_A, "project", 11, PROJECT_PATH, SECRET_A, 1)
            .await
    }

    /// Seeds a live, idle GitLab runner of the pool, registered through
    /// `connection_id` as GitLab runner `gitlab_runner_id`.
    async fn runner(&self, id: &str, connection_id: &str, gitlab_runner_id: u64) -> Result<()> {
        sqlx::query("INSERT INTO runners (id,pool_id,name,provider,status,busy,ci_platform,gitlab_connection_id,gitlab_runner_id,last_heartbeat_at,created_at,updated_at) VALUES (?,?,?,'docker','online',0,'gitlab',?,?,1,1,1)")
            .bind(id)
            .bind(POOL)
            .bind(id)
            .bind(connection_id)
            .bind(i64::try_from(gitlab_runner_id)?)
            .execute(&self.database)
            .await?;
        Ok(())
    }

    /// Seeds job `job_id` of project `project_id` on the fake instance, so
    /// `GitLabApi::job` answers it with `tag_list`.
    fn gitlab_job(
        &self,
        base_url: &str,
        project_id: u64,
        job_id: u64,
        tag_list: &[&str],
    ) -> Result<()> {
        self.gitlab.with_job(
            &url(base_url)?,
            Job {
                id: ext(job_id)?,
                project_id: ext(project_id)?,
                pipeline_id: ext(PIPELINE)?,
                status: JobStatus::Pending,
                tag_list: tags(tag_list),
            },
        );
        Ok(())
    }

    /// Seeds a `gitlab_jobs` row of connection A as #148's poll writes it:
    /// no runner, tagged `docker`, `status` since `at`.
    async fn polled_row(&self, job_id: i64, status: &str, at: i64) -> Result<JobRow> {
        sqlx::query("INSERT INTO gitlab_jobs (id,connection_id,project_id,job_id,pipeline_id,status,tag_list,status_changed_at,retries_count,gitlab_runner_id,pool_id,created_at,updated_at) VALUES (?,?,11,?,7,?,'[\"docker\"]',?,0,NULL,NULL,1,1)")
            .bind(uuid::Uuid::new_v4().to_string())
            .bind(CONN_A)
            .bind(job_id)
            .bind(status)
            .bind(at)
            .execute(&self.database)
            .await?;
        self.job_row(CONN_A, job_id)
            .await?
            .with_context(|| format!("seeded job {job_id}"))
    }

    /// Posts raw bytes to the GitLab webhook with `headers`.
    async fn post(&self, headers: &[(&str, &str)], body: Vec<u8>) -> Result<(StatusCode, Value)> {
        let mut request = Request::builder()
            .method(Method::POST)
            .uri(WEBHOOK)
            .header(header::CONTENT_TYPE, "application/json");
        for (name, value) in headers {
            request = request.header(*name, *value);
        }
        self.respond(request.body(Body::from(body))?).await
    }

    /// Sends `event` as a Job Hook with `token` (none when `None`) and the
    /// delivery key `key` in `Idempotency-Key`.
    async fn job_hook(
        &self,
        token: Option<&str>,
        key: &str,
        event: &Value,
    ) -> Result<(StatusCode, Value)> {
        let mut headers = vec![("x-gitlab-event", "Job Hook"), ("idempotency-key", key)];
        if let Some(token) = token {
            headers.push(("x-gitlab-token", token));
        }
        self.post(&headers, event.to_string().into_bytes()).await
    }

    /// Sends `event` with `token` under a fresh delivery key and requires a
    /// 202; returns the key.
    async fn deliver(&self, token: &str, event: &Value) -> Result<String> {
        let key = uuid::Uuid::new_v4().to_string();
        let (status, body) = self.job_hook(Some(token), &key, event).await?;
        anyhow::ensure!(
            status == StatusCode::ACCEPTED,
            "delivering {event}: {status} {body}"
        );
        Ok(key)
    }

    /// Retries delivery `id` as the system administrator.
    async fn retry(&self, id: &str) -> Result<(StatusCode, Value)> {
        let request = Request::builder()
            .method(Method::POST)
            .uri(format!("/api/v1/webhooks/{id}/retry"))
            .header(header::ORIGIN, PUBLIC)
            .header(header::COOKIE, &self.admin)
            .body(Body::empty())?;
        self.respond(request).await
    }

    async fn respond(&self, request: Request<Body>) -> Result<(StatusCode, Value)> {
        let response = self.router.clone().oneshot(request).await?;
        let status = response.status();
        let bytes = axum::body::to_bytes(response.into_body(), 4 * 1024 * 1024).await?;
        let body = if bytes.is_empty() {
            Value::Null
        } else {
            serde_json::from_slice(&bytes)
                .unwrap_or_else(|_| Value::String(String::from_utf8_lossy(&bytes).into_owned()))
        };
        Ok((status, body))
    }

    async fn job_row(&self, connection_id: &str, job_id: i64) -> Result<Option<JobRow>> {
        Ok(sqlx::query_as::<_, JobRow>(
            "SELECT connection_id,project_id,job_id,pipeline_id,status,tag_list,status_changed_at,
                    retries_count,gitlab_runner_id,pool_id,updated_at
               FROM gitlab_jobs WHERE connection_id=? AND job_id=?",
        )
        .bind(connection_id)
        .bind(job_id)
        .fetch_optional(&self.database)
        .await?)
    }

    async fn job_a(&self, job_id: i64) -> Result<JobRow> {
        self.job_row(CONN_A, job_id)
            .await?
            .with_context(|| format!("job {job_id} of connection A is recorded"))
    }

    async fn jobs(&self) -> Result<Vec<JobRow>> {
        Ok(sqlx::query_as::<_, JobRow>(
            "SELECT connection_id,project_id,job_id,pipeline_id,status,tag_list,status_changed_at,
                    retries_count,gitlab_runner_id,pool_id,updated_at
               FROM gitlab_jobs ORDER BY connection_id,job_id",
        )
        .fetch_all(&self.database)
        .await?)
    }

    async fn runner_row(&self, id: &str) -> Result<RunnerRow> {
        Ok(sqlx::query_as::<_, RunnerRow>(
            "SELECT id,status,busy,current_job_id,last_job_id,github_runner_id,last_heartbeat_at,updated_at
               FROM runners WHERE id=?",
        )
        .bind(id)
        .fetch_one(&self.database)
        .await?)
    }

    async fn runners(&self) -> Result<Vec<RunnerRow>> {
        Ok(sqlx::query_as::<_, RunnerRow>(
            "SELECT id,status,busy,current_job_id,last_job_id,github_runner_id,last_heartbeat_at,updated_at
               FROM runners ORDER BY id",
        )
        .fetch_all(&self.database)
        .await?)
    }

    /// Every `runner_events` row, oldest first.
    async fn events(&self) -> Result<Vec<EventRow>> {
        Ok(sqlx::query_as::<_, EventRow>(
            "SELECT runner_id,pool_id,level,event,message,metadata FROM runner_events
              ORDER BY created_at,rowid",
        )
        .fetch_all(&self.database)
        .await?)
    }

    async fn delivery(&self, id: &str) -> Result<Option<DeliveryRow>> {
        Ok(sqlx::query_as::<_, DeliveryRow>(
            "SELECT id,event,action,hook_id,installation_id,repository_id,signature_valid,status
               FROM webhook_deliveries WHERE id=?",
        )
        .bind(id)
        .fetch_optional(&self.database)
        .await?)
    }

    async fn delivery_ids(&self) -> Result<Vec<String>> {
        Ok(
            sqlx::query_scalar("SELECT id FROM webhook_deliveries ORDER BY id")
                .fetch_all(&self.database)
                .await?,
        )
    }

    async fn delivery_status(&self, id: &str) -> Result<String> {
        Ok(self
            .delivery(id)
            .await?
            .with_context(|| format!("delivery {id} is recorded"))?
            .status)
    }

    /// The instances `GitLabApi::job` was called on, in order.
    fn job_lookups(&self) -> Vec<Url> {
        self.gitlab
            .calls()
            .into_iter()
            .filter(|call| call.method == FakeMethod::Job)
            .map(|call| call.base_url)
            .collect()
    }
}

/// The delivery id of key `key` accepted for connection `connection_id`.
fn accepted_id(connection_id: &str, key: &str) -> String {
    format!("gitlab:{connection_id}:{key}")
}

/// The `runner_events` row a busy change on runner `runner_id` writes.
fn busy_event(
    runner_id: &str,
    status: &str,
    message: &str,
    job_id: u64,
    connection_id: &str,
) -> EventRow {
    EventRow {
        runner_id: Some(runner_id.to_owned()),
        pool_id: Some(POOL.to_owned()),
        level: "info".to_owned(),
        event: format!("GitLab job {status}"),
        message: message.to_owned(),
        metadata: json!({
            "jobId": job_id,
            "projectId": PROJECT,
            "pipelineId": PIPELINE,
            "connectionId": connection_id,
        })
        .to_string(),
    }
}

/// Compares event rows with their metadata parsed, so key order is free.
fn same_events(actual: &[EventRow], expected: &[EventRow]) -> Result<()> {
    let normalise = |rows: &[EventRow]| -> Result<Vec<(EventRow, Value)>> {
        rows.iter()
            .map(|row| {
                let metadata: Value = serde_json::from_str(&row.metadata)
                    .with_context(|| format!("metadata of {row:?}"))?;
                Ok((
                    EventRow {
                        metadata: String::new(),
                        ..row.clone()
                    },
                    metadata,
                ))
            })
            .collect()
    };
    assert_eq!(normalise(actual)?, normalise(expected)?);
    Ok(())
}

// ---------------------------------------------------------------------------
// Authentication and the delivery record
// ---------------------------------------------------------------------------

/// Done when: "a bad token". W-4: a rejected delivery is recorded as
/// `gitlab:unmatched:<key>` with `signature_valid` false, and `retry` refuses
/// it like any unverified row.
#[tokio::test]
async fn a_bad_or_missing_token_is_a_401_recorded_as_rejected_that_cannot_be_retried() -> Result<()>
{
    let h = harness().await?;
    h.connection_a().await?;
    h.gitlab_job(GITLAB_A, PROJECT, 100, &["docker"])?;
    let event = job_event(100, "pending", Some(T));

    let (status, body) = h
        .job_hook(Some("not-the-secret"), "bad-token", &event)
        .await?;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "{body}");
    openapi_schema::check_response(WEBHOOK, "post", 401, &body)?;
    let rejected = h
        .delivery("gitlab:unmatched:bad-token")
        .await?
        .context("the rejected delivery is recorded as gitlab:unmatched:<key>")?;
    assert!(!rejected.signature_valid);
    assert_eq!(rejected.status, "rejected");

    let (status, body) = h.job_hook(None, "no-token", &event).await?;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "without a token: {body}");

    assert_eq!(h.jobs().await?, vec![], "a rejected delivery writes no job");
    assert_eq!(h.job_lookups(), Vec::<Url>::new(), "and calls no GitLab");

    let (status, body) = h.retry("gitlab:unmatched:bad-token").await?;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert!(
        error_text(&body).contains("Only verified webhook payloads can be retried"),
        "{body}"
    );
    assert_eq!(h.jobs().await?, vec![]);
    Ok(())
}

/// Done when: "an unknown project". The token is a real connection's secret,
/// so the 401 comes from the project lookup, not the token.
#[tokio::test]
async fn an_unknown_project_is_a_401_recorded_as_rejected() -> Result<()> {
    let h = harness().await?;
    h.connection_a().await?;
    let event = in_project(job_event(100, "created", Some(T)), 999, "elsewhere/app");

    let (status, body) = h.job_hook(Some(SECRET_A), "unknown", &event).await?;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "{body}");
    let rejected = h
        .delivery("gitlab:unmatched:unknown")
        .await?
        .context("the rejected delivery is recorded")?;
    assert_eq!(
        (rejected.signature_valid, rejected.status.as_str()),
        (false, "rejected")
    );
    assert_eq!(h.jobs().await?, vec![]);
    Ok(())
}

/// Done when: "a pending job looked up for tags". The payload carries only the
/// runner's tags, so the job's own come from `GitLabApi::job` on the matched
/// connection's instance. Also pins the delivery row and the 202 shape.
#[tokio::test]
async fn a_pending_job_is_looked_up_for_its_tags_and_recorded() -> Result<()> {
    let h = harness().await?;
    h.connection_a().await?;
    h.gitlab_job(GITLAB_A, PROJECT, 100, &["docker", "gpu"])?;
    let event = job_event(100, "pending", Some(T));

    let (status, body) = h.job_hook(Some(SECRET_A), "pending-1", &event).await?;
    assert_eq!(status, StatusCode::ACCEPTED, "{body}");
    openapi_schema::check_response(WEBHOOK, "post", 202, &body)?;

    assert_eq!(h.job_lookups(), vec![url(GITLAB_A)?]);
    let row = h.job_a(100).await?;
    assert_eq!(
        JobRow {
            updated_at: 0,
            ..row
        },
        JobRow {
            connection_id: CONN_A.to_owned(),
            project_id: 11,
            job_id: 100,
            pipeline_id: 7,
            status: "pending".to_owned(),
            tag_list: r#"["docker","gpu"]"#.to_owned(),
            status_changed_at: T,
            retries_count: 0,
            gitlab_runner_id: None,
            pool_id: None,
            updated_at: 0,
        }
    );
    assert_eq!(
        h.delivery(&accepted_id(CONN_A, "pending-1")).await?,
        Some(DeliveryRow {
            id: accepted_id(CONN_A, "pending-1"),
            event: "Job Hook".to_owned(),
            action: Some("pending".to_owned()),
            hook_id: None,
            installation_id: None,
            repository_id: None,
            signature_valid: true,
            status: "processed".to_owned(),
        })
    );
    Ok(())
}

/// Orchestrator decision NIT-1: the delivery key is `Idempotency-Key`, then
/// `webhook-id`, then `X-Gitlab-Event-UUID`, then the SHA-256 of the body.
#[tokio::test]
async fn the_delivery_key_is_the_idempotency_key_then_webhook_id_then_event_uuid_then_the_body_hash()
-> Result<()> {
    let h = harness().await?;
    h.connection_a().await?;
    let base = [("x-gitlab-token", SECRET_A), ("x-gitlab-event", "Job Hook")];
    let cases: [(u64, &[(&str, &str)]); 4] = [
        (
            201,
            &[
                ("idempotency-key", "idem-1"),
                ("webhook-id", "hook-1"),
                ("x-gitlab-event-uuid", "uuid-1"),
            ],
        ),
        (
            202,
            &[("webhook-id", "hook-2"), ("x-gitlab-event-uuid", "uuid-2")],
        ),
        (203, &[("x-gitlab-event-uuid", "uuid-3")]),
        (204, &[]),
    ];
    let mut hashed = String::new();
    for (job_id, extra) in cases {
        let body = job_event(job_id, "created", Some(T))
            .to_string()
            .into_bytes();
        if extra.is_empty() {
            hashed = hex::encode(Sha256::digest(&body));
        }
        let headers = base.iter().chain(extra).copied().collect::<Vec<_>>();
        let (status, response) = h.post(&headers, body).await?;
        assert_eq!(status, StatusCode::ACCEPTED, "job {job_id}: {response}");
    }
    let mut expected = vec![
        accepted_id(CONN_A, "idem-1"),
        accepted_id(CONN_A, "hook-2"),
        accepted_id(CONN_A, "uuid-3"),
        accepted_id(CONN_A, &hashed),
    ];
    expected.sort();
    assert_eq!(h.delivery_ids().await?, expected);
    Ok(())
}

/// Done when: "a duplicate delivery". Orchestrator decision W-1: any existing
/// delivery id is a duplicate, whatever its status, and is acknowledged
/// without reprocessing; a failed one is recovered by `retry`, not by
/// GitLab sending it again.
#[tokio::test]
async fn a_duplicate_delivery_is_acknowledged_without_reprocessing() -> Result<()> {
    let h = harness().await?;
    h.connection_a().await?;
    h.gitlab_job(GITLAB_A, PROJECT, 100, &["docker"])?;
    h.gitlab_job(GITLAB_A, PROJECT, 101, &["docker"])?;

    let processed = job_event(100, "pending", Some(T));
    let (status, _) = h.job_hook(Some(SECRET_A), "dup-1", &processed).await?;
    assert_eq!(status, StatusCode::ACCEPTED);
    let row = h.job_a(100).await?;
    let (status, body) = h.job_hook(Some(SECRET_A), "dup-1", &processed).await?;
    assert_eq!(status, StatusCode::ACCEPTED, "{body}");
    openapi_schema::check_response(WEBHOOK, "post", 202, &body)?;
    assert_eq!(
        h.job_lookups().len(),
        1,
        "the duplicate is not looked up again"
    );
    assert_eq!(h.job_a(100).await?, row, "nor written again");

    // A delivery that failed is still a duplicate.
    h.gitlab.fail_next(
        &url(GITLAB_A)?,
        FakeMethod::Job,
        GitLabError::Upstream("down".into()),
    );
    let failed = job_event(101, "pending", Some(T));
    let (status, _) = h.job_hook(Some(SECRET_A), "dup-2", &failed).await?;
    assert_eq!(status, StatusCode::ACCEPTED);
    assert_eq!(
        h.delivery_status(&accepted_id(CONN_A, "dup-2")).await?,
        "failed"
    );
    let (status, _) = h.job_hook(Some(SECRET_A), "dup-2", &failed).await?;
    assert_eq!(status, StatusCode::ACCEPTED);
    assert_eq!(
        h.job_lookups().len(),
        2,
        "the failed duplicate is not reprocessed"
    );
    assert_eq!(h.job_row(CONN_A, 101).await?, None);
    assert_eq!(
        h.delivery_status(&accepted_id(CONN_A, "dup-2")).await?,
        "failed"
    );

    assert_eq!(
        h.delivery_ids().await?,
        vec![accepted_id(CONN_A, "dup-1"), accepted_id(CONN_A, "dup-2")]
    );
    Ok(())
}

/// Done when: "a malformed body". Valid headers, a body that is not a JSON
/// object: 400 with an `ApiErrorBody`, and nothing written.
#[tokio::test]
async fn a_body_that_is_not_a_json_object_is_a_400_and_writes_nothing() -> Result<()> {
    let h = harness().await?;
    h.connection_a().await?;
    h.runner("target", CONN_A, RUNNER_ID).await?;
    let runners = h.runners().await?;
    let headers = [
        ("x-gitlab-token", SECRET_A),
        ("x-gitlab-event", "Job Hook"),
        ("idempotency-key", "malformed"),
    ];
    for body in ["this is not JSON", "[]"] {
        let (status, response) = h.post(&headers, body.as_bytes().to_vec()).await?;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}: {response}");
        openapi_schema::check_response(WEBHOOK, "post", 400, &response)?;
    }
    assert_eq!(h.jobs().await?, vec![]);
    assert_eq!(h.runners().await?, runners);
    Ok(())
}

// ---------------------------------------------------------------------------
// Connection matching
// ---------------------------------------------------------------------------

/// Amendment "Connection lookup" and brief gate B-1: a project connection, a
/// group connection that contains the project, and a project connection on
/// another instance with the same project id are all candidates; the one
/// whose secret matches the token is used, with that connection's
/// credentials. No match among several is a 401.
#[tokio::test]
async fn among_several_candidates_the_one_whose_secret_matches_the_token_is_used() -> Result<()> {
    let h = harness().await?;
    h.connection(CONN_A, GITLAB_A, "project", 11, PROJECT_PATH, SECRET_A, 1)
        .await?;
    h.connection(CONN_B, GITLAB_A, "group", 5, "team", SECRET_B, 2)
        .await?;
    h.connection(CONN_C, GITLAB_B, "project", 11, "other/app", SECRET_C, 3)
        .await?;
    h.gitlab_job(GITLAB_B, PROJECT, 402, &["mac"])?;

    h.deliver(SECRET_B, &job_event(401, "created", Some(T)))
        .await?;
    h.deliver(SECRET_C, &job_event(402, "pending", Some(T)))
        .await?;
    h.deliver(SECRET_A, &job_event(403, "created", Some(T)))
        .await?;
    let recorded = h
        .jobs()
        .await?
        .into_iter()
        .map(|row| (row.connection_id, row.job_id))
        .collect::<Vec<_>>();
    let mut expected = vec![
        (CONN_B.to_owned(), 401),
        (CONN_C.to_owned(), 402),
        (CONN_A.to_owned(), 403),
    ];
    expected.sort();
    assert_eq!(recorded, expected);
    assert_eq!(
        h.job_lookups(),
        vec![url(GITLAB_B)?],
        "the pending job is looked up on the matched connection's instance"
    );

    let (status, body) = h
        .job_hook(
            Some("nobody's"),
            "none",
            &job_event(404, "created", Some(T)),
        )
        .await?;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "{body}");
    assert!(h.delivery("gitlab:unmatched:none").await?.is_some());
    assert_eq!(h.jobs().await?.len(), 3);
    Ok(())
}

/// Brief gate B-1: when several candidates' secrets match, the project
/// connection wins over the group one, then the oldest `created_at`. Here the
/// group connection is the oldest of all, and of the two project
/// connections the one on instance B is older, so B's wins.
#[tokio::test]
async fn when_several_candidates_match_a_project_connection_wins_then_the_oldest() -> Result<()> {
    let h = harness().await?;
    h.connection(CONN_A, GITLAB_A, "project", 11, PROJECT_PATH, "shared", 5)
        .await?;
    h.connection(CONN_B, GITLAB_A, "group", 5, "team", "shared", 1)
        .await?;
    h.connection(CONN_C, GITLAB_B, "project", 11, "other/app", "shared", 3)
        .await?;

    let key = h
        .deliver("shared", &job_event(500, "created", Some(T)))
        .await?;
    let recorded = h
        .jobs()
        .await?
        .into_iter()
        .map(|row| row.connection_id)
        .collect::<Vec<_>>();
    assert_eq!(recorded, vec![CONN_C.to_owned()]);
    assert_eq!(h.delivery_ids().await?, vec![accepted_id(CONN_C, &key)]);
    Ok(())
}

/// Brief gate B-1: a group connection matches a project whose
/// `path_with_namespace` starts with the group's `scope_path` at a `/`
/// boundary, at any depth, and nothing else.
#[tokio::test]
async fn a_group_connection_matches_projects_under_its_path_at_a_slash_boundary() -> Result<()> {
    let h = harness().await?;
    h.connection(CONN_B, GITLAB_A, "group", 5, "acme", SECRET_B, 1)
        .await?;

    for (project_id, path, job_id) in [(21, "acme/app", 601), (22, "acme/sub/app", 602)] {
        let event = in_project(job_event(job_id, "created", Some(T)), project_id, path);
        h.deliver(SECRET_B, &event).await?;
    }
    let event = in_project(job_event(603, "created", Some(T)), 23, "acme-other/app");
    let (status, body) = h.job_hook(Some(SECRET_B), "sibling", &event).await?;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "acme-other/app: {body}");

    let recorded = h
        .jobs()
        .await?
        .into_iter()
        .map(|row| (row.connection_id, row.project_id, row.job_id))
        .collect::<Vec<_>>();
    assert_eq!(
        recorded,
        vec![(CONN_B.to_owned(), 21, 601), (CONN_B.to_owned(), 22, 602)]
    );
    Ok(())
}

// ---------------------------------------------------------------------------
// The job record and the status rule
// ---------------------------------------------------------------------------

/// Done when: "a retried job". A retry carries a new `build_id` and a
/// `retries_count` above 0, so it is a new row and the original is untouched.
#[tokio::test]
async fn a_retried_job_is_a_new_row() -> Result<()> {
    let h = harness().await?;
    h.connection_a().await?;
    h.deliver(SECRET_A, &job_event(100, "created", Some(T)))
        .await?;
    h.deliver(SECRET_A, &job_event(100, "failed", Some(T + 1_000)))
        .await?;
    let original = h.job_a(100).await?;

    let mut retry = job_event(101, "created", Some(T + 2_000));
    retry["retries_count"] = json!(1);
    h.deliver(SECRET_A, &retry).await?;

    assert_eq!(h.job_a(100).await?, original);
    let row = h.job_a(101).await?;
    assert_eq!(
        (
            row.status.as_str(),
            row.retries_count,
            row.status_changed_at
        ),
        ("created", 1, T + 2_000)
    );
    assert_eq!(h.jobs().await?.len(), 2);
    Ok(())
}

/// Done when: "Never backwards", and orchestrator decision W-3: a `pending`
/// event after `running`, even with a newer time, is ignored before any
/// `GitLabApi::job` call.
#[tokio::test]
async fn a_pending_event_after_running_is_ignored_without_a_lookup() -> Result<()> {
    let h = harness().await?;
    h.connection_a().await?;
    h.gitlab_job(GITLAB_A, PROJECT, 100, &["docker"])?;
    h.deliver(SECRET_A, &job_event(100, "running", Some(T)))
        .await?;
    let running = h.job_a(100).await?;

    let key = h
        .deliver(SECRET_A, &job_event(100, "pending", Some(T + 60_000)))
        .await?;
    assert_eq!(h.job_a(100).await?, running);
    assert_eq!(h.job_lookups(), Vec::<Url>::new());
    assert_eq!(
        h.delivery_status(&accepted_id(CONN_A, &key)).await?,
        "processed"
    );
    Ok(())
}

/// Done when: "A later tier wins over time". #148's poll stored `pending` at
/// T; the webhook's `running` stamped earlier still applies and marks the
/// runner busy.
#[tokio::test]
async fn a_later_tier_applies_even_with_an_older_time() -> Result<()> {
    let h = harness().await?;
    h.connection_a().await?;
    h.runner("target", CONN_A, RUNNER_ID).await?;
    h.polled_row(100, "pending", T).await?;

    h.deliver(
        SECRET_A,
        &on_runner(job_event(100, "running", Some(T - 1_000)), RUNNER_ID),
    )
    .await?;
    let row = h.job_a(100).await?;
    assert_eq!(
        (
            row.status.as_str(),
            row.status_changed_at,
            row.gitlab_runner_id
        ),
        ("running", T - 1_000, Some(55))
    );
    assert!(h.runner_row("target").await?.busy);
    Ok(())
}

/// Done when: "Within a tier, time decides": `created` then `preparing`
/// (both tier 0) applies only with a newer time.
#[tokio::test]
async fn within_a_tier_only_a_newer_event_applies() -> Result<()> {
    let h = harness().await?;
    h.connection_a().await?;
    h.deliver(SECRET_A, &job_event(100, "created", Some(T)))
        .await?;
    let created = h.job_a(100).await?;

    h.deliver(SECRET_A, &job_event(100, "preparing", Some(T - 1)))
        .await?;
    assert_eq!(
        h.job_a(100).await?,
        created,
        "an older preparing is ignored"
    );

    h.deliver(SECRET_A, &job_event(100, "preparing", Some(T + 1)))
        .await?;
    let row = h.job_a(100).await?;
    assert_eq!(
        (row.status.as_str(), row.status_changed_at),
        ("preparing", T + 1)
    );
    Ok(())
}

/// Done when: "Timestamp unit". `status_changed_at` is
/// `build_status_changed_at` in epoch milliseconds; when that is missing,
/// `null` or unparsable, it is the receive time (`now_millis()`).
#[tokio::test]
async fn the_status_time_is_the_payloads_in_epoch_ms_or_else_the_receive_time() -> Result<()> {
    let h = harness().await?;
    h.connection_a().await?;

    h.deliver(SECRET_A, &job_event(301, "created", Some(T + 123)))
        .await?;
    assert_eq!(h.job_a(301).await?.status_changed_at, T + 123);

    let mut absent = job_event(302, "created", None);
    absent
        .as_object_mut()
        .context("an object")?
        .remove("build_status_changed_at");
    let null = job_event(303, "created", None);
    let mut unparsable = job_event(304, "created", None);
    unparsable["build_status_changed_at"] = json!("last Tuesday");
    for (job_id, event) in [(302, absent), (303, null), (304, unparsable)] {
        let before = gridops_core::now_millis();
        h.deliver(SECRET_A, &event).await?;
        let after = gridops_core::now_millis();
        let at = h.job_a(job_id).await?.status_changed_at;
        assert!(
            (before..=after).contains(&at),
            "job {job_id}: {at} is not the receive time ({before}..={after})"
        );
    }
    Ok(())
}

/// Done when: "`unknown`". A status GitLab may add later is recorded as a
/// processed delivery and writes no row (writers never store `unknown`).
#[tokio::test]
async fn a_build_event_with_an_unrecognised_status_writes_no_row() -> Result<()> {
    let h = harness().await?;
    h.connection_a().await?;
    let key = h
        .deliver(SECRET_A, &job_event(100, "teleporting", Some(T)))
        .await?;
    assert_eq!(h.jobs().await?, vec![]);
    assert_eq!(
        h.delivery_status(&accepted_id(CONN_A, &key)).await?,
        "processed"
    );
    Ok(())
}

/// Brief gate: "Non-`build` events" are recorded and processed with no
/// effect. The payload is a Pipeline Hook as GitLab sends it: the project is
/// only in `project`, there is no top-level `project_id`.
#[tokio::test]
async fn a_non_build_event_is_recorded_and_processed_with_no_effect() -> Result<()> {
    let h = harness().await?;
    h.connection_a().await?;
    h.runner("target", CONN_A, RUNNER_ID).await?;
    let runners = h.runners().await?;
    let pipeline = json!({
        "object_kind": "pipeline",
        "object_attributes": { "id": PIPELINE, "status": "running", "ref": "main" },
        "builds": [{ "id": 100, "status": "running", "runner": { "id": RUNNER_ID } }],
        "project": { "id": PROJECT, "name": "app", "path_with_namespace": PROJECT_PATH },
    });
    let (status, body) = h
        .post(
            &[
                ("x-gitlab-token", SECRET_A),
                ("x-gitlab-event", "Pipeline Hook"),
                ("idempotency-key", "pipeline-1"),
            ],
            pipeline.to_string().into_bytes(),
        )
        .await?;
    assert_eq!(status, StatusCode::ACCEPTED, "{body}");
    let delivery = h
        .delivery(&accepted_id(CONN_A, "pipeline-1"))
        .await?
        .context("the pipeline delivery is recorded")?;
    assert_eq!(
        (delivery.event.as_str(), delivery.status.as_str()),
        ("Pipeline Hook", "processed")
    );
    assert_eq!(h.jobs().await?, vec![]);
    assert_eq!(h.runners().await?, runners);
    assert_eq!(h.events().await?, vec![]);
    Ok(())
}

/// Round-3 Done when: "A non-`pending` insert writes `[]`" and makes no
/// lookup.
#[tokio::test]
async fn a_first_seen_non_pending_event_stores_no_tags_and_looks_nothing_up() -> Result<()> {
    let h = harness().await?;
    h.connection_a().await?;
    h.gitlab_job(GITLAB_A, PROJECT, 100, &["docker"])?;
    h.deliver(SECRET_A, &job_event(100, "created", Some(T)))
        .await?;
    assert_eq!(h.job_a(100).await?.tag_list, "[]");
    assert_eq!(h.job_lookups(), Vec::<Url>::new());
    Ok(())
}

/// Round-3 Done when: "The tags survive a later event".
#[tokio::test]
async fn the_fetched_tags_survive_later_events() -> Result<()> {
    let h = harness().await?;
    h.connection_a().await?;
    h.gitlab_job(GITLAB_A, PROJECT, 100, &["a", "b"])?;
    h.deliver(SECRET_A, &job_event(100, "pending", Some(T)))
        .await?;
    h.deliver(SECRET_A, &job_event(100, "running", Some(T + 1_000)))
        .await?;
    h.deliver(SECRET_A, &job_event(100, "success", Some(T + 2_000)))
        .await?;
    let row = h.job_a(100).await?;
    assert_eq!(
        (row.status.as_str(), row.tag_list.as_str()),
        ("success", r#"["a","b"]"#)
    );
    Ok(())
}

// ---------------------------------------------------------------------------
// Busy tracking
// ---------------------------------------------------------------------------

/// Done when: "busy set and cleared on the right runner", with the
/// `runner_events` rows (W-7, NIT-1). Runner 55 on another instance and
/// runner 56 on this one are left alone, and the busy write never touches the
/// GitHub job columns.
#[tokio::test]
async fn running_marks_the_runner_busy_and_a_conclusion_clears_it_on_that_runner_only() -> Result<()>
{
    let h = harness().await?;
    h.connection_a().await?;
    h.connection(CONN_B, GITLAB_B, "project", 12, "team/other", SECRET_B, 2)
        .await?;
    h.runner("target", CONN_A, RUNNER_ID).await?;
    h.runner("other-instance", CONN_B, RUNNER_ID).await?;
    h.runner("other-id", CONN_A, 56).await?;
    let others = [
        h.runner_row("other-instance").await?,
        h.runner_row("other-id").await?,
    ];

    h.deliver(
        SECRET_A,
        &on_runner(job_event(100, "running", Some(T)), RUNNER_ID),
    )
    .await?;
    let target = h.runner_row("target").await?;
    assert_eq!(
        (
            target.busy,
            target.status.as_str(),
            target.current_job_id,
            target.last_job_id,
            target.github_runner_id
        ),
        (true, "busy", None, None, None)
    );
    assert_eq!(h.job_a(100).await?.gitlab_runner_id, Some(55));

    h.deliver(
        SECRET_A,
        &on_runner(job_event(100, "success", Some(T + 1_000)), RUNNER_ID),
    )
    .await?;
    let target = h.runner_row("target").await?;
    assert_eq!(
        (
            target.busy,
            target.status.as_str(),
            target.current_job_id,
            target.last_job_id
        ),
        (false, "online", None, None)
    );
    assert_eq!(
        [
            h.runner_row("other-instance").await?,
            h.runner_row("other-id").await?,
        ],
        others
    );
    same_events(
        &h.events().await?,
        &[
            busy_event("target", "running", "rspec · pipeline 7", 100, CONN_A),
            busy_event("target", "success", "rspec · pipeline 7", 100, CONN_A),
        ],
    )
}

/// Brief W-1: a `running` event without `runner.id` records the job and has
/// no busy effect.
#[tokio::test]
async fn running_without_a_runner_records_the_job_and_touches_no_runner() -> Result<()> {
    let h = harness().await?;
    h.connection_a().await?;
    h.runner("target", CONN_A, RUNNER_ID).await?;
    let runners = h.runners().await?;
    h.deliver(SECRET_A, &job_event(100, "running", Some(T)))
        .await?;
    let row = h.job_a(100).await?;
    assert_eq!(
        (row.status.as_str(), row.gitlab_runner_id),
        ("running", None)
    );
    assert_eq!(h.runners().await?, runners);
    assert_eq!(h.events().await?, vec![]);
    Ok(())
}

/// Round-4 Done when: "First-seen `running`" inserts the row, marks the
/// runner busy and writes one event (with `Job` when `build_name` is
/// absent); a `pending` delivered afterwards is ignored without a lookup.
#[tokio::test]
async fn a_first_seen_running_event_marks_the_runner_busy_and_a_later_pending_is_ignored()
-> Result<()> {
    let h = harness().await?;
    h.connection_a().await?;
    h.runner("target", CONN_A, RUNNER_ID).await?;
    h.gitlab_job(GITLAB_A, PROJECT, 100, &["docker"])?;
    let mut running = on_runner(job_event(100, "running", Some(T)), RUNNER_ID);
    running
        .as_object_mut()
        .context("an object")?
        .remove("build_name");
    h.deliver(SECRET_A, &running).await?;

    let row = h.job_a(100).await?;
    assert_eq!(
        (row.status.as_str(), row.gitlab_runner_id),
        ("running", Some(55))
    );
    assert!(h.runner_row("target").await?.busy);
    same_events(
        &h.events().await?,
        &[busy_event(
            "target",
            "running",
            "Job · pipeline 7",
            100,
            CONN_A,
        )],
    )?;

    h.deliver(SECRET_A, &job_event(100, "pending", Some(T + 1_000)))
        .await?;
    assert_eq!(h.job_a(100).await?, row);
    assert_eq!(h.job_lookups(), Vec::<Url>::new());
    Ok(())
}

/// Round-4 Done when: "`running` after a failed `pending` fetch".
#[tokio::test]
async fn running_after_a_failed_pending_lookup_marks_the_runner_busy() -> Result<()> {
    let h = harness().await?;
    h.connection_a().await?;
    h.runner("target", CONN_A, RUNNER_ID).await?;
    h.gitlab_job(GITLAB_A, PROJECT, 100, &["docker"])?;
    h.gitlab.fail_next(
        &url(GITLAB_A)?,
        FakeMethod::Job,
        GitLabError::Upstream("down".into()),
    );

    let key = h
        .deliver(SECRET_A, &job_event(100, "pending", Some(T)))
        .await?;
    assert_eq!(
        h.delivery_status(&accepted_id(CONN_A, &key)).await?,
        "failed"
    );
    assert_eq!(h.jobs().await?, vec![], "a failed lookup writes no row");

    h.deliver(
        SECRET_A,
        &on_runner(job_event(100, "running", Some(T + 1_000)), RUNNER_ID),
    )
    .await?;
    assert_eq!(h.job_a(100).await?.status, "running");
    assert!(h.runner_row("target").await?.busy);
    Ok(())
}

/// Round-3 Done when: "The runner is filled after the poll wins". The poll
/// stored `running` with no runner at T; the webhook's older `running` with
/// runner 55 fills the runner and marks it busy without moving the status
/// time. Against a terminal row the same event changes nothing.
#[tokio::test]
async fn an_older_running_event_fills_the_runner_the_poll_did_not_know() -> Result<()> {
    let h = harness().await?;
    h.connection_a().await?;
    h.runner("target", CONN_A, RUNNER_ID).await?;
    let polled = h.polled_row(100, "running", T).await?;

    h.deliver(
        SECRET_A,
        &on_runner(job_event(100, "running", Some(T - 1_000)), RUNNER_ID),
    )
    .await?;
    let row = h.job_a(100).await?;
    assert_eq!(
        JobRow {
            updated_at: polled.updated_at,
            ..row
        },
        JobRow {
            gitlab_runner_id: Some(55),
            ..polled
        },
        "only the runner is filled"
    );
    assert!(h.runner_row("target").await?.busy);
    assert_eq!(h.events().await?.len(), 1);

    let concluded = h.polled_row(101, "success", T).await?;
    let target = h.runner_row("target").await?;
    h.deliver(
        SECRET_A,
        &on_runner(job_event(101, "running", Some(T - 1_000)), RUNNER_ID),
    )
    .await?;
    assert_eq!(
        h.job_a(101).await?,
        concluded,
        "a terminal row is never filled"
    );
    assert_eq!(h.runner_row("target").await?, target);
    assert_eq!(h.events().await?.len(), 1);
    Ok(())
}

/// Round-4 Done when: "The fill is recognised under clock skew". The poll
/// stored `running` with no runner at T; the webhook's `running` with runner
/// 55 is newer, so it applies and the runner is marked busy.
#[tokio::test]
async fn a_newer_running_event_onto_a_runnerless_running_row_marks_the_runner_busy() -> Result<()> {
    let h = harness().await?;
    h.connection_a().await?;
    h.runner("target", CONN_A, RUNNER_ID).await?;
    h.polled_row(100, "running", T).await?;

    h.deliver(
        SECRET_A,
        &on_runner(job_event(100, "running", Some(T + 1_000)), RUNNER_ID),
    )
    .await?;
    let row = h.job_a(100).await?;
    assert_eq!(
        (row.status_changed_at, row.gitlab_runner_id),
        (T + 1_000, Some(55))
    );
    assert!(h.runner_row("target").await?.busy);
    assert_eq!(h.events().await?.len(), 1);
    Ok(())
}

/// Round-4 Done when: "No double set". A second `running` for the same job
/// and runner changes no runner and writes no second event.
#[tokio::test]
async fn a_second_running_event_for_the_same_runner_sets_nothing_again() -> Result<()> {
    let h = harness().await?;
    h.connection_a().await?;
    h.runner("target", CONN_A, RUNNER_ID).await?;
    h.deliver(
        SECRET_A,
        &on_runner(job_event(100, "running", Some(T)), RUNNER_ID),
    )
    .await?;
    let target = h.runner_row("target").await?;
    assert!(target.busy);

    h.deliver(
        SECRET_A,
        &on_runner(job_event(100, "running", Some(T + 1_000)), RUNNER_ID),
    )
    .await?;
    assert_eq!(h.job_a(100).await?.status_changed_at, T + 1_000);
    assert_eq!(h.runner_row("target").await?, target);
    assert_eq!(h.events().await?.len(), 1);
    Ok(())
}

/// Round-2 Done when: "Terminal is final". After `success`, a newer
/// `running` or `pending` changes neither the row nor any runner, and the
/// `pending` is not looked up.
#[tokio::test]
async fn after_a_conclusion_no_later_event_changes_the_job_or_a_runner() -> Result<()> {
    let h = harness().await?;
    h.connection_a().await?;
    h.runner("target", CONN_A, RUNNER_ID).await?;
    h.gitlab_job(GITLAB_A, PROJECT, 100, &["docker"])?;
    h.deliver(
        SECRET_A,
        &on_runner(job_event(100, "running", Some(T)), RUNNER_ID),
    )
    .await?;
    h.deliver(
        SECRET_A,
        &on_runner(job_event(100, "success", Some(T + 1_000)), RUNNER_ID),
    )
    .await?;
    let concluded = h.job_a(100).await?;
    let runners = h.runners().await?;

    h.deliver(
        SECRET_A,
        &on_runner(job_event(100, "running", Some(T + 2_000)), RUNNER_ID),
    )
    .await?;
    h.deliver(SECRET_A, &job_event(100, "pending", Some(T + 3_000)))
        .await?;
    assert_eq!(h.job_a(100).await?, concluded);
    assert_eq!(h.runners().await?, runners);
    assert_eq!(h.events().await?.len(), 2);
    assert_eq!(h.job_lookups(), Vec::<Url>::new());
    Ok(())
}

/// Round-2 Done when: "A stale terminal event changes nothing". After
/// `success` cleared the runner, a `failed` stamped before that success
/// touches neither the row nor the runner.
#[tokio::test]
async fn a_stale_conclusion_changes_neither_the_job_nor_the_runner() -> Result<()> {
    let h = harness().await?;
    h.connection_a().await?;
    h.runner("target", CONN_A, RUNNER_ID).await?;
    h.deliver(
        SECRET_A,
        &on_runner(job_event(100, "running", Some(T)), RUNNER_ID),
    )
    .await?;
    h.deliver(
        SECRET_A,
        &on_runner(job_event(100, "success", Some(T + 2_000)), RUNNER_ID),
    )
    .await?;
    let concluded = h.job_a(100).await?;
    let target = h.runner_row("target").await?;
    assert!(!target.busy);

    h.deliver(
        SECRET_A,
        &on_runner(job_event(100, "failed", Some(T + 1_000)), RUNNER_ID),
    )
    .await?;
    assert_eq!(h.job_a(100).await?, concluded);
    assert_eq!(h.runner_row("target").await?, target);
    assert_eq!(h.events().await?.len(), 2);
    Ok(())
}

// ---------------------------------------------------------------------------
// Retry and failure recovery
// ---------------------------------------------------------------------------

/// Brief gate: "Retry through the GitLab arm". A delivery whose tag lookup
/// failed is retried through `retry`, which re-runs the GitLab processing
/// (GitHub's `process` would record nothing for a Job Hook).
#[tokio::test]
async fn a_failed_delivery_is_retried_through_the_gitlab_arm() -> Result<()> {
    let h = harness().await?;
    h.connection_a().await?;
    h.gitlab_job(GITLAB_A, PROJECT, 100, &["docker"])?;
    h.gitlab.fail_next(
        &url(GITLAB_A)?,
        FakeMethod::Job,
        GitLabError::Upstream("down".into()),
    );

    let (status, body) = h
        .job_hook(
            Some(SECRET_A),
            "retry-1",
            &job_event(100, "pending", Some(T)),
        )
        .await?;
    assert_eq!(status, StatusCode::ACCEPTED, "{body}");
    openapi_schema::check_response(WEBHOOK, "post", 202, &body)?;
    let id = accepted_id(CONN_A, "retry-1");
    assert_eq!(h.delivery_status(&id).await?, "failed");
    assert_eq!(h.jobs().await?, vec![]);

    let (status, body) = h.retry(&id).await?;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(h.delivery_status(&id).await?, "processed");
    let row = h.job_a(100).await?;
    assert_eq!(
        (row.status.as_str(), row.tag_list.as_str()),
        ("pending", r#"["docker"]"#)
    );
    assert_eq!(h.job_lookups(), vec![url(GITLAB_A)?, url(GITLAB_A)?]);
    Ok(())
}

/// Brief W-4 option (a) and NIT-1: a retry uses the connection recorded in
/// the delivery id, never the candidate set, and a key containing `:`
/// round-trips. Here the candidate preference (project, then oldest) would
/// pick connection A, but the delivery was matched to B.
#[tokio::test]
async fn a_retry_uses_the_recorded_connection_even_when_the_key_contains_colons() -> Result<()> {
    let h = harness().await?;
    h.connection(CONN_A, GITLAB_A, "project", 11, PROJECT_PATH, SECRET_A, 1)
        .await?;
    h.connection(CONN_B, GITLAB_B, "project", 11, PROJECT_PATH, SECRET_B, 2)
        .await?;
    h.gitlab_job(GITLAB_A, PROJECT, 100, &["linux"])?;
    h.gitlab_job(GITLAB_B, PROJECT, 100, &["mac"])?;
    h.gitlab.fail_next(
        &url(GITLAB_B)?,
        FakeMethod::Job,
        GitLabError::Upstream("down".into()),
    );

    let key = "gl:retry:1";
    let (status, body) = h
        .job_hook(Some(SECRET_B), key, &job_event(100, "pending", Some(T)))
        .await?;
    assert_eq!(status, StatusCode::ACCEPTED, "{body}");
    let id = accepted_id(CONN_B, key);
    assert_eq!(h.delivery_status(&id).await?, "failed");

    let (status, body) = h.retry(&id).await?;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(h.job_lookups(), vec![url(GITLAB_B)?, url(GITLAB_B)?]);
    let recorded = h
        .jobs()
        .await?
        .into_iter()
        .map(|row| (row.connection_id, row.job_id, row.tag_list))
        .collect::<Vec<_>>();
    assert_eq!(
        recorded,
        vec![(CONN_B.to_owned(), 100, r#"["mac"]"#.to_owned())]
    );
    Ok(())
}

/// Round-4 Done when: "Partial failure is recoverable". One transaction
/// holds the job write, the busy change and the event: when the event insert
/// fails, the delivery fails with nothing written, and a retry applies it in
/// full. The same holds for a conclusion, which leaves the runner busy until
/// the retry clears it.
#[tokio::test]
async fn a_failure_inside_the_write_rolls_it_all_back_and_a_retry_applies_it() -> Result<()> {
    let h = harness().await?;
    h.connection_a().await?;
    h.runner("target", CONN_A, RUNNER_ID).await?;
    let break_events = "CREATE TRIGGER fail_runner_events BEFORE INSERT ON runner_events
                        BEGIN SELECT RAISE(ABORT, 'injected runner_events failure'); END;";
    let mend_events = "DROP TRIGGER fail_runner_events;";

    sqlx::raw_sql(break_events).execute(&h.database).await?;
    let started = h
        .deliver(
            SECRET_A,
            &on_runner(job_event(100, "running", Some(T)), RUNNER_ID),
        )
        .await?;
    let started = accepted_id(CONN_A, &started);
    assert_eq!(h.delivery_status(&started).await?, "failed");
    assert_eq!(h.jobs().await?, vec![], "the job write is rolled back");
    assert!(!h.runner_row("target").await?.busy, "so is the busy flag");

    sqlx::raw_sql(mend_events).execute(&h.database).await?;
    let (status, body) = h.retry(&started).await?;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(h.delivery_status(&started).await?, "processed");
    assert_eq!(h.job_a(100).await?.status, "running");
    assert!(h.runner_row("target").await?.busy);
    assert_eq!(h.events().await?.len(), 1);

    sqlx::raw_sql(break_events).execute(&h.database).await?;
    let finished = h
        .deliver(
            SECRET_A,
            &on_runner(job_event(100, "success", Some(T + 1_000)), RUNNER_ID),
        )
        .await?;
    let finished = accepted_id(CONN_A, &finished);
    assert_eq!(h.delivery_status(&finished).await?, "failed");
    assert_eq!(h.job_a(100).await?.status, "running");
    assert!(
        h.runner_row("target").await?.busy,
        "still busy until the retry"
    );

    sqlx::raw_sql(mend_events).execute(&h.database).await?;
    let (status, body) = h.retry(&finished).await?;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(h.delivery_status(&finished).await?, "processed");
    assert_eq!(h.job_a(100).await?.status, "success");
    assert!(!h.runner_row("target").await?.busy);
    assert_eq!(h.events().await?.len(), 2);
    Ok(())
}
