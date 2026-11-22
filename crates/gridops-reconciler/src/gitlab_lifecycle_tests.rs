//! The GitLab runner lifecycle as the reconciler runs it (#139): register,
//! hand the runner its registration, roll back a failed start, deregister on
//! every exit path, count refusals towards the pool's circuit, and sweep
//! orphaned GitLab runners.
//!
//! Through the stand-in runner manager of `start_tests.rs` (the built-in host,
//! Docker and Tart), a fleet machine whose agent the tests play by recording
//! its command results through the real registry, `FakeGitLab`, and the real
//! database. Fixture values come from the published contract, never typed
//! here: the manager's `exitReason` from `GITLAB_EXIT_REASONS`, the agent's
//! outcomes from `FailureCode` / `DiagnosticCode`, the idle timeout from
//! `GITLAB_DEFAULT_WAIT_TIMEOUT_SECS`.

use super::*;
use start_tests::{PASS_LIMIT, Setup};

use gridops_core::{
    fleet::{
        CiTarget, ExternalId, GitlabConnectionId, HostId, PlacementId, UpstreamText,
        ids::{DiagnosticId, PreparedEnvironmentId, ResultId},
        protocol::{
            agent::{
                AgentCommandEnvelope, AgentOperation, CommandScope, GitlabRunnerMaterial,
                LocalResourceIdentity,
            },
            outcomes::{
                AgentCommandResult, CommandOutcome, DelayedStartExclusionClaim, DiagnosticCode,
                DiagnosticReference, FailureCode, LocalAbsenceClaim, SuccessEvidence,
            },
            primitives::{Sha256Digest, WireTimestamp},
        },
        registry::{AuthenticatedHost, OneTimeSecret, RegistryService, Verifier},
        targets,
    },
    gitlab::{
        GitLabError, Scope,
        fake::{FakeMethod, FakeRunner},
    },
    provisioning::{
        GITLAB_DEFAULT_WAIT_TIMEOUT_SECS, GITLAB_EXIT_COMPLETED, GITLAB_EXIT_IDLE_EXPIRED,
        GITLAB_EXIT_REASONS, GITLAB_EXIT_REGISTRATION_REFUSED,
    },
};
use reqwest::Url;

/// This deployment (`GRIDOPS_BASE_URL`), and the `<instance>` its runner
/// descriptions carry: the host and the non-default port.
const INSTANCE_URL: &str = "https://gridops.example.test:8443/";
const INSTANCE: &str = "gridops.example.test:8443";
const RUNNER_IMAGE: &str = "registry.example.test/gitlab-runner:v17";
/// What `FakeGitLab` answers while GitLab's client is not wired (#146).
const PLACEHOLDER_ERROR: &str = "GitLab client not available yet";

/// A GitLab connection: a project or group on one GitLab instance.
struct Connection {
    id: &'static str,
    name: &'static str,
    base_url: &'static str,
    kind: &'static str,
    scope_id: u64,
    path: &'static str,
    created_at: i64,
}

/// The connection most tests use.
const ACME: Connection = Connection {
    id: "gl-acme",
    name: "Acme API",
    base_url: "https://gitlab.example.test",
    kind: "project",
    scope_id: 42,
    path: "acme/api",
    created_at: 1,
};

impl Connection {
    fn url(&self) -> Result<Url> {
        Ok(Url::parse(self.base_url)?)
    }

    fn scope(&self) -> Result<Scope> {
        let id = ExternalId::new(self.scope_id)?;
        let path = self.path.to_owned();
        Ok(match self.kind {
            "group" => Scope::Group { id, path },
            _ => Scope::Project { id, path },
        })
    }

    fn target(&self) -> Result<CiTarget> {
        Ok(self.scope()?.target(GitlabConnectionId::new(self.id)?))
    }

    /// Stores the connection with its access token (sealed, by key, as
    /// Bitbucket's are) and makes it a member of `pools`.
    async fn add(&self, setup: &Setup, pools: &[&str]) -> Result<()> {
        let token_key = format!("gitlab.{}.access_token", self.id);
        sqlx::query(
            "INSERT INTO gitlab_connections (id,name,base_url,scope_kind,scope_id,scope_path,access_token_key,webhook_secret_key,created_at,updated_at)
             VALUES (?,?,?,?,?,?,?,?,?,?)",
        )
        .bind(self.id)
        .bind(self.name)
        .bind(self.base_url)
        .bind(self.kind)
        .bind(i64::try_from(self.scope_id)?)
        .bind(self.path)
        .bind(&token_key)
        .bind(format!("gitlab.{}.webhook_secret", self.id))
        .bind(self.created_at)
        .bind(self.created_at)
        .execute(&setup.app.database)
        .await?;
        sqlx::query("INSERT INTO runtime_secrets (key,value,updated_at) VALUES (?,?,1)")
            .bind(&token_key)
            .bind(setup.app.vault.seal(&format!("glpat-{}", self.id))?)
            .execute(&setup.app.database)
            .await?;
        for pool in pools {
            sqlx::query(
                "INSERT INTO runner_pool_gitlab_connections (pool_id,connection_id,created_at) VALUES (?,?,?)",
            )
            .bind(pool)
            .bind(self.id)
            .bind(self.created_at)
            .execute(&setup.app.database)
            .await?;
        }
        Ok(())
    }
}

/// A GitLab pool (#138's shape: `scope='gitlab'`) on `provider`, labelled
/// `docker` and `linux-large`, taking untagged jobs, wanting `desired`.
fn gitlab_pool(
    id: &str,
    provider: &str,
    docker_image: &str,
    tart_image: &str,
    desired: i64,
) -> String {
    let image = if provider == "tart" {
        tart_image
    } else {
        docker_image
    };
    format!(
        r#"INSERT INTO runner_pools (id,installation_id,name,scope,image,provider,providers,labels,docker_image,tart_image,desired_count,min_count,run_untagged,autoscaling_enabled,created_at,updated_at)
             VALUES ('{id}',1,'{id}','gitlab','{image}','{provider}','["{provider}"]','["docker","linux-large"]','{docker_image}','{tart_image}',{desired},0,1,0,10,10);"#
    )
}

/// `Setup` with its own two pools wanting nothing, `extra` (SQL) applied, and
/// this deployment served from `INSTANCE_URL`.
async fn gitlab_setup(extra: &str) -> Result<Setup> {
    let mut setup = Setup::new(
        &format!(
            "UPDATE runner_pools SET desired_count=0 WHERE id IN ('pool-mac','pool-linux'); {extra}"
        ),
        tart_lane::SLOT_WATCH_INTERVAL,
    )
    .await?;
    setup.app.config = setup.app.config.with_base_url(Url::parse(INSTANCE_URL)?);
    Ok(setup)
}

/// The description `GridOps` gives the GitLab runner of runner row `runner_id`.
fn marker(runner_id: &str) -> String {
    format!("gridops-runner:{INSTANCE}:{runner_id}")
}

/// The `exitReason` the manager reports for a GitLab runner script's exit
/// status, from the published table.
fn exit_reason(status: i32) -> Result<&'static str> {
    GITLAB_EXIT_REASONS
        .iter()
        .find(|(code, _)| *code == status)
        .map(|(_, reason)| *reason)
        .context("a published GitLab exit reason")
}

/// The token `FakeGitLab` hands out with GitLab runner `id`.
fn fake_token(id: i64) -> String {
    format!("glrt-fake-runner-token-{id:04}")
}

async fn pass(setup: &Setup) -> Result<()> {
    tokio::time::timeout(PASS_LIMIT, run_pass(&setup.app)).await?;
    Ok(())
}

#[derive(Debug, FromRow)]
struct RunnerRow {
    id: String,
    status: String,
    ci_platform: String,
    gitlab_connection_id: Option<String>,
    gitlab_runner_id: Option<i64>,
    deleted: bool,
}

/// Every runner row of the pool, deleted ones included, oldest first.
async fn rows(setup: &Setup, pool_id: &str) -> Result<Vec<RunnerRow>> {
    Ok(sqlx::query_as(
        "SELECT id,status,ci_platform,gitlab_connection_id,gitlab_runner_id,deleted_at IS NOT NULL AS deleted
           FROM runners WHERE pool_id=? ORDER BY created_at,rowid",
    )
    .bind(pool_id)
    .fetch_all(&setup.app.database)
    .await?)
}

/// The pool's rows still in service (not deleted, not failed).
async fn live(setup: &Setup, pool_id: &str) -> Result<Vec<RunnerRow>> {
    Ok(rows(setup, pool_id)
        .await?
        .into_iter()
        .filter(|row| !row.deleted && !matches!(row.status.as_str(), "deleted" | "failed"))
        .collect())
}

/// The pool's provisioning circuit: failures, open, retry at, state.
async fn circuit(setup: &Setup, pool_id: &str) -> Result<(i64, bool, Option<i64>, String)> {
    Ok(sqlx::query_as(
        "SELECT provision_failure_count,provision_circuit_open,provision_retry_at,state FROM runner_pools WHERE id=?",
    )
    .bind(pool_id)
    .fetch_one(&setup.app.database)
    .await?)
}

/// Lets the pool's failure pause end, keeping its count and circuit.
async fn end_backoff(setup: &Setup, pool_id: &str) -> Result<()> {
    sqlx::query("UPDATE runner_pools SET provision_retry_at=1 WHERE id=?")
        .bind(pool_id)
        .execute(&setup.app.database)
        .await?;
    Ok(())
}

/// The base URL of every `method` call `FakeGitLab` got, in order.
fn gitlab_calls(setup: &Setup, method: FakeMethod) -> Vec<Url> {
    setup
        .gitlab
        .calls()
        .into_iter()
        .filter(|call| call.method == method)
        .map(|call| call.base_url)
        .collect()
}

/// The GitLab runner ids live on the connection's instance, in order.
fn gitlab_runner_ids(setup: &Setup, connection: &Connection) -> Result<Vec<i64>> {
    let mut ids = setup
        .gitlab
        .runners(&connection.url()?)
        .into_iter()
        .map(|runner| i64::try_from(runner.id.get()))
        .collect::<Result<Vec<_>, _>>()?;
    ids.sort_unstable();
    Ok(ids)
}

/// The runner starts the manager was asked for on behalf of `pool_id`.
fn manager_bodies(setup: &Setup, pool_id: &str) -> Vec<Value> {
    setup
        .manager
        .bodies()
        .into_iter()
        .filter(|body| body["poolId"] == pool_id)
        .collect()
}

/// What `GridOps` recorded about the pool's runners and starts: the runners'
/// failure reasons, the pool's events and its wait reason.
async fn recorded_texts(setup: &Setup, pool_id: &str) -> Result<Vec<String>> {
    let mut texts: Vec<String> = sqlx::query_scalar(
        "SELECT failure_reason FROM runners WHERE pool_id=? AND failure_reason IS NOT NULL",
    )
    .bind(pool_id)
    .fetch_all(&setup.app.database)
    .await?;
    texts.extend(
        sqlx::query_scalar::<_, String>("SELECT message FROM runner_events WHERE pool_id=?")
            .bind(pool_id)
            .fetch_all(&setup.app.database)
            .await?,
    );
    texts.extend(
        sqlx::query_scalar::<_, String>("SELECT reason FROM runner_pool_waits WHERE pool_id=?")
            .bind(pool_id)
            .fetch_all(&setup.app.database)
            .await?,
    );
    Ok(texts)
}

/// A readable diagnostic (brief-gate W-2): one recorded text names the
/// connection and `outcome` (case aside), and none holds a runner token.
async fn assert_readable(setup: &Setup, pool_id: &str, outcome: &str) -> Result<()> {
    let texts = recorded_texts(setup, pool_id).await?;
    let (name, outcome) = (ACME.name.to_lowercase(), outcome.to_lowercase());
    assert!(
        texts.iter().any(|text| {
            let text = text.to_lowercase();
            text.contains(&name) && text.contains(&outcome)
        }),
        "no recorded text names the connection ({}) and {outcome}: {texts:?}",
        ACME.name
    );
    assert!(
        texts.iter().all(|text| !text.contains("glrt-")),
        "{texts:?}"
    );
    Ok(())
}

/// #173, beside `assert_readable`: the runner row of a GitLab runner whose
/// Tart image has no `gitlab-runner` holds a `failure_reason` that names the
/// outcome, the missing `gitlab-runner` and the script that prepares the
/// image, and the pool has exactly one `error` event for it, carrying the
/// outcome and the runner, whose message names that script too. Neither
/// holds a runner token.
async fn assert_runner_missing(
    setup: &Setup,
    pool_id: &str,
    runner_id: &str,
    outcome: &str,
) -> Result<()> {
    const SCRIPT: &str = "prepare-tart-gitlab-runner-image.sh";
    let reason: Option<String> =
        sqlx::query_scalar("SELECT failure_reason FROM runners WHERE id=?")
            .bind(runner_id)
            .fetch_one(&setup.app.database)
            .await?;
    let reason = reason.context("the runner row has no failure_reason")?;
    let lower = reason.to_lowercase();
    assert!(
        lower.contains(outcome) && lower.contains("gitlab-runner") && reason.contains(SCRIPT),
        "the failure_reason does not name {outcome}, gitlab-runner and {SCRIPT}: {reason:?}"
    );
    let events: Vec<(String, String)> = sqlx::query_as(
        "SELECT message,metadata FROM runner_events WHERE pool_id=? AND level='error' AND json_extract(metadata,'$.outcome')=?",
    )
    .bind(pool_id)
    .bind(outcome)
    .fetch_all(&setup.app.database)
    .await?;
    let [(message, metadata)] = events.as_slice() else {
        anyhow::bail!("expected one {outcome} error event on {pool_id}, got {events:?}");
    };
    let metadata: Value = serde_json::from_str(metadata)?;
    assert_eq!(metadata["runnerId"], runner_id, "{metadata}");
    assert!(message.contains(SCRIPT), "{message:?}");
    assert!(
        !reason.contains("glrt-") && !message.contains("glrt-"),
        "{reason:?} {message:?}"
    );
    Ok(())
}

/// Token secrecy: no `glrt-` token in any column of any runner row, in a
/// runner or pool event, a pool's wait reason or an audit event.
async fn assert_no_token_stored(setup: &Setup) -> Result<()> {
    let columns: Vec<String> = sqlx::query_scalar("SELECT name FROM pragma_table_info('runners')")
        .fetch_all(&setup.app.database)
        .await?;
    let every_column = columns
        .iter()
        .map(|column| format!("COALESCE(CAST(\"{column}\" AS TEXT),'')"))
        .collect::<Vec<_>>()
        .join("||'|'||");
    let mut stored: Vec<String> = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
        "SELECT {every_column} FROM runners"
    )))
    .fetch_all(&setup.app.database)
    .await?;
    for query in [
        "SELECT message||'|'||metadata FROM runner_events",
        "SELECT reason FROM runner_pool_waits",
        "SELECT metadata FROM audit_events",
    ] {
        stored.extend(
            sqlx::query_scalar::<_, String>(query)
                .fetch_all(&setup.app.database)
                .await?,
        );
    }
    assert!(
        stored.iter().all(|text| !text.contains("glrt-")),
        "a runner token was stored: {stored:?}"
    );
    Ok(())
}

/// #171: the name of the warning a refused GitLab runner deletion records on
/// the runner's pool, as the pool's Events list shows it.
const DELETION_REFUSED: &str = "GitLab runner not deleted";

/// What `FakeGitLab` answers a delete with when the token lacks
/// `manage_runner`, as GitLab 17.1+ does.
fn missing_manage_runner() -> GitLabError {
    GitLabError::MissingScope("manage_runner".to_owned())
}

#[derive(Debug, FromRow)]
struct RefusalEvent {
    runner_id: Option<String>,
    level: String,
    message: String,
    metadata: String,
}

/// The pool's "GitLab runner not deleted" events, oldest first.
async fn refusals(setup: &Setup, pool_id: &str) -> Result<Vec<RefusalEvent>> {
    Ok(sqlx::query_as(
        "SELECT runner_id,level,message,metadata FROM runner_events
          WHERE pool_id=? AND event=? ORDER BY created_at,rowid",
    )
    .bind(pool_id)
    .bind(DELETION_REFUSED)
    .fetch_all(&setup.app.database)
    .await?)
}

/// Whether `text` names the number `number` as a whole number (so runner 1
/// is not found in "runner 12").
fn names_number(text: &str, number: i64) -> bool {
    let number = number.to_string();
    text.split(|character: char| !character.is_ascii_digit())
        .any(|part| part == number)
}

/// A refused deletion as the operator reads it: a `warning` (exactly: the
/// pool's Events list shows any other level but `error` as a success) whose
/// message names the connection, the GitLab runner id and `reason`, and
/// carries no access token (`glpat-`) or runner token (`glrt-`).
fn assert_refusal(event: &RefusalEvent, connection: &str, gitlab_runner_id: i64, reason: &str) {
    assert_eq!(event.level, "warning", "{event:?}");
    assert!(
        event.message.contains(connection),
        "names the connection: {event:?}"
    );
    assert!(
        names_number(&event.message, gitlab_runner_id),
        "names GitLab runner {gitlab_runner_id}: {event:?}"
    );
    assert!(event.message.contains(reason), "names {reason}: {event:?}");
    for text in [&event.message, &event.metadata] {
        assert!(
            !text.contains("glpat-") && !text.contains("glrt-"),
            "no token: {event:?}"
        );
    }
}

/// The reconciler's own log lines (`gridops_*` targets, every level), for
/// the token-secrecy check. A test runs on one thread, so a thread-local
/// subscriber sees everything its pass logs.
#[derive(Clone, Default)]
struct Logs(Arc<std::sync::Mutex<Vec<u8>>>);

impl std::io::Write for Logs {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for Logs {
    type Writer = Self;

    fn make_writer(&'a self) -> Self::Writer {
        self.clone()
    }
}

impl Logs {
    fn capture() -> (Self, tracing::subscriber::DefaultGuard) {
        let logs = Self::default();
        let subscriber = tracing_subscriber::registry()
            .with(
                tracing_subscriber::fmt::layer()
                    .with_writer(logs.clone())
                    .with_ansi(false),
            )
            .with(
                tracing_subscriber::filter::Targets::new()
                    .with_target("gridops_reconciler", tracing::Level::TRACE)
                    .with_target("gridops_core", tracing::Level::TRACE),
            );
        let guard = tracing::subscriber::set_default(subscriber);
        tracing::info!("GitLab lifecycle log capture");
        (logs, guard)
    }

    fn text(&self) -> String {
        String::from_utf8_lossy(
            &self
                .0
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
        )
        .into_owned()
    }

    fn assert_token_free(&self) {
        let text = self.text();
        assert!(
            text.contains("GitLab lifecycle log capture"),
            "the capture works: {text}"
        );
        assert!(!text.contains("glrt-"), "a runner token was logged: {text}");
    }
}

// ---------------------------------------------------------------------------
// The built-in host (runner manager): Docker and Tart.
// ---------------------------------------------------------------------------

/// Done-when 1 (Docker): the pool's start registers a GitLab runner (the
/// pool's labels as tags, its `run_untagged`, the instance marker naming the
/// runner row), records the GitLab identity on the row, and hands the
/// manager the GitLab request (D10). The `glrt-` token travels only in that
/// request: never in a row, an event or the reconciler's log.
#[tokio::test]
async fn a_docker_start_registers_a_gitlab_runner_and_hands_the_manager_its_token() -> Result<()> {
    let setup = gitlab_setup(&gitlab_pool("pool-gl", "docker", RUNNER_IMAGE, "", 1)).await?;
    ACME.add(&setup, &["pool-gl"]).await?;
    let (logs, _guard) = Logs::capture();

    pass(&setup).await?;

    let rows = rows(&setup, "pool-gl").await?;
    assert_eq!(rows.len(), 1, "{rows:?}");
    let row = &rows[0];
    assert_eq!(
        setup.gitlab.runners(&ACME.url()?),
        [FakeRunner {
            id: ExternalId::new(1)?,
            scope: ACME.scope()?,
            tags: vec!["docker".to_owned(), "linux-large".to_owned()],
            run_untagged: true,
            description: marker(&row.id),
        }],
        "registered on the connection's scope, with the pool's labels and the marker"
    );
    assert_eq!(
        gitlab_calls(&setup, FakeMethod::CreateRunner),
        [ACME.url()?]
    );
    assert_eq!(
        (
            row.ci_platform.as_str(),
            row.gitlab_connection_id.as_deref(),
            row.gitlab_runner_id,
            row.status.as_str(),
        ),
        ("gitlab", Some(ACME.id), Some(1), "online")
    );
    let bodies = manager_bodies(&setup, "pool-gl");
    assert_eq!(bodies.len(), 1, "{bodies:?}");
    let body = &bodies[0];
    assert_eq!(body["provider"], "docker");
    assert_eq!(body["image"], RUNNER_IMAGE);
    assert_eq!(body["platform"], "gitlab");
    assert_eq!(body["mode"], "ephemeral");
    assert_eq!(body["runtime"], "vm");
    assert_eq!(
        Url::parse(body["gitlab"]["url"].as_str().context("gitlab.url")?)?,
        ACME.url()?
    );
    assert_eq!(
        body["gitlab"]["waitTimeoutSeconds"],
        json!(GITLAB_DEFAULT_WAIT_TIMEOUT_SECS)
    );
    assert_eq!(body["gitlabRunnerToken"], fake_token(1));
    for absent in [
        "jitConfig",
        "registrationToken",
        "bitbucket",
        "bitbucketOauthClientSecret",
    ] {
        assert!(body.get(absent).is_none(), "{absent} in {body}");
    }
    assert_no_token_stored(&setup).await?;
    logs.assert_token_free();
    setup.finish().await
}

/// Done-when 1 (Tart) and 3: a Tart-only GitLab pool has no Docker image
/// (`docker_image=''`), and that is no reason to refuse it: its start
/// registers a GitLab runner and sends the pool's Tart image, as a GitLab
/// request.
#[tokio::test]
async fn a_tart_start_registers_a_gitlab_runner_and_sends_the_tart_image() -> Result<()> {
    let setup = gitlab_setup(&gitlab_pool(
        "pool-gl-mac",
        "tart",
        "",
        "gitlab-macos-runner",
        1,
    ))
    .await?;
    ACME.add(&setup, &["pool-gl-mac"]).await?;

    pass(&setup).await?;
    setup.tart_start_reached_manager().await?;
    setup.manager.tart_gate.add_permits(1);
    setup.tart_start_ended().await?;

    assert_eq!(
        gitlab_calls(&setup, FakeMethod::CreateRunner),
        [ACME.url()?]
    );
    let bodies = manager_bodies(&setup, "pool-gl-mac");
    assert_eq!(bodies.len(), 1, "{bodies:?}");
    let body = &bodies[0];
    assert_eq!(
        (
            &body["provider"],
            &body["image"],
            &body["platform"],
            &body["runtime"]
        ),
        (
            &json!("tart"),
            &json!("gitlab-macos-runner"),
            &json!("gitlab"),
            &json!("vm")
        )
    );
    assert_eq!(body["gitlabRunnerToken"], fake_token(1));
    let rows = rows(&setup, "pool-gl-mac").await?;
    assert_eq!(
        rows.iter()
            .map(|row| (row.ci_platform.as_str(), row.gitlab_runner_id))
            .collect::<Vec<_>>(),
        [("gitlab", Some(1))]
    );
    setup.finish().await
}

/// Done-when 4 (manager path, D147-1): a Docker start for a GitLab pool with
/// no Docker image of its own runs the pinned gitlab-runner image, never the
/// GitHub stock image.
#[tokio::test]
async fn a_docker_start_without_a_pool_image_runs_the_pinned_gitlab_runner_image() -> Result<()> {
    let setup = gitlab_setup(&gitlab_pool("pool-gl", "docker", "", "", 1)).await?;
    ACME.add(&setup, &["pool-gl"]).await?;

    pass(&setup).await?;

    assert_eq!(gitlab_calls(&setup, FakeMethod::CreateRunner).len(), 1);
    let bodies = manager_bodies(&setup, "pool-gl");
    assert_eq!(bodies.len(), 1, "{bodies:?}");
    assert_eq!(
        bodies[0]["image"],
        gridops_core::provisioning::GITLAB_RUNNER_IMAGE
    );
    assert_eq!(bodies[0]["platform"], "gitlab");
    setup.finish().await
}

/// Done-when 6 (manager path): the manager failing the start deletes the
/// GitLab runner just registered, releases the capacity lease, fails the
/// runner and counts one failure; once the pause is over the next pass
/// starts again.
#[tokio::test]
async fn a_failed_manager_start_deregisters_the_runner_and_the_next_pass_starts_again() -> Result<()>
{
    let setup = gitlab_setup(&gitlab_pool("pool-gl", "docker", RUNNER_IMAGE, "", 1)).await?;
    ACME.add(&setup, &["pool-gl"]).await?;
    setup.manager.docker_fails.store(true, Ordering::SeqCst);

    pass(&setup).await?;

    assert_eq!(gitlab_calls(&setup, FakeMethod::CreateRunner).len(), 1);
    assert_eq!(
        gitlab_runner_ids(&setup, &ACME)?,
        Vec::<i64>::new(),
        "the registered runner was deleted"
    );
    assert!(
        setup
            .manager
            .other_requests()
            .iter()
            .any(|request| request.starts_with("DELETE /v1/admissions/")),
        "the capacity lease is released: {:?}",
        setup.manager.other_requests()
    );
    let rows_after = rows(&setup, "pool-gl").await?;
    assert_eq!(rows_after.len(), 1);
    assert_eq!(rows_after[0].status, "failed");
    let (failures, open, _, _) = circuit(&setup, "pool-gl").await?;
    assert_eq!((failures, open), (1, false), "counted once");
    assert_no_token_stored(&setup).await?;

    setup.manager.docker_fails.store(false, Ordering::SeqCst);
    end_backoff(&setup, "pool-gl").await?;
    pass(&setup).await?;
    let started = live(&setup, "pool-gl").await?;
    assert_eq!(
        started
            .iter()
            .map(|row| (row.status.as_str(), row.gitlab_runner_id))
            .collect::<Vec<_>>(),
        [("online", Some(2))],
        "the next pass registers and starts a new runner"
    );
    assert_eq!(gitlab_runner_ids(&setup, &ACME)?, [2]);
    setup.finish().await
}

/// Done-when 7 and 10 (manager path), and the probe of round-4 W2: a GitLab
/// runner that exits `completed` or `idle_expired` is deregistered and
/// removed without error, its pool's circuit is reset (it proved the
/// registration works), and `desired_count` replaces it. The start before it,
/// the half-open probe of an open circuit, is one start whatever the pool
/// wants, and a start the manager accepts resets nothing.
#[tokio::test]
async fn an_exited_gitlab_runner_is_deregistered_replaced_and_proves_the_circuit() -> Result<()> {
    for status in [GITLAB_EXIT_COMPLETED, GITLAB_EXIT_IDLE_EXPIRED] {
        let reason = exit_reason(status)?;
        let setup = gitlab_setup(&format!(
            "{} UPDATE runner_pools SET provision_failure_count=3,provision_circuit_open=1,provision_retry_at=1,state='blocked' WHERE id='pool-gl';",
            gitlab_pool("pool-gl", "docker", RUNNER_IMAGE, "", 2)
        ))
        .await?;
        ACME.add(&setup, &["pool-gl"]).await?;

        pass(&setup).await?;
        assert_eq!(
            manager_bodies(&setup, "pool-gl").len(),
            1,
            "{reason}: the probe is one start"
        );
        let (failures, open, _, _) = circuit(&setup, "pool-gl").await?;
        assert_eq!(
            (failures, open),
            (3, true),
            "{reason}: a start the manager accepted proves nothing yet"
        );
        let first = live(&setup, "pool-gl").await?;
        assert_eq!(first.len(), 1, "{reason}");
        assert!(setup.manager.exit(&first[0].id, Some(reason)));

        pass(&setup).await?;

        let all = rows(&setup, "pool-gl").await?;
        let exited = all
            .iter()
            .find(|row| row.id == first[0].id)
            .context("the exited runner's row")?;
        assert!(exited.deleted, "{reason}: removed: {exited:?}");
        assert!(
            !gitlab_runner_ids(&setup, &ACME)?.contains(&1),
            "{reason}: its GitLab runner is deregistered"
        );
        assert_eq!(gitlab_calls(&setup, FakeMethod::DeleteRunner).len(), 1);
        assert_eq!(
            setup
                .github_removals
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .len(),
            0,
            "{reason}: never deregistered from GitHub"
        );
        let (failures, open, retry_at, state) = circuit(&setup, "pool-gl").await?;
        assert_eq!(
            (failures, open, retry_at),
            (0, false, None),
            "{reason}: the circuit is reset"
        );
        assert_ne!(state, "blocked", "{reason}: the pass that reset it");
        assert_eq!(
            live(&setup, "pool-gl").await?.len(),
            2,
            "{reason}: replaced up to the desired count in the same pass"
        );
        let errors: Vec<String> = sqlx::query_scalar(
            "SELECT event FROM runner_events WHERE pool_id='pool-gl' AND level='error'",
        )
        .fetch_all(&setup.app.database)
        .await?;
        assert_eq!(
            errors,
            Vec::<String>::new(),
            "{reason}: removed without error"
        );
        setup.finish().await?;
    }
    Ok(())
}

/// Done-when 7 (drain) and 8: lowering the desired count removes the idle
/// GitLab runner through the GitLab deregistration, never GitHub's.
#[tokio::test]
async fn a_drained_gitlab_runner_is_deregistered_from_gitlab_not_github() -> Result<()> {
    let setup = gitlab_setup(&gitlab_pool("pool-gl", "docker", RUNNER_IMAGE, "", 1)).await?;
    ACME.add(&setup, &["pool-gl"]).await?;
    pass(&setup).await?;
    let started = live(&setup, "pool-gl").await?;
    assert_eq!(started.len(), 1);

    sqlx::query("UPDATE runner_pools SET desired_count=0 WHERE id='pool-gl'")
        .execute(&setup.app.database)
        .await?;
    pass(&setup).await?;

    assert_eq!(live(&setup, "pool-gl").await?.len(), 0, "drained");
    assert_eq!(
        gitlab_calls(&setup, FakeMethod::DeleteRunner),
        [ACME.url()?]
    );
    assert_eq!(gitlab_runner_ids(&setup, &ACME)?, Vec::<i64>::new());
    assert_eq!(
        setup
            .github_removals
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone(),
        Vec::<String>::new(),
        "a GitLab runner is never sent down the GitHub branch"
    );
    setup.finish().await
}

/// Done-when 7 (a failed delete) and 11 (the sweep is its retry): GitLab
/// failing the deregistration is logged, the runner still leaves service and
/// the pass goes on (the replacement starts); the orphan sweep removes the
/// GitLab runner later.
#[tokio::test]
async fn a_failed_gitlab_delete_does_not_fail_the_pass_and_the_sweep_retries_it() -> Result<()> {
    let setup = gitlab_setup(&gitlab_pool("pool-gl", "docker", RUNNER_IMAGE, "", 1)).await?;
    ACME.add(&setup, &["pool-gl"]).await?;
    pass(&setup).await?;
    let first = live(&setup, "pool-gl").await?;
    assert_eq!(first.len(), 1);
    assert!(
        setup
            .manager
            .exit(&first[0].id, Some(exit_reason(GITLAB_EXIT_COMPLETED)?))
    );
    setup.gitlab.fail_next(
        &ACME.url()?,
        FakeMethod::DeleteRunner,
        GitLabError::Upstream("502 from GitLab: internal-detail-7f3a".to_owned()),
    );

    pass(&setup).await?;

    let exited = rows(&setup, "pool-gl")
        .await?
        .into_iter()
        .find(|row| row.id == first[0].id)
        .context("the exited runner's row")?;
    assert!(exited.deleted, "it left service: {exited:?}");
    let failed_passes: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM runner_events WHERE pool_id='pool-gl' AND event='Pool reconciliation failed'",
    )
    .fetch_one(&setup.app.database)
    .await?;
    assert_eq!(failed_passes, 0, "the pass did not fail");
    assert_eq!(live(&setup, "pool-gl").await?.len(), 1, "replaced");
    assert_eq!(
        gitlab_runner_ids(&setup, &ACME)?,
        [1, 2],
        "the first is left for the sweep"
    );
    // #171: GitLab refusing the delete is shown on the pool, not only logged.
    // A 502 is not a scope problem, so the warning does not send the operator
    // after manage_runner.
    let refused = refusals(&setup, "pool-gl").await?;
    assert_eq!(refused.len(), 1, "{refused:?}");
    assert_refusal(&refused[0], ACME.name, 1, "GitLab request failed");
    assert_eq!(refused[0].runner_id.as_deref(), Some(first[0].id.as_str()));
    assert!(!refused[0].message.contains("manage_runner"), "{refused:?}");
    // The upstream detail is free-form text: it stays in the log, as the
    // API keeps it out of its responses, and is not in the event.
    for text in [&refused[0].message, &refused[0].metadata] {
        assert!(
            !text.contains("internal-detail-7f3a") && !text.contains("502"),
            "no upstream detail in the event: {refused:?}"
        );
    }

    // The sweep's interval has passed.
    sqlx::query("UPDATE settings SET value='0' WHERE key='lastGitlabRunnerSweepAt'")
        .execute(&setup.app.database)
        .await?;
    pass(&setup).await?;
    assert_eq!(gitlab_runner_ids(&setup, &ACME)?, [2], "swept");
    setup.finish().await
}

/// #171: a runner leaving service whose GitLab deletion is refused for a
/// missing `manage_runner` writes one warning on its pool, linked to the
/// runner row, naming the connection, the GitLab runner and the scope; a
/// runner GitLab no longer has (`NotFound`) is already gone and writes none.
#[tokio::test]
async fn a_refused_gitlab_delete_is_a_pool_warning_and_a_runner_already_gone_is_not() -> Result<()>
{
    let setup = gitlab_setup(&gitlab_pool("pool-gl", "docker", RUNNER_IMAGE, "", 1)).await?;
    ACME.add(&setup, &["pool-gl"]).await?;
    pass(&setup).await?;

    // GitLab no longer has the first runner.
    let first = live(&setup, "pool-gl").await?;
    assert_eq!(first.len(), 1);
    assert!(
        setup
            .manager
            .exit(&first[0].id, Some(exit_reason(GITLAB_EXIT_COMPLETED)?))
    );
    setup.gitlab.fail_next(
        &ACME.url()?,
        FakeMethod::DeleteRunner,
        GitLabError::NotFound,
    );
    pass(&setup).await?;
    assert_eq!(
        refusals(&setup, "pool-gl").await?.len(),
        0,
        "a runner already gone is no warning"
    );

    // The token cannot delete its replacement.
    let second = live(&setup, "pool-gl").await?;
    assert_eq!(second.len(), 1, "replaced");
    assert_eq!(second[0].gitlab_runner_id, Some(2));
    assert!(
        setup
            .manager
            .exit(&second[0].id, Some(exit_reason(GITLAB_EXIT_COMPLETED)?))
    );
    setup.gitlab.fail_next(
        &ACME.url()?,
        FakeMethod::DeleteRunner,
        missing_manage_runner(),
    );
    pass(&setup).await?;

    let refused = refusals(&setup, "pool-gl").await?;
    assert_eq!(
        refused.len(),
        1,
        "one warning for the one refusal: {refused:?}"
    );
    assert_refusal(&refused[0], ACME.name, 2, "manage_runner");
    assert_eq!(
        refused[0].runner_id.as_deref(),
        Some(second[0].id.as_str()),
        "linked to the runner row"
    );
    assert!(
        gitlab_runner_ids(&setup, &ACME)?.contains(&2),
        "left for the sweep"
    );
    assert_no_token_stored(&setup).await?;
    setup.finish().await
}

/// #171: the failed-start cleanup (the manager refused the start, so the
/// GitLab runner just registered is deleted) records a refused deletion on
/// the pool too, linked to the failed runner row.
#[tokio::test]
async fn a_refused_delete_after_a_failed_manager_start_is_a_pool_warning() -> Result<()> {
    let setup = gitlab_setup(&gitlab_pool("pool-gl", "docker", RUNNER_IMAGE, "", 1)).await?;
    ACME.add(&setup, &["pool-gl"]).await?;
    setup.manager.docker_fails.store(true, Ordering::SeqCst);
    setup.gitlab.fail_next(
        &ACME.url()?,
        FakeMethod::DeleteRunner,
        missing_manage_runner(),
    );

    pass(&setup).await?;

    let failed = rows(&setup, "pool-gl").await?;
    assert_eq!(failed.len(), 1);
    assert_eq!(failed[0].status, "failed");
    assert_eq!(
        gitlab_runner_ids(&setup, &ACME)?,
        [1],
        "GitLab refused the delete"
    );
    let refused = refusals(&setup, "pool-gl").await?;
    assert_eq!(refused.len(), 1, "{refused:?}");
    assert_refusal(&refused[0], ACME.name, 1, "manage_runner");
    assert_eq!(refused[0].runner_id.as_deref(), Some(failed[0].id.as_str()));
    setup.finish().await
}

/// Done-when 9 and 10 (manager path), round-4 block: a GitLab runner that
/// exits `registration_refused` fails with a readable diagnostic and counts
/// one failure; the pass that counts it starts nothing more and ends in
/// backoff, and later passes seeing the same refusal count nothing more.
#[tokio::test]
async fn a_refused_registration_counts_once_with_a_readable_diagnostic() -> Result<()> {
    let refused = exit_reason(GITLAB_EXIT_REGISTRATION_REFUSED)?;
    let setup = gitlab_setup(&gitlab_pool("pool-gl", "docker", RUNNER_IMAGE, "", 1)).await?;
    ACME.add(&setup, &["pool-gl"]).await?;
    pass(&setup).await?;
    let first = live(&setup, "pool-gl").await?;
    assert_eq!(first.len(), 1);
    assert!(setup.manager.exit(&first[0].id, Some(refused)));

    pass(&setup).await?;

    let (failures, open, _, state) = circuit(&setup, "pool-gl").await?;
    assert_eq!((failures, open), (1, false), "counted once");
    assert_eq!(state, "backoff", "the counting pass ends in backoff");
    assert_eq!(
        gitlab_calls(&setup, FakeMethod::CreateRunner).len(),
        1,
        "and starts nothing while the backoff holds"
    );
    assert_eq!(live(&setup, "pool-gl").await?.len(), 0);
    assert_eq!(
        gitlab_runner_ids(&setup, &ACME)?,
        Vec::<i64>::new(),
        "deregistered"
    );
    assert_readable(&setup, "pool-gl", refused).await?;

    for _ in 0..2 {
        pass(&setup).await?;
    }
    let (failures, open, _, _) = circuit(&setup, "pool-gl").await?;
    assert_eq!((failures, open), (1, false), "still one refusal");
    assert_eq!(gitlab_calls(&setup, FakeMethod::CreateRunner).len(), 1);
    assert_no_token_stored(&setup).await?;
    setup.finish().await
}

/// #173 (manager path): a Tart GitLab runner whose image has no
/// `gitlab-runner` exits 78, which the Tart agent lists as `runner_missing`.
/// That is a provisioning failure, as a refusal is: the runner fails with a
/// reason naming the problem and its fix, the pool gets an error event, and
/// its failure count rises by one and backs off, once however many passes
/// see the exit, keeping the failures counted before it. A runner that
/// completes later still clears them.
#[tokio::test]
async fn a_tart_runner_without_gitlab_runner_counts_once_with_a_readable_reason() -> Result<()> {
    // Typed here, not looked up, so that before #173 the pass, not the
    // lookup, is what fails; the lookup at the end ties it to the table.
    let missing = "runner_missing";
    let setup = gitlab_setup(&format!(
        "{} UPDATE runner_pools SET provision_failure_count=1 WHERE id='pool-gl-mac';",
        gitlab_pool("pool-gl-mac", "tart", "", "gitlab-macos-runner", 1)
    ))
    .await?;
    ACME.add(&setup, &["pool-gl-mac"]).await?;
    pass(&setup).await?;
    setup.tart_start_reached_manager().await?;
    setup.manager.tart_gate.add_permits(1);
    setup.tart_start_ended().await?;
    let first = live(&setup, "pool-gl-mac").await?;
    assert_eq!(first.len(), 1, "{first:?}");
    assert!(setup.manager.exit(&first[0].id, Some(missing)));

    pass(&setup).await?;

    let (failures, open, retry_at, state) = circuit(&setup, "pool-gl-mac").await?;
    assert_eq!(
        (failures, open),
        (2, false),
        "one failure more, the one before it kept"
    );
    assert_eq!(state, "backoff", "the counting pass ends in backoff");
    assert!(
        retry_at.is_some_and(|at| at > now_millis()),
        "a retry time ahead: {retry_at:?}"
    );
    assert_eq!(live(&setup, "pool-gl-mac").await?.len(), 0);
    assert_eq!(
        gitlab_runner_ids(&setup, &ACME)?,
        Vec::<i64>::new(),
        "deregistered"
    );
    assert_runner_missing(&setup, "pool-gl-mac", &first[0].id, missing).await?;

    for _ in 0..2 {
        pass(&setup).await?;
    }
    assert_eq!(
        circuit(&setup, "pool-gl-mac").await?.0,
        2,
        "later passes seeing the same exit count nothing more"
    );
    assert_eq!(
        gitlab_calls(&setup, FakeMethod::CreateRunner).len(),
        1,
        "and start nothing while the backoff holds"
    );
    assert_runner_missing(&setup, "pool-gl-mac", &first[0].id, missing).await?;
    assert_no_token_stored(&setup).await?;

    // Once the pause is over, a runner that completes clears the count.
    end_backoff(&setup, "pool-gl-mac").await?;
    pass(&setup).await?;
    tokio::time::timeout(Duration::from_secs(5), async {
        while setup.manager.starts("tart").len() < 2 {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .context("the second Tart start did not reach the manager")?;
    setup.manager.tart_gate.add_permits(1);
    setup.tart_start_ended().await?;
    let second = live(&setup, "pool-gl-mac").await?;
    assert_eq!(second.len(), 1, "{second:?}");
    assert!(
        setup
            .manager
            .exit(&second[0].id, Some(exit_reason(GITLAB_EXIT_COMPLETED)?))
    );
    pass(&setup).await?;
    let (failures, open, retry_at, _) = circuit(&setup, "pool-gl-mac").await?;
    assert_eq!(
        (failures, open, retry_at),
        (0, false, None),
        "a completed runner clears the failures"
    );

    assert_eq!(
        exit_reason(78)?,
        missing,
        "the published name of exit 78 (EX_CONFIG)"
    );
    setup.finish().await
}

/// Done-when 10 (manager path): three distinct refused runners open the
/// pool's circuit, which then holds the next start back.
#[tokio::test]
async fn three_refused_gitlab_runners_open_the_circuit() -> Result<()> {
    let refused = exit_reason(GITLAB_EXIT_REGISTRATION_REFUSED)?;
    let setup = gitlab_setup(&gitlab_pool("pool-gl", "docker", RUNNER_IMAGE, "", 1)).await?;
    ACME.add(&setup, &["pool-gl"]).await?;
    for attempt in 1..=3 {
        pass(&setup).await?;
        let started = live(&setup, "pool-gl").await?;
        assert_eq!(started.len(), 1, "attempt {attempt}");
        assert!(setup.manager.exit(&started[0].id, Some(refused)));
        pass(&setup).await?;
        if attempt < 3 {
            end_backoff(&setup, "pool-gl").await?;
        }
    }

    let (failures, open, _, state) = circuit(&setup, "pool-gl").await?;
    assert_eq!((failures, open, state.as_str()), (3, true, "blocked"));
    let opened: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM runner_events WHERE pool_id='pool-gl' AND event='Provisioning circuit opened'",
    )
    .fetch_one(&setup.app.database)
    .await?;
    assert_eq!(opened, 1);
    pass(&setup).await?;
    assert_eq!(
        gitlab_calls(&setup, FakeMethod::CreateRunner).len(),
        3,
        "the open circuit holds the next start back"
    );
    setup.finish().await
}

/// Done-when 10, round-3 block 1 (manager path): the reset happens once, when
/// a runner first leaves service. A completed runner whose removal keeps
/// failing is seen exited on every pass: it resets the count the first time,
/// and must not reset a count that later refusals raised.
#[tokio::test]
async fn an_old_completed_runner_does_not_reset_a_count_raised_after_it() -> Result<()> {
    let setup = gitlab_setup(&gitlab_pool("pool-gl", "docker", RUNNER_IMAGE, "", 1)).await?;
    ACME.add(&setup, &["pool-gl"]).await?;
    pass(&setup).await?;
    let first = live(&setup, "pool-gl").await?;
    assert_eq!(first.len(), 1);
    sqlx::query("UPDATE runner_pools SET provision_failure_count=1 WHERE id='pool-gl'")
        .execute(&setup.app.database)
        .await?;
    setup.manager.removal_fails.store(true, Ordering::SeqCst);
    assert!(
        setup
            .manager
            .exit(&first[0].id, Some(exit_reason(GITLAB_EXIT_COMPLETED)?))
    );
    pass(&setup).await?;
    assert_eq!(
        circuit(&setup, "pool-gl").await?.0,
        0,
        "the completed runner reset the count when it first left service"
    );
    // Refusals of other runners raised the count since.
    sqlx::query(
        "UPDATE runner_pools SET provision_failure_count=2,provision_retry_at=1 WHERE id='pool-gl'",
    )
    .execute(&setup.app.database)
    .await?;

    for _ in 0..2 {
        pass(&setup).await?;
    }

    let (failures, _, _, _) = circuit(&setup, "pool-gl").await?;
    assert_eq!(failures, 2, "the old completed runner reset nothing again");
    setup.finish().await
}

/// Done-when 16, brief-gate W-3 and W4: with GitLab's client not wired yet
/// (every `create_runner` fails), each start fails at registration and counts
/// once, no runner is left starting, the capacity lease is released and no
/// GitLab runner exists; the pass still returns `Ok` and reconciles the other
/// pools; after three failures the circuit opens, with a readable reason,
/// and holds further attempts back.
#[tokio::test]
async fn an_unwired_gitlab_client_fails_each_start_cleanly_until_the_circuit_opens() -> Result<()> {
    let setup = gitlab_setup(&format!(
        "{} UPDATE runner_pools SET desired_count=1 WHERE id='pool-linux';",
        gitlab_pool("pool-gl", "docker", RUNNER_IMAGE, "", 1)
    ))
    .await?;
    ACME.add(&setup, &["pool-gl"]).await?;
    for _ in 0..4 {
        setup.gitlab.fail_next(
            &ACME.url()?,
            FakeMethod::CreateRunner,
            GitLabError::Upstream(PLACEHOLDER_ERROR.to_owned()),
        );
    }

    for attempt in 1..=3 {
        reconcile(&setup.app).await?;
        let (failures, open, _, _) = circuit(&setup, "pool-gl").await?;
        assert_eq!(
            (failures, open),
            (attempt, attempt == 3),
            "attempt {attempt}"
        );
        assert!(
            rows(&setup, "pool-gl")
                .await?
                .iter()
                .all(|row| row.status != "starting"),
            "attempt {attempt}: no runner left starting"
        );
        assert_eq!(
            gitlab_runner_ids(&setup, &ACME)?,
            Vec::<i64>::new(),
            "attempt {attempt}"
        );
        if attempt < 3 {
            end_backoff(&setup, "pool-gl").await?;
        }
    }
    let releases = setup
        .manager
        .other_requests()
        .into_iter()
        .filter(|request| request.starts_with("DELETE /v1/admissions/"))
        .count();
    assert_eq!(
        releases, 3,
        "every lease taken for a failed start is released"
    );
    assert_eq!(
        setup.manager.starts("docker"),
        ["pool-linux"],
        "the other pool was reconciled; the GitLab pool never reached the manager"
    );
    assert_eq!(circuit(&setup, "pool-gl").await?.3, "blocked");
    assert_readable(&setup, "pool-gl", PLACEHOLDER_ERROR).await?;

    reconcile(&setup.app).await?;
    assert_eq!(
        gitlab_calls(&setup, FakeMethod::CreateRunner).len(),
        3,
        "no attempt while the circuit is open"
    );
    setup.finish().await
}

// ---------------------------------------------------------------------------
// The orphan sweep (D17).
// ---------------------------------------------------------------------------

/// A runner row of `pool_id` in `status`, deleted or not: its id.
async fn runner_row(setup: &Setup, pool_id: &str, status: &str, deleted: bool) -> Result<String> {
    let id = uuid::Uuid::new_v4().to_string();
    sqlx::query(
        "INSERT INTO runners (id,pool_id,name,provider,ci_platform,gitlab_connection_id,status,ephemeral,configuration_version,deleted_at,created_at,updated_at)
         VALUES (?,?,?,'docker','gitlab',NULL,?,1,1,?,1,1)",
    )
    .bind(&id)
    .bind(pool_id)
    .bind(format!("runner-{}", &id[..8]))
    .bind(status)
    .bind(deleted.then_some(2_i64))
    .execute(&setup.app.database)
    .await?;
    Ok(id)
}

/// Seeds a GitLab runner `GridOps` may or may not have created, on the
/// connection's scope.
fn seed(setup: &Setup, connection: &Connection, id: u64, description: &str) -> Result<()> {
    setup.gitlab.with_runner(
        &connection.url()?,
        FakeRunner {
            id: ExternalId::new(id)?,
            scope: connection.scope()?,
            tags: Vec::new(),
            run_untagged: false,
            description: description.to_owned(),
        },
    );
    Ok(())
}

/// Done-when 11: the sweep deletes a GitLab runner marked by this deployment
/// whose runner row is gone or deleted, including after the connection's only
/// pool was deleted (its rows cascaded away); it keeps a runner whose row is
/// live, `starting` included, and never touches a runner it did not mark,
/// one another deployment marked, or a malformed marker. It records when it
/// ran.
#[tokio::test]
async fn the_sweep_deletes_only_this_deployments_orphans() -> Result<()> {
    const GONE: Connection = Connection {
        id: "gl-gone",
        name: "Gone",
        base_url: "https://gitlab-two.example.test",
        kind: "group",
        scope_id: 7,
        path: "gone",
        created_at: 2,
    };
    // Two live rows and a pool that wants two: nothing is started or drained.
    let setup = gitlab_setup(&format!(
        "{} {}",
        gitlab_pool("pool-gl", "docker", RUNNER_IMAGE, "", 2),
        gitlab_pool("pool-gl-gone", "docker", RUNNER_IMAGE, "", 0)
    ))
    .await?;
    ACME.add(&setup, &["pool-gl"]).await?;
    GONE.add(&setup, &["pool-gl-gone"]).await?;
    let starting = runner_row(&setup, "pool-gl", "starting", false).await?;
    let online = runner_row(&setup, "pool-gl", "online", false).await?;
    let deleted = runner_row(&setup, "pool-gl", "deleted", true).await?;
    let cascaded = runner_row(&setup, "pool-gl-gone", "online", false).await?;
    assert!(targets::delete_pool(&setup.app.database, "pool-gl-gone").await?);
    let never_existed = uuid::Uuid::new_v4().to_string();
    seed(&setup, &ACME, 11, &marker(&starting))?;
    seed(&setup, &ACME, 12, &marker(&online))?;
    seed(&setup, &ACME, 13, &marker(&deleted))?;
    seed(&setup, &ACME, 14, &marker(&never_existed))?;
    seed(&setup, &ACME, 15, "a laptop runner")?;
    seed(
        &setup,
        &ACME,
        16,
        &format!("gridops-runner:other.example.test:{never_existed}"),
    )?;
    seed(
        &setup,
        &ACME,
        17,
        &format!("gridops-runner:{INSTANCE}:not-a-uuid"),
    )?;
    seed(&setup, &GONE, 21, &marker(&cascaded))?;

    pass(&setup).await?;

    assert_eq!(
        gitlab_runner_ids(&setup, &ACME)?,
        [11, 12, 15, 16, 17],
        "the deleted row's and the missing row's runners are swept"
    );
    assert_eq!(
        gitlab_runner_ids(&setup, &GONE)?,
        Vec::<i64>::new(),
        "swept after its only pool was deleted"
    );
    let swept_at: Option<String> =
        sqlx::query_scalar("SELECT value FROM settings WHERE key='lastGitlabRunnerSweepAt'")
            .fetch_optional(&setup.app.database)
            .await?;
    assert!(swept_at.is_some(), "the sweep records when it ran");
    setup.finish().await
}

/// Done-when 11: a connection whose listing or deletion fails is logged and
/// skipped; the sweep goes on to the others and the pass returns `Ok`, the
/// pools reconciled. The sweep runs once per interval
/// (`gitlabRunnerSweepIntervalSeconds`, 600 s by default), so a failed
/// delete is retried when the interval has passed, not every pass.
#[tokio::test]
async fn a_failing_connection_does_not_stop_the_sweep_which_respects_its_interval() -> Result<()> {
    // Created first, so they come first whatever order the sweep takes.
    const LIST_FAILS: Connection = Connection {
        id: "gl-0-list-fails",
        name: "List fails",
        base_url: "https://gitlab-one.example.test",
        kind: "project",
        scope_id: 1,
        path: "one/project",
        created_at: 1,
    };
    const DELETE_FAILS: Connection = Connection {
        id: "gl-1-delete-fails",
        name: "Delete fails",
        base_url: "https://gitlab-two.example.test",
        kind: "project",
        scope_id: 2,
        path: "two/project",
        created_at: 2,
    };
    const HEALTHY: Connection = Connection {
        id: "gl-2-healthy",
        name: "Healthy",
        base_url: "https://gitlab-three.example.test",
        kind: "group",
        scope_id: 3,
        path: "three",
        created_at: 3,
    };
    let setup =
        gitlab_setup("UPDATE runner_pools SET desired_count=1 WHERE id='pool-linux';").await?;
    for connection in [&LIST_FAILS, &DELETE_FAILS, &HEALTHY] {
        connection.add(&setup, &[]).await?;
    }
    let orphan = |id: u64| marker(&format!("00000000-0000-4000-8000-{id:012}"));
    seed(&setup, &LIST_FAILS, 1, &orphan(1))?;
    seed(&setup, &DELETE_FAILS, 2, &orphan(2))?;
    seed(&setup, &HEALTHY, 3, &orphan(3))?;
    setup.gitlab.fail_next(
        &LIST_FAILS.url()?,
        FakeMethod::ListRunners,
        GitLabError::Upstream("502 from GitLab".to_owned()),
    );
    setup.gitlab.fail_next(
        &DELETE_FAILS.url()?,
        FakeMethod::DeleteRunner,
        GitLabError::RateLimited { retry_after: None },
    );

    reconcile(&setup.app).await?;

    assert_eq!(gitlab_runner_ids(&setup, &LIST_FAILS)?, [1]);
    assert_eq!(gitlab_runner_ids(&setup, &DELETE_FAILS)?, [2]);
    assert_eq!(
        gitlab_runner_ids(&setup, &HEALTHY)?,
        Vec::<i64>::new(),
        "the healthy connection was swept"
    );
    assert_eq!(
        setup.manager.starts("docker"),
        ["pool-linux"],
        "the pass went on to the pools"
    );

    // Within the interval: nothing is listed again.
    seed(&setup, &HEALTHY, 4, &orphan(4))?;
    let listings = gitlab_calls(&setup, FakeMethod::ListRunners).len();
    reconcile(&setup.app).await?;
    assert_eq!(
        gitlab_calls(&setup, FakeMethod::ListRunners).len(),
        listings
    );
    assert_eq!(gitlab_runner_ids(&setup, &HEALTHY)?, [4]);

    // Once the interval has passed, everything left is swept.
    sqlx::query("UPDATE settings SET value=? WHERE key='lastGitlabRunnerSweepAt'")
        .bind((now_millis() - 601_000).to_string())
        .execute(&setup.app.database)
        .await?;
    reconcile(&setup.app).await?;
    for connection in [&LIST_FAILS, &DELETE_FAILS, &HEALTHY] {
        assert_eq!(
            gitlab_runner_ids(&setup, connection)?,
            Vec::<i64>::new(),
            "{}",
            connection.id
        );
    }
    setup.finish().await
}

/// The sweep's interval has passed: the next pass sweeps again.
async fn sweep_again(setup: &Setup) -> Result<()> {
    sqlx::query("UPDATE settings SET value='0' WHERE key='lastGitlabRunnerSweepAt'")
        .execute(&setup.app.database)
        .await?;
    reconcile(&setup.app).await
}

/// How many refused-deletion warnings `pool-a` and `pool-b` hold.
async fn counts(setup: &Setup) -> Result<(usize, usize)> {
    Ok((
        refusals(setup, "pool-a").await?.len(),
        refusals(setup, "pool-b").await?.len(),
    ))
}

/// #171: the orphan sweep's refused deletions are warnings on every pool
/// linked to the connection (no runner row to link: `runner_id` stays NULL),
/// deduplicated to at most one per pool, connection and reason an hour, so a
/// token that keeps failing does not write a row per runner per sweep. A new
/// reason, or the hour passing, writes again. A warning that cannot be
/// written never stops the sweep.
#[tokio::test]
async fn the_sweeps_refused_deletions_warn_each_linked_pool_once_an_hour_per_reason() -> Result<()>
{
    const OTHER: Connection = Connection {
        id: "gl-other",
        name: "Other GitLab",
        base_url: "https://gitlab-other.example.test",
        kind: "project",
        scope_id: 9,
        path: "other/project",
        created_at: 2,
    };
    // Each backdating moves the warnings two hours back (more than the hour's
    // window); it is applied twice, so the oldest rows end up four hours old.
    const TWO_HOURS: i64 = 2 * 3_600_000;
    let setup = gitlab_setup(&format!(
        "{} {}",
        gitlab_pool("pool-a", "docker", RUNNER_IMAGE, "", 0),
        gitlab_pool("pool-b", "docker", RUNNER_IMAGE, "", 0)
    ))
    .await?;
    ACME.add(&setup, &["pool-a", "pool-b"]).await?;
    OTHER.add(&setup, &["pool-a"]).await?;
    let orphan = |id: u64| marker(&format!("00000000-0000-4000-8000-{id:012}"));
    seed(&setup, &ACME, 31, &orphan(31))?;
    seed(&setup, &ACME, 32, &orphan(32))?;
    seed(&setup, &OTHER, 41, &orphan(41))?;
    let refuse = |connection: &Connection, times: usize, error: &GitLabError| -> Result<()> {
        for _ in 0..times {
            setup
                .gitlab
                .fail_next(&connection.url()?, FakeMethod::DeleteRunner, error.clone());
        }
        Ok(())
    };

    // Both of Acme's orphans and Other's one are refused for manage_runner.
    refuse(&ACME, 2, &missing_manage_runner())?;
    refuse(&OTHER, 1, &missing_manage_runner())?;
    reconcile(&setup.app).await?;
    assert_eq!(gitlab_runner_ids(&setup, &ACME)?, [31, 32]);
    assert_eq!(gitlab_runner_ids(&setup, &OTHER)?, [41]);
    let pool_a = refusals(&setup, "pool-a").await?;
    let pool_b = refusals(&setup, "pool-b").await?;
    assert_eq!(
        (pool_a.len(), pool_b.len()),
        (2, 1),
        "one per pool and connection, not one per runner: {pool_a:?} {pool_b:?}"
    );
    for event in pool_a.iter().chain(&pool_b) {
        assert_eq!(event.runner_id, None, "{event:?}");
    }
    let acme_on_a = pool_a
        .iter()
        .find(|event| event.message.contains(ACME.name))
        .context("Acme's warning on pool-a")?;
    let acme_runner = if names_number(&acme_on_a.message, 31) {
        31
    } else {
        32
    };
    assert_refusal(acme_on_a, ACME.name, acme_runner, "manage_runner");
    let other_on_a = pool_a
        .iter()
        .find(|event| event.message.contains(OTHER.name))
        .context("Other's warning on pool-a")?;
    assert_refusal(other_on_a, OTHER.name, 41, "manage_runner");
    assert!(pool_b[0].message.contains(ACME.name), "{pool_b:?}");
    assert_refusal(&pool_b[0], ACME.name, acme_runner, "manage_runner");

    // The next sweep, within the hour, is refused for the same reason.
    refuse(&ACME, 2, &missing_manage_runner())?;
    refuse(&OTHER, 1, &missing_manage_runner())?;
    sweep_again(&setup).await?;
    assert_eq!(
        gitlab_calls(&setup, FakeMethod::DeleteRunner).len(),
        6,
        "retried"
    );
    assert_eq!(counts(&setup).await?, (2, 1), "nothing new within the hour");

    // Within the hour, but for another reason: a new warning per pool.
    refuse(&ACME, 2, &GitLabError::Unauthorized)?;
    refuse(&OTHER, 1, &missing_manage_runner())?;
    sweep_again(&setup).await?;
    assert_eq!(counts(&setup).await?, (3, 2), "a new reason warns again");

    // An hour later the same reason warns again.
    sqlx::query("UPDATE runner_events SET created_at=created_at-? WHERE event=?")
        .bind(TWO_HOURS)
        .bind(DELETION_REFUSED)
        .execute(&setup.app.database)
        .await?;
    refuse(&ACME, 2, &missing_manage_runner())?;
    refuse(&OTHER, 1, &missing_manage_runner())?;
    sweep_again(&setup).await?;
    assert_eq!(
        counts(&setup).await?,
        (5, 3),
        "after the hour, once more per pool"
    );

    // A warning that cannot be written: the sweep still deletes what it can.
    sqlx::query("UPDATE runner_events SET created_at=created_at-? WHERE event=?")
        .bind(TWO_HOURS)
        .bind(DELETION_REFUSED)
        .execute(&setup.app.database)
        .await?;
    sqlx::raw_sql(
        "CREATE TRIGGER fail_refusal_event BEFORE INSERT ON runner_events
           WHEN NEW.event='GitLab runner not deleted'
         BEGIN SELECT RAISE(ABORT,'disk I/O error'); END;",
    )
    .execute(&setup.app.database)
    .await?;
    refuse(&ACME, 1, &missing_manage_runner())?;
    sweep_again(&setup).await?;
    assert_eq!(
        gitlab_runner_ids(&setup, &ACME)?.len(),
        1,
        "the orphan after the refused one is still deleted"
    );
    assert_eq!(
        gitlab_runner_ids(&setup, &OTHER)?,
        Vec::<i64>::new(),
        "and the next connection is swept"
    );
    assert_no_token_stored(&setup).await?;
    setup.finish().await
}

/// #171: the sweep's warning for one pool that cannot be written does not
/// keep the connection's other linked pools from being told.
#[tokio::test]
async fn a_pool_whose_sweep_warning_cannot_be_written_does_not_silence_the_others() -> Result<()> {
    let setup = gitlab_setup(&format!(
        "{} {}",
        gitlab_pool("pool-a", "docker", RUNNER_IMAGE, "", 0),
        gitlab_pool("pool-b", "docker", RUNNER_IMAGE, "", 0)
    ))
    .await?;
    ACME.add(&setup, &["pool-a", "pool-b"]).await?;
    seed(
        &setup,
        &ACME,
        31,
        &marker("00000000-0000-4000-8000-000000000031"),
    )?;
    // Pools are told in id order: pool-a, the first, is the one that fails.
    sqlx::raw_sql(
        "CREATE TRIGGER fail_pool_a_refusal BEFORE INSERT ON runner_events
           WHEN NEW.event='GitLab runner not deleted' AND NEW.pool_id='pool-a'
         BEGIN SELECT RAISE(ABORT,'disk I/O error'); END;",
    )
    .execute(&setup.app.database)
    .await?;
    setup.gitlab.fail_next(
        &ACME.url()?,
        FakeMethod::DeleteRunner,
        missing_manage_runner(),
    );

    reconcile(&setup.app).await?;

    assert_eq!(
        gitlab_runner_ids(&setup, &ACME)?,
        [31],
        "left for the sweep"
    );
    assert_eq!(counts(&setup).await?, (0, 1), "pool-b is still told");
    let pool_b = refusals(&setup, "pool-b").await?;
    assert_refusal(&pool_b[0], ACME.name, 31, "manage_runner");
    setup.finish().await
}

// ---------------------------------------------------------------------------
// A fleet machine. The tests play its agent: they answer the commands the
// reconciler issues by recording results through the real registry, as the
// host agent does.
// ---------------------------------------------------------------------------

const HOST: &str = "b1000000-0000-4000-8000-000000000001";
const DOMAIN: &str = "b2000000-0000-4000-8000-000000000001";
const BACKEND: &str = "b3000000-0000-4000-8000-000000000001";
const SESSION: &str = "b4000000-0000-4000-8000-000000000001";
const INCARNATION: &str = "b5000000-0000-4000-8000-000000000001";
const PROFILE: &str = "b6000000-0000-4000-8000-000000000001";
const BOOT: &str = "boot-gitlab-fleet";

/// A GitLab pool (`pool-gl`) that runs on one approved, active Linux Docker
/// machine with work authority, granted the pool's connection's target. From
/// [`Fleet::new`] the pool keeps one runner (`min_count` 1, autoscaling off);
/// from [`Fleet::autoscaling`] it scales from nothing on its pending jobs.
struct Fleet {
    setup: Setup,
    registry: RegistryService,
    host: AuthenticatedHost,
}

impl Fleet {
    async fn new(requires_docker_socket: bool) -> Result<Self> {
        Self::with_pool(
            requires_docker_socket,
            &format!(
                "{} UPDATE runner_pools SET min_count=1 WHERE id='pool-gl';",
                gitlab_pool("pool-gl", "docker", RUNNER_IMAGE, "", 1)
            ),
        )
        .await
    }

    /// #181: `pool-gl` autoscaling from `min_count` 0 up to `max_count`, as
    /// prod's `gl-e2e-prod` was: wanting nothing until its pending jobs raise
    /// its desired count. Its queue scale factor is the default, 1.
    async fn autoscaling(max_count: i64) -> Result<Self> {
        Self::with_pool(
            true,
            &format!(
                "{} UPDATE runner_pools SET min_count=0,max_count={max_count},autoscaling_enabled=1 WHERE id='pool-gl';",
                gitlab_pool("pool-gl", "docker", RUNNER_IMAGE, "", 0)
            ),
        )
        .await
    }

    async fn with_pool(requires_docker_socket: bool, pool_sql: &str) -> Result<Self> {
        let setup = gitlab_setup(pool_sql).await?;
        ACME.add(&setup, &["pool-gl"]).await?;
        let database = &setup.app.database;
        let now = now_millis();
        sqlx::query(
            "INSERT INTO fleet_control_plane (singleton,incarnation,readiness,schema_version,protocol_min,protocol_max,updated_at)
             VALUES (1,?,'active',1,1,1,?)
             ON CONFLICT(singleton) DO UPDATE SET incarnation=excluded.incarnation,readiness='active'",
        )
        .bind(INCARNATION)
        .bind(now)
        .execute(database)
        .await?;
        sqlx::query(
            "INSERT INTO fleet_hosts (id,name,host_os,architecture,enrollment_state,lifecycle_state,integrity_state,epoch,
               boot_id,agent_session_id,authority_incarnation,last_heartbeat_at,last_inventory_at,authority_expires_at,
               inventory_complete,inventory_epoch,inventory_session_id,inventory_boot_id,pressure_state,created_at,updated_at)
             VALUES (?,'gitlab-machine','linux','x64','approved','active','verified',1,?,?,?,?,?,?,1,1,?,?,'normal',1,1)",
        )
        .bind(HOST)
        .bind(BOOT)
        .bind(SESSION)
        .bind(INCARNATION)
        .bind(now)
        .bind(now)
        .bind(now + 90_000)
        .bind(SESSION)
        .bind(BOOT)
        .execute(database)
        .await?;
        sqlx::query(
            "INSERT INTO host_resource_domains (id,host_id,parent_domain_id,name,cpu_millis,memory_mib,disk_bytes)
             VALUES (?,?,NULL,'Physical',8000,16384,107374182400)",
        )
        .bind(DOMAIN)
        .bind(HOST)
        .execute(database)
        .await?;
        let capabilities = json!({
            "schema_version": 1,
            "runtime_kind": "docker",
            "execution_os": "linux",
            "architecture": "x64",
            "supported_runners": [
                {"target_kind": "gitlab_project", "mode": "ephemeral"},
                {"target_kind": "gitlab_group", "mode": "ephemeral"},
            ],
            "environments": [{"reference": RUNNER_IMAGE, "readiness": "ready", "version": null}],
            "resources": {"cpu": "reservation", "memory": "reservation", "disk": "estimate"},
            "interactive": "unsupported",
            "docker_socket": "requires_explicit_grant",
        });
        sqlx::query(
            "INSERT INTO host_backends (id,host_id,domain_id,runtime_kind,execution_os,architecture,readiness,enabled,
               capabilities_json,last_probed_at,probe_epoch,probe_session_id,probe_boot_id,probe_incarnation,created_at,updated_at)
             VALUES (?,?,?,'docker','linux','x64','ready',1,?,?,1,?,?,?,1,1)",
        )
        .bind(BACKEND)
        .bind(HOST)
        .bind(DOMAIN)
        .bind(capabilities.to_string())
        .bind(now)
        .bind(SESSION)
        .bind(BOOT)
        .bind(INCARNATION)
        .execute(database)
        .await?;
        let secret = OneTimeSecret::generate();
        sqlx::query(
            "INSERT INTO host_credentials (id,host_id,host_epoch,generation,verifier,created_at)
             VALUES (?,?,1,1,?,?)",
        )
        .bind(uuid::Uuid::new_v4().to_string())
        .bind(HOST)
        .bind(Verifier::from_secret(secret.expose()).as_str())
        .bind(now)
        .execute(database)
        .await?;
        let target = targets::ensure_target(database, &ACME.target()?).await?;
        sqlx::query(
            "INSERT INTO host_target_grants (host_id,target_id,allow_schedule,allow_docker_socket,revision)
             VALUES (?,?,1,1,1)",
        )
        .bind(HOST)
        .bind(&target)
        .execute(database)
        .await?;
        sqlx::query(
            "INSERT INTO pool_execution_profiles (id,pool_id,name,revision,enabled,runtime_kind,execution_os,architecture,mode,
               requires_docker_socket,all_authorized_hosts,image,labels_json,cpu_millis,memory_mib,disk_bytes,placement,created_at,updated_at)
             VALUES (?,'pool-gl','linux-x64-docker',1,1,'docker','linux','x64','ephemeral',?,1,?,'[]',1000,1024,0,'machines',1,1)",
        )
        .bind(PROFILE)
        .bind(requires_docker_socket)
        .bind(RUNNER_IMAGE)
        .execute(database)
        .await?;
        sqlx::query("INSERT INTO profile_ci_targets (profile_id,target_id,enabled) VALUES (?,?,1)")
            .bind(PROFILE)
            .bind(&target)
            .execute(database)
            .await?;
        let registry = RegistryService::new(database.clone(), setup.app.vault.clone());
        let host = registry
            .authenticate(HOST.parse::<HostId>()?, secret.expose())
            .await?;
        Ok(Self {
            setup,
            registry,
            host,
        })
    }

    /// The machine keeps its heartbeat, inventory, authority and probe
    /// current, as its agent does.
    async fn fresh(&self) -> Result<()> {
        let now = now_millis();
        sqlx::query(
            "UPDATE fleet_hosts SET last_heartbeat_at=?1,last_inventory_at=?1,authority_expires_at=?1+90000 WHERE id=?2",
        )
        .bind(now)
        .bind(HOST)
        .execute(&self.setup.app.database)
        .await?;
        sqlx::query("UPDATE host_backends SET last_probed_at=? WHERE id=?")
            .bind(now)
            .bind(BACKEND)
            .execute(&self.setup.app.database)
            .await?;
        Ok(())
    }

    async fn pass(&self) -> Result<()> {
        self.fresh().await?;
        pass(&self.setup).await
    }

    /// The newest runner row of the pool and its placement.
    async fn newest(&self) -> Result<(String, PlacementId)> {
        let Some((runner, placement)): Option<(String, String)> = sqlx::query_as(
            "SELECT r.id,p.id FROM runners r JOIN workload_placements p ON p.workload_id=r.id
              WHERE r.pool_id='pool-gl' ORDER BY r.created_at DESC,r.rowid DESC LIMIT 1",
        )
        .fetch_optional(&self.setup.app.database)
        .await?
        else {
            let events: Vec<String> = sqlx::query_scalar(
                "SELECT event||': '||message FROM runner_events WHERE pool_id='pool-gl' ORDER BY rowid",
            )
            .fetch_all(&self.setup.app.database)
            .await?;
            bail!("no placed runner in pool-gl; the pool's events: {events:?}");
        };
        Ok((runner, placement.parse()?))
    }

    async fn command(&self, placement: PlacementId, kind: &str) -> Result<AgentCommandEnvelope> {
        let payload: String = sqlx::query_scalar(
            "SELECT payload_json FROM host_commands WHERE placement_id=? AND kind=? ORDER BY cursor DESC LIMIT 1",
        )
        .bind(placement.to_string())
        .bind(kind)
        .fetch_one(&self.setup.app.database)
        .await
        .with_context(|| format!("a {kind} command"))?;
        Ok(serde_json::from_str(&payload)?)
    }

    async fn commands(&self, placement: PlacementId, kind: &str) -> Result<i64> {
        Ok(
            sqlx::query_scalar(
                "SELECT COUNT(*) FROM host_commands WHERE placement_id=? AND kind=?",
            )
            .bind(placement.to_string())
            .bind(kind)
            .fetch_one(&self.setup.app.database)
            .await?,
        )
    }

    /// The agent's answer to the placement's newest `kind` command.
    async fn answer(
        &self,
        placement: PlacementId,
        kind: &str,
        outcome: CommandOutcome,
    ) -> Result<()> {
        let envelope = self.command(placement, kind).await?;
        let result = AgentCommandResult {
            result_id: ResultId::try_new(uuid::Uuid::new_v4())?,
            command_id: envelope.command_id,
            operation_id: envelope.operation_id,
            host_id: envelope.host_id,
            host_epoch: envelope.host_epoch,
            control_plane_incarnation: envelope.control_plane_incarnation,
            authority_session_id: envelope.authority_session_id,
            scope: envelope.scope.clone(),
            generation: match &envelope.scope {
                CommandScope::Placement { generation, .. } => Some(*generation),
                CommandScope::Host => None,
            },
            request_hash: Sha256Digest::try_from(envelope.request_hash.clone())?,
            observed_at: WireTimestamp::from_millis(now_millis())?,
            outcome,
        };
        self.registry
            .record_command_result(&self.host, &result)
            .await?;
        Ok(())
    }

    fn container() -> Result<LocalResourceIdentity> {
        Ok(LocalResourceIdentity::Docker {
            object_id: UpstreamText::new("gitlab-runner-container")?,
        })
    }

    async fn prepared(&self, placement: PlacementId) -> Result<()> {
        self.answer(
            placement,
            "prepare_environment",
            CommandOutcome::Succeeded {
                evidence: Box::new(SuccessEvidence::PreparedEnvironment {
                    prepared_environment_id: PreparedEnvironmentId::try_new(uuid::Uuid::new_v4())?,
                }),
            },
        )
        .await
    }

    async fn started(&self, placement: PlacementId) -> Result<()> {
        let envelope = self.command(placement, "start_runner").await?;
        let AgentOperation::StartRunner(start) = &envelope.operation else {
            bail!("the start command is not a runner start");
        };
        let evidence = SuccessEvidence::RunnerStarted {
            prepared_environment_id: start.prepared_environment_id,
            execution_profile_id: start.execution_profile_id,
            profile_revision: start.profile_revision,
            local_identity: Self::container()?,
        };
        self.answer(
            placement,
            "start_runner",
            CommandOutcome::Succeeded {
                evidence: Box::new(evidence),
            },
        )
        .await
    }

    /// The agent's failure of the placement's `kind` command, with the
    /// published codes.
    async fn failed(
        &self,
        placement: PlacementId,
        kind: &str,
        code: FailureCode,
        diagnostic: Option<DiagnosticCode>,
    ) -> Result<()> {
        let diagnostic = match diagnostic {
            Some(code) => Some(DiagnosticReference {
                id: DiagnosticId::try_new(uuid::Uuid::new_v4())?,
                code,
            }),
            None => None,
        };
        self.answer(
            placement,
            kind,
            CommandOutcome::Failed {
                code,
                retryable: false,
                diagnostic,
            },
        )
        .await
    }

    async fn cleaned(&self, placement: PlacementId) -> Result<()> {
        let resource = Self::container()?;
        let now = WireTimestamp::from_millis(now_millis())?;
        let evidence = || -> Result<DiagnosticReference> {
            Ok(DiagnosticReference {
                id: DiagnosticId::try_new(uuid::Uuid::new_v4())?,
                code: DiagnosticCode::Runtime,
            })
        };
        let cleaned = SuccessEvidence::Cleaned {
            local_identity: resource.clone(),
            local_absence: LocalAbsenceClaim {
                resource: resource.clone(),
                observed_at: now,
                evidence: evidence()?,
            },
            delayed_start_exclusion: DelayedStartExclusionClaim {
                resource,
                excluded_at: now,
                evidence: evidence()?,
            },
        };
        self.answer(
            placement,
            "cleanup",
            CommandOutcome::Succeeded {
                evidence: Box::new(cleaned),
            },
        )
        .await
    }

    /// A pass admits a new runner, its environment is prepared, and the next
    /// pass registers it and asks the machine to start it.
    async fn registered(&self) -> Result<(String, PlacementId)> {
        self.pass().await?;
        let (runner, placement) = self.newest().await?;
        self.prepared(placement).await?;
        self.pass().await?;
        Ok((runner, placement))
    }

    /// Registered, started, and seen online; the pass issued its inspect.
    async fn running(&self) -> Result<(String, PlacementId)> {
        let (runner, placement) = self.registered().await?;
        self.started(placement).await?;
        self.pass().await?;
        Ok((runner, placement))
    }

    async fn row(&self, runner: &str) -> Result<RunnerRow> {
        rows(&self.setup, "pool-gl")
            .await?
            .into_iter()
            .find(|row| row.id == runner)
            .context("the runner's row")
    }

    /// Records jobs of `ACME`'s project in `statuses`, tagged `docker` (one
    /// of the pool's labels), as the poll records them, one job id each.
    async fn gitlab_jobs(&self, statuses: &[&str]) -> Result<()> {
        let now = now_millis();
        let rows = statuses
            .iter()
            .zip(1_i64..)
            .map(|(status, job_id)| gitlab_poll_tests::row(ACME.id, 42, job_id, status, now))
            .collect::<String>();
        sqlx::raw_sql(sqlx::AssertSqlSafe(rows))
            .execute(&self.setup.app.database)
            .await?;
        Ok(())
    }

    /// How many fleet runners the reconciler has placed for `pool-gl`: one
    /// workload intent each.
    async fn placed(&self) -> Result<i64> {
        Ok(
            sqlx::query_scalar("SELECT COUNT(*) FROM workload_intents WHERE pool_id='pool-gl'")
                .fetch_one(&self.setup.app.database)
                .await?,
        )
    }

    async fn finish(self) -> Result<()> {
        self.setup.finish().await
    }
}

/// #181 regression: a fleet GitLab pool at `min_count` 0 with no runners and
/// one pending job it owns places exactly one runner in one pass; the next
/// pass, with that runner starting and covering the job, places no second.
/// Before #181 the pass placed nothing: GitLab jobs never made a fleet
/// platform behind, and the pool already had its minimum of 0 runners.
#[tokio::test]
async fn a_pending_job_places_one_fleet_runner_for_a_pool_at_min_zero() -> Result<()> {
    let fleet = Fleet::autoscaling(3).await?;
    fleet.gitlab_jobs(&["pending"]).await?;

    fleet.pass().await?;
    let desired: i64 =
        sqlx::query_scalar("SELECT desired_count FROM runner_pools WHERE id='pool-gl'")
            .fetch_one(&fleet.setup.app.database)
            .await?;
    assert_eq!(desired, 1, "the scaler counts the pending job as demand");
    assert_eq!(
        fleet.placed().await?,
        1,
        "one pending job places one fleet runner"
    );
    let (runner, _) = fleet.newest().await?;
    assert_eq!(fleet.row(&runner).await?.status, "starting");

    fleet.pass().await?;
    assert_eq!(
        fleet.placed().await?,
        1,
        "the starting runner covers the job, so no second runner is placed"
    );
    fleet.finish().await
}

/// #181 scope 2: with more pending jobs than `max_count`, the pool's fleet
/// runners grow to `max_count` and never past it, however many passes run.
#[tokio::test]
async fn pending_jobs_scale_a_fleet_gitlab_pool_up_to_max_count() -> Result<()> {
    let fleet = Fleet::autoscaling(2).await?;
    fleet
        .gitlab_jobs(&["pending", "pending", "pending"])
        .await?;

    let mut totals = Vec::new();
    for _ in 0..3 {
        fleet.pass().await?;
        totals.push(fleet.placed().await?);
    }

    assert!(
        totals.iter().all(|total| *total <= 2),
        "never more runners than max_count 2, per pass: {totals:?}"
    );
    assert_eq!(
        totals.last().copied(),
        Some(2),
        "three pending jobs bring the pool to max_count, per pass: {totals:?}"
    );
    fleet.finish().await
}

/// #181: only `pending` GitLab jobs are fleet demand. A fleet GitLab pool at
/// `min_count` 0 whose jobs are `created` or `manual` places nothing, even
/// with its desired count still raised from earlier work: `provision` runs
/// (desired 2 is above the 0 active runners), its platform is not behind, so
/// the `NoDemand` guard (`active >= min_count`) stops it.
///
/// The idle timeout is set far beyond the age of any timestamp, so
/// `maybe_scale_down` keeps `desired_count` at 2 through both passes. With
/// the default 5 minutes it would reset desired to `min_count` 0 first and
/// `provision` would never run, leaving the guard untested.
#[tokio::test]
async fn jobs_that_are_not_pending_place_no_fleet_runner_for_a_pool_at_min_zero() -> Result<()> {
    let fleet = Fleet::autoscaling(3).await?;
    fleet.gitlab_jobs(&["created", "manual"]).await?;
    sqlx::query(
        "UPDATE runner_pools SET desired_count=2,idle_timeout_minutes=100000000 WHERE id='pool-gl'",
    )
    .execute(&fleet.setup.app.database)
    .await?;

    fleet.pass().await?;
    fleet.pass().await?;

    let desired: i64 =
        sqlx::query_scalar("SELECT desired_count FROM runner_pools WHERE id='pool-gl'")
            .fetch_one(&fleet.setup.app.database)
            .await?;
    assert_eq!(
        desired, 2,
        "desired stays above min_count, so provision ran and the guard decided"
    );
    assert_eq!(
        fleet.placed().await?,
        0,
        "the platform is not behind, so the NoDemand guard places nothing"
    );
    assert_eq!(gitlab_calls(&fleet.setup, FakeMethod::CreateRunner), []);
    fleet.finish().await
}

/// Done-when 2: on a fleet machine the runner is registered in
/// `register_and_start` (labels, `run_untagged`, marker), the row records its
/// GitLab identity, the sealed bootstrap material is the typed GitLab runner
/// material (the token, the connection's URL, the default idle timeout), and
/// the intent's target is the selected connection's own target. The token is
/// stored nowhere else and never logged.
#[tokio::test]
async fn a_fleet_start_registers_the_runner_and_seals_the_gitlab_material() -> Result<()> {
    const LATER: Connection = Connection {
        id: "gl-later",
        name: "Later",
        base_url: "https://gitlab-two.example.test",
        kind: "group",
        scope_id: 7,
        path: "later",
        created_at: 5,
    };
    let fleet = Fleet::new(true).await?;
    // A second connection, added after: the first is chosen while neither
    // has a live runner.
    LATER.add(&fleet.setup, &["pool-gl"]).await?;
    let (logs, _guard) = Logs::capture();

    let (runner, placement) = fleet.registered().await?;

    let setup = &fleet.setup;
    assert_eq!(gitlab_calls(setup, FakeMethod::CreateRunner), [ACME.url()?]);
    assert_eq!(
        setup.gitlab.runners(&ACME.url()?),
        [FakeRunner {
            id: ExternalId::new(1)?,
            scope: ACME.scope()?,
            tags: vec!["docker".to_owned(), "linux-large".to_owned()],
            run_untagged: true,
            description: marker(&runner),
        }]
    );
    let row = fleet.row(&runner).await?;
    assert_eq!(
        (
            row.ci_platform.as_str(),
            row.gitlab_connection_id.as_deref(),
            row.gitlab_runner_id
        ),
        ("gitlab", Some(ACME.id), Some(1))
    );
    let intent_target: String =
        sqlx::query_scalar("SELECT target_id FROM workload_intents WHERE workload_id=?")
            .bind(&runner)
            .fetch_one(&setup.app.database)
            .await?;
    assert_eq!(
        intent_target,
        targets::ensure_target(&setup.app.database, &ACME.target()?).await?,
        "the selected connection's target"
    );
    let sealed: String =
        sqlx::query_scalar("SELECT sealed FROM fleet_bootstrap_material WHERE placement_id=?")
            .bind(placement.to_string())
            .fetch_one(&setup.app.database)
            .await?;
    let material: GitlabRunnerMaterial = serde_json::from_str(&setup.app.vault.open(&sealed)?)?;
    let GitlabRunnerMaterial::GitlabRunner {
        token,
        url,
        wait_timeout_seconds,
    } = material;
    assert_eq!(token, fake_token(1));
    assert_eq!(Url::parse(&url)?, ACME.url()?);
    assert_eq!(wait_timeout_seconds, GITLAB_DEFAULT_WAIT_TIMEOUT_SECS);
    assert_eq!(fleet.commands(placement, "start_runner").await?, 1);
    assert_no_token_stored(setup).await?;
    logs.assert_token_free();
    fleet.finish().await
}

/// Done-when 5 (a): a GitLab runner runs `gitlab-runner` with the host's
/// Docker socket, so a platform without `requires_docker_socket` is refused
/// in `fleet::provision`, before the intent is submitted: nothing is
/// registered, no prepare is issued, and the reason is readable.
#[tokio::test]
async fn a_fleet_platform_without_the_docker_socket_is_refused_before_registering() -> Result<()> {
    let fleet = Fleet::new(false).await?;

    fleet.pass().await?;

    let setup = &fleet.setup;
    assert_eq!(gitlab_calls(setup, FakeMethod::CreateRunner), []);
    let submitted: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM workload_intents WHERE pool_id='pool-gl'")
            .fetch_one(&setup.app.database)
            .await?;
    assert_eq!(submitted, 0, "no intent, so no prepare");
    let commands: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM host_commands")
        .fetch_one(&setup.app.database)
        .await?;
    assert_eq!(commands, 0);
    assert!(
        rows(setup, "pool-gl")
            .await?
            .iter()
            .all(|row| row.status != "starting"),
        "no runner is left starting"
    );
    assert_readable(setup, "pool-gl", "docker socket").await?;
    fleet.finish().await
}

/// Done-when 5 (b): the machine refusing the prepare (`Configuration`) of a
/// GitLab runner counts one failure for that runner, however many passes
/// see its failed placement.
#[tokio::test]
async fn a_refused_fleet_prepare_counts_once() -> Result<()> {
    let fleet = Fleet::new(true).await?;
    fleet.pass().await?;
    let (runner, placement) = fleet.newest().await?;

    fleet
        .failed(
            placement,
            "prepare_environment",
            FailureCode::Configuration,
            None,
        )
        .await?;
    for _ in 0..3 {
        fleet.pass().await?;
    }

    let setup = &fleet.setup;
    let (failures, open, _, _) = circuit(setup, "pool-gl").await?;
    assert_eq!((failures, open), (1, false));
    assert_ne!(fleet.row(&runner).await?.status, "starting");
    assert_eq!(gitlab_calls(setup, FakeMethod::CreateRunner), []);
    fleet.finish().await
}

/// Done-when 6 and 16 (fleet path, `create_runner` fails, as it does with the
/// unwired client): the placement is abandoned and its capacity released
/// before anything starts, the row fails, and the failure counts once.
#[tokio::test]
async fn a_failed_fleet_registration_abandons_the_placement_and_counts_once() -> Result<()> {
    let fleet = Fleet::new(true).await?;
    let setup = &fleet.setup;
    setup.gitlab.fail_next(
        &ACME.url()?,
        FakeMethod::CreateRunner,
        GitLabError::Upstream(PLACEHOLDER_ERROR.to_owned()),
    );

    let (runner, placement) = fleet.registered().await?;
    for _ in 0..2 {
        fleet.pass().await?;
    }

    assert_eq!(gitlab_runner_ids(setup, &ACME)?, Vec::<i64>::new());
    assert_eq!(fleet.commands(placement, "start_runner").await?, 0);
    let (state, held): (String, i64) = sqlx::query_as(
        "SELECT p.state,(SELECT COUNT(*) FROM capacity_allocations a WHERE a.placement_id=p.id AND a.state!='released')
           FROM workload_placements p WHERE p.id=?",
    )
    .bind(placement.to_string())
    .fetch_one(&setup.app.database)
    .await?;
    assert_eq!(
        (state.as_str(), held),
        ("failed", 0),
        "abandoned and released"
    );
    assert_ne!(fleet.row(&runner).await?.status, "starting");
    let (failures, open, _, _) = circuit(setup, "pool-gl").await?;
    assert_eq!((failures, open), (1, false), "counted once");
    fleet.finish().await
}

/// Done-when 6 (fleet path, the bootstrap store fails after `create_runner`)
/// and round-4 W3: the GitLab runner is deleted once and forgotten on the
/// row, the placement abandoned (no start was issued), the failure counted
/// once; once the pause is over the next pass starts again.
#[tokio::test]
async fn a_failed_bootstrap_store_deletes_the_gitlab_runner_once_and_the_next_start_works()
-> Result<()> {
    let fleet = Fleet::new(true).await?;
    let setup = &fleet.setup;
    sqlx::raw_sql(
        "CREATE TRIGGER fail_bootstrap BEFORE INSERT ON fleet_bootstrap_material
         BEGIN SELECT RAISE(ABORT,'disk I/O error'); END;",
    )
    .execute(&setup.app.database)
    .await?;

    let (runner, placement) = fleet.registered().await?;
    for _ in 0..2 {
        fleet.pass().await?;
    }

    assert_eq!(
        gitlab_runner_ids(setup, &ACME)?,
        Vec::<i64>::new(),
        "deleted"
    );
    assert_eq!(
        gitlab_calls(setup, FakeMethod::DeleteRunner).len(),
        1,
        "deleted once"
    );
    assert_eq!(fleet.row(&runner).await?.gitlab_runner_id, None);
    assert_eq!(fleet.commands(placement, "start_runner").await?, 0);
    let held: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM capacity_allocations WHERE placement_id=? AND state!='released'",
    )
    .bind(placement.to_string())
    .fetch_one(&setup.app.database)
    .await?;
    assert_eq!(held, 0, "the placement's capacity is released");
    let (failures, _, _, _) = circuit(setup, "pool-gl").await?;
    assert_eq!(failures, 1, "counted once");

    sqlx::raw_sql("DROP TRIGGER fail_bootstrap")
        .execute(&setup.app.database)
        .await?;
    end_backoff(setup, "pool-gl").await?;
    let (next, next_placement) = fleet.registered().await?;
    assert_ne!(next, runner);
    assert_eq!(gitlab_runner_ids(setup, &ACME)?, [2]);
    assert_eq!(fleet.commands(next_placement, "start_runner").await?, 1);
    fleet.finish().await
}

/// #171 (fleet path, `fleet_rollback`): when the start cannot be handed to the
/// machine and GitLab refuses to delete the runner just registered, the
/// refusal is a warning on the pool, linked to the runner row.
#[tokio::test]
async fn a_refused_delete_in_a_fleet_rollback_is_a_pool_warning() -> Result<()> {
    let fleet = Fleet::new(true).await?;
    let setup = &fleet.setup;
    sqlx::raw_sql(
        "CREATE TRIGGER fail_bootstrap BEFORE INSERT ON fleet_bootstrap_material
         BEGIN SELECT RAISE(ABORT,'disk I/O error'); END;",
    )
    .execute(&setup.app.database)
    .await?;
    setup.gitlab.fail_next(
        &ACME.url()?,
        FakeMethod::DeleteRunner,
        missing_manage_runner(),
    );

    let (runner, _) = fleet.registered().await?;

    // The rollback's delete is the first; it keeps the registration on the
    // row, so the row leaving service deletes it again.
    assert!(
        !gitlab_calls(setup, FakeMethod::DeleteRunner).is_empty(),
        "the rollback asked GitLab to delete the runner"
    );
    let refused = refusals(setup, "pool-gl").await?;
    assert_eq!(refused.len(), 1, "{refused:?}");
    assert_refusal(&refused[0], ACME.name, 1, "manage_runner");
    assert_eq!(refused[0].runner_id.as_deref(), Some(runner.as_str()));
    fleet.finish().await
}

/// #171 (fleet path, `fleet_material`): a runner row still holding an earlier
/// attempt's GitLab runner deletes it before registering again; GitLab
/// refusing that delete is a warning on the pool naming the earlier runner,
/// and the start goes on.
#[tokio::test]
async fn a_refused_delete_of_an_earlier_attempts_runner_is_a_pool_warning() -> Result<()> {
    const EARLIER: i64 = 77;
    let fleet = Fleet::new(true).await?;
    let setup = &fleet.setup;
    fleet.pass().await?;
    let (runner, placement) = fleet.newest().await?;
    let admitted = fleet.row(&runner).await?;
    assert_eq!(
        (
            admitted.gitlab_connection_id.as_deref(),
            admitted.gitlab_runner_id
        ),
        (Some(ACME.id), None),
        "admitted on the connection, not registered yet"
    );
    // An earlier attempt registered a GitLab runner and stopped there.
    seed(setup, &ACME, u64::try_from(EARLIER)?, &marker(&runner))?;
    sqlx::query("UPDATE runners SET gitlab_runner_id=? WHERE id=?")
        .bind(EARLIER)
        .bind(&runner)
        .execute(&setup.app.database)
        .await?;
    setup.gitlab.fail_next(
        &ACME.url()?,
        FakeMethod::DeleteRunner,
        missing_manage_runner(),
    );

    fleet.prepared(placement).await?;
    fleet.pass().await?;

    assert!(
        gitlab_runner_ids(setup, &ACME)?.contains(&EARLIER),
        "GitLab refused the delete"
    );
    assert_eq!(
        fleet.commands(placement, "start_runner").await?,
        1,
        "the start goes on"
    );
    let refused = refusals(setup, "pool-gl").await?;
    assert_eq!(refused.len(), 1, "{refused:?}");
    assert_refusal(&refused[0], ACME.name, EARLIER, "manage_runner");
    assert_eq!(refused[0].runner_id.as_deref(), Some(runner.as_str()));
    fleet.finish().await
}

/// Done-when 6 (fleet path, a start command exists and the machine fails the
/// start) and 12: the failed runner counts once and its GitLab runner is
/// deregistered; the machine freed its capacity; the start is retried as a
/// new runner on the same connection (`gitlab_connection_id` copied,
/// `gitlab_runner_id` not), which registers a GitLab runner of its own.
#[tokio::test]
async fn a_failed_fleet_start_is_retried_as_a_new_gitlab_runner_on_the_same_connection()
-> Result<()> {
    let fleet = Fleet::new(true).await?;
    let setup = &fleet.setup;
    let (first, first_placement) = fleet.registered().await?;
    assert_eq!(fleet.row(&first).await?.gitlab_runner_id, Some(1));

    fleet
        .failed(
            first_placement,
            "start_runner",
            FailureCode::Dependency,
            None,
        )
        .await?;
    fleet.pass().await?;

    assert_ne!(fleet.row(&first).await?.status, "starting");
    assert!(
        !gitlab_runner_ids(setup, &ACME)?.contains(&1),
        "the failed runner is deregistered"
    );
    let held: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM capacity_allocations WHERE placement_id=? AND state!='released'",
    )
    .bind(first_placement.to_string())
    .fetch_one(&setup.app.database)
    .await?;
    assert_eq!(held, 0, "no capacity is held by the failed start");
    let (failures, _, _, _) = circuit(setup, "pool-gl").await?;
    assert_eq!(failures, 1, "counted once");
    let (retry, retry_placement) = fleet.newest().await?;
    assert_ne!(retry, first, "retried as a new runner");
    let retried = fleet.row(&retry).await?;
    assert_eq!(
        (
            retried.ci_platform.as_str(),
            retried.gitlab_connection_id.as_deref(),
            retried.gitlab_runner_id
        ),
        ("gitlab", Some(ACME.id), None),
        "the connection is copied, the GitLab runner is not"
    );

    fleet.prepared(retry_placement).await?;
    fleet.pass().await?;
    assert_eq!(fleet.row(&retry).await?.gitlab_runner_id, Some(2));
    assert_eq!(gitlab_runner_ids(setup, &ACME)?, [2]);
    let (failures, _, _, _) = circuit(setup, "pool-gl").await?;
    assert_eq!(failures, 1, "still one failure");
    fleet.finish().await
}

/// Done-when 7 and 10 (fleet path): a GitLab runner that completed (Inspect
/// `ResourceUnavailable`, no diagnostic) or idled out (`IdleExpired`) is
/// removed without error and deregistered, resets the pool's failure count,
/// and is replaced.
#[tokio::test]
async fn an_exited_fleet_gitlab_runner_is_deregistered_resets_the_count_and_is_replaced()
-> Result<()> {
    for diagnostic in [None, Some(DiagnosticCode::IdleExpired)] {
        let fleet = Fleet::new(true).await?;
        let setup = &fleet.setup;
        let (runner, placement) = fleet.running().await?;
        assert_eq!(fleet.row(&runner).await?.status, "online", "{diagnostic:?}");
        sqlx::query("UPDATE runner_pools SET provision_failure_count=2 WHERE id='pool-gl'")
            .execute(&setup.app.database)
            .await?;

        fleet
            .failed(
                placement,
                "inspect",
                FailureCode::ResourceUnavailable,
                diagnostic,
            )
            .await?;
        fleet.pass().await?;

        let row = fleet.row(&runner).await?;
        assert!(
            row.deleted || matches!(row.status.as_str(), "deleted" | "failed"),
            "{diagnostic:?}: out of service: {row:?}"
        );
        assert_eq!(
            gitlab_runner_ids(setup, &ACME)?,
            Vec::<i64>::new(),
            "{diagnostic:?}: deregistered"
        );
        assert_eq!(
            fleet.commands(placement, "cleanup").await?,
            1,
            "{diagnostic:?}"
        );
        let (failures, open, _, _) = circuit(setup, "pool-gl").await?;
        assert_eq!((failures, open), (0, false), "{diagnostic:?}: reset");
        let (replacement, _) = fleet.newest().await?;
        assert_ne!(replacement, runner, "{diagnostic:?}: replaced");
        let errors: Vec<String> = sqlx::query_scalar(
            "SELECT event FROM runner_events WHERE pool_id='pool-gl' AND level='error'",
        )
        .fetch_all(&setup.app.database)
        .await?;
        assert_eq!(
            errors,
            Vec::<String>::new(),
            "{diagnostic:?}: without error"
        );
        fleet.finish().await?;
    }
    Ok(())
}

/// Done-when 7 (drain, fleet path) and 8: lowering the desired count removes
/// the idle fleet runner through the GitLab deregistration, and the next
/// pass cleans its runtime up.
#[tokio::test]
async fn a_drained_fleet_gitlab_runner_is_deregistered_and_cleaned_up() -> Result<()> {
    let fleet = Fleet::new(true).await?;
    let setup = &fleet.setup;
    let (runner, placement) = fleet.running().await?;
    sqlx::query("UPDATE runner_pools SET desired_count=0,min_count=0 WHERE id='pool-gl'")
        .execute(&setup.app.database)
        .await?;

    fleet.pass().await?;
    assert!(fleet.row(&runner).await?.deleted, "drained");
    assert_eq!(gitlab_calls(setup, FakeMethod::DeleteRunner), [ACME.url()?]);
    assert_eq!(gitlab_runner_ids(setup, &ACME)?, Vec::<i64>::new());
    assert_eq!(
        setup
            .github_removals
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .len(),
        0
    );

    fleet.pass().await?;
    assert_eq!(fleet.commands(placement, "cleanup").await?, 1, "cleaned up");
    fleet.finish().await
}

/// Done-when 9 and 10 (fleet path, Start-time refusal): GitLab refusing the
/// token fails the runner with a readable diagnostic, deregisters it, and
/// counts one failure, however many passes revisit its failed placement.
#[tokio::test]
async fn a_start_time_registration_refusal_counts_once_with_a_readable_diagnostic() -> Result<()> {
    let fleet = Fleet::new(true).await?;
    let setup = &fleet.setup;
    let (runner, placement) = fleet.registered().await?;

    fleet
        .failed(
            placement,
            "start_runner",
            FailureCode::Configuration,
            Some(DiagnosticCode::RegistrationRefused),
        )
        .await?;
    for _ in 0..3 {
        fleet.pass().await?;
    }

    let (failures, open, _, _) = circuit(setup, "pool-gl").await?;
    assert_eq!((failures, open), (1, false), "counted once");
    assert_ne!(fleet.row(&runner).await?.status, "starting");
    assert_eq!(
        gitlab_runner_ids(setup, &ACME)?,
        Vec::<i64>::new(),
        "deregistered"
    );
    assert_readable(setup, "pool-gl", "registration_refused").await?;
    fleet.finish().await
}

/// Done-when 9 and 10 (fleet path, late refusal): GitLab refusing the token
/// after the start succeeded (the runner exits 77; Inspect carries
/// `RegistrationRefused`) fails the runner, counted once, like a Start-time
/// refusal.
#[tokio::test]
async fn a_late_inspect_registration_refusal_counts_once_with_a_readable_diagnostic() -> Result<()>
{
    let fleet = Fleet::new(true).await?;
    let setup = &fleet.setup;
    let (runner, placement) = fleet.running().await?;

    fleet
        .failed(
            placement,
            "inspect",
            FailureCode::ResourceUnavailable,
            Some(DiagnosticCode::RegistrationRefused),
        )
        .await?;
    for _ in 0..3 {
        fleet.pass().await?;
    }

    let (failures, open, _, _) = circuit(setup, "pool-gl").await?;
    assert_eq!((failures, open), (1, false), "counted once");
    let row = fleet.row(&runner).await?;
    assert!(
        row.deleted || matches!(row.status.as_str(), "deleted" | "failed"),
        "{row:?}"
    );
    assert_eq!(
        gitlab_runner_ids(setup, &ACME)?,
        Vec::<i64>::new(),
        "deregistered"
    );
    assert_readable(setup, "pool-gl", "registration_refused").await?;
    fleet.finish().await
}

/// Done-when 10 (fleet path): three distinct refused runners open the pool's
/// circuit, and passes that revisit their failed placements count nothing
/// more.
#[tokio::test]
async fn three_refused_fleet_gitlab_runners_open_the_circuit() -> Result<()> {
    let fleet = Fleet::new(true).await?;
    let setup = &fleet.setup;
    for attempt in 1..=3 {
        let (_, placement) = fleet.registered().await?;
        fleet
            .failed(
                placement,
                "start_runner",
                FailureCode::Configuration,
                Some(DiagnosticCode::RegistrationRefused),
            )
            .await?;
        fleet.pass().await?;
        let (failures, _, _, _) = circuit(setup, "pool-gl").await?;
        assert_eq!(failures, attempt, "attempt {attempt}");
        if attempt < 3 {
            end_backoff(setup, "pool-gl").await?;
        }
    }
    for _ in 0..2 {
        fleet.pass().await?;
    }

    let (failures, open, _, state) = circuit(setup, "pool-gl").await?;
    assert_eq!((failures, open, state.as_str()), (3, true, "blocked"));
    fleet.finish().await
}

/// Done-when 10, round-3 block 1 (fleet path): a completed placement is
/// revisited on every pass (by `follow` until its cleanup lands, then by
/// `finish`); it resets the count once, when its runner leaves service, and
/// must not reset a count later refusals raised.
#[tokio::test]
async fn an_old_completed_fleet_placement_does_not_reset_a_count_raised_after_it() -> Result<()> {
    let fleet = Fleet::new(true).await?;
    let setup = &fleet.setup;
    let (_, placement) = fleet.running().await?;
    sqlx::query("UPDATE runner_pools SET provision_failure_count=1 WHERE id='pool-gl'")
        .execute(&setup.app.database)
        .await?;
    fleet
        .failed(placement, "inspect", FailureCode::ResourceUnavailable, None)
        .await?;
    fleet.pass().await?;
    assert_eq!(
        circuit(setup, "pool-gl").await?.0,
        0,
        "the completed runner reset the count when it left service"
    );
    // Refusals of later runners raised the count; the pool pauses.
    sqlx::query(
        "UPDATE runner_pools SET provision_failure_count=2,provision_retry_at=? WHERE id='pool-gl'",
    )
    .bind(now_millis() + 600_000)
    .execute(&setup.app.database)
    .await?;

    fleet.pass().await?;
    fleet.cleaned(placement).await?;
    for _ in 0..2 {
        fleet.pass().await?;
    }

    let (failures, _, _, _) = circuit(setup, "pool-gl").await?;
    assert_eq!(
        failures, 2,
        "the old completed placement reset nothing again"
    );
    fleet.finish().await
}

/// Round-4 W2 (fleet path): the half-open probe of an open circuit is one
/// start, whatever the pool wants, and it is a GitLab start: the next pass
/// registers its GitLab runner.
#[tokio::test]
async fn the_fleet_probe_of_an_open_circuit_is_one_start() -> Result<()> {
    let fleet = Fleet::new(true).await?;
    let setup = &fleet.setup;
    sqlx::query(
        "UPDATE runner_pools SET desired_count=3,min_count=3,provision_failure_count=3,provision_circuit_open=1,provision_retry_at=1,state='blocked' WHERE id='pool-gl'",
    )
    .execute(&setup.app.database)
    .await?;

    fleet.pass().await?;

    assert_eq!(rows(setup, "pool-gl").await?.len(), 1, "one probe start");
    let (probe, placement) = fleet.newest().await?;
    fleet.prepared(placement).await?;
    fleet.pass().await?;
    assert_eq!(fleet.row(&probe).await?.gitlab_runner_id, Some(1));
    assert_eq!(gitlab_runner_ids(setup, &ACME)?, [1]);
    fleet.finish().await
}
