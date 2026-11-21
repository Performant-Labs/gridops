//! #138: the GitLab platform-connection routes, GitLab runner pools and the
//! GitLab arm of runner deletion, driven through the real routers with
//! `FakeGitLab` injected through `AppState::with_gitlab`.
//!
//! Every test builds its own temporary database with the migrations applied,
//! signs in an administrator (`admin-1`) and a member (`member-1`, who
//! administers installation 42 but is not a system administrator), and sends
//! requests through `crate::session_routes()` and `crate::token_routes()` with
//! `tower::ServiceExt::oneshot`.
//!
//! `FakeGitLab` keys its state by base URL and never sees the token (see its
//! docs), so "the right credentials" is checked as "the call went to the
//! connection's GitLab instance".

use std::{path::PathBuf, sync::Arc};

use anyhow::{Context as _, Result, anyhow, ensure};
use axum::{
    Router,
    body::Body,
    http::{Method, Request, StatusCode, header},
};
use gridops_core::{
    BitbucketClient, Config, GitHubClient, Vault,
    fleet::{CiTarget, ExternalId, GitlabConnectionId},
    gitlab::{
        GitLabApi as _, GitLabCredentials, GitLabError, Scope,
        fake::{FakeGitLab, FakeMethod},
    },
};
use secrecy::SecretString;
use serde_json::{Value, json};
use sqlx::SqlitePool;
use tower::ServiceExt as _;
use url::Url;

use crate::state::AppState;

#[path = "../../tests/support/openapi_schema.rs"]
mod openapi_schema;

const PUBLIC: &str = "https://gridops.example";
const GITLAB: &str = "https://gitlab.example.com";
const OTHER_GITLAB: &str = "https://gitlab.other.example";
const WEBHOOK_URL: &str = "https://gridops.example/api/v1/webhooks/gitlab";

const CONNECTIONS: &str = "/api/v1/platform-connections/gitlab";
const CONNECTION: &str = "/api/v1/platform-connections/gitlab/{connection_id}";
const VERIFY: &str = "/api/v1/platform-connections/gitlab/{connection_id}/verify";
const ROTATE: &str = "/api/v1/platform-connections/gitlab/{connection_id}/webhook-secret/rotate";

const TOKEN: &str = "glpat-first-token-0123456789abcdef";
const REPLACEMENT_TOKEN: &str = "glpat-second-token-0123456789abcdef";

const CONNECTION_1: &str = "a1000000-0000-4000-8000-000000000001";
const CONNECTION_2: &str = "a1000000-0000-4000-8000-000000000002";
const UNKNOWN_CONNECTION: &str = "a1000000-0000-4000-8000-0000000000ff";
const BITBUCKET_CONNECTION: &str = "b1000000-0000-4000-8000-000000000001";
const GITLAB_POOL: &str = "c1000000-0000-4000-8000-000000000001";
const ORG_POOL: &str = "c1000000-0000-4000-8000-000000000002";

struct Harness {
    router: Router,
    vault: Vault,
    gitlab: FakeGitLab,
    database: SqlitePool,
    path: PathBuf,
    admin: String,
    member: String,
}

impl Drop for Harness {
    fn drop(&mut self) {
        for suffix in ["", "-shm", "-wal"] {
            let _ = std::fs::remove_file(format!("{}{suffix}", self.path.display()));
        }
    }
}

fn url(value: &str) -> Result<Url> {
    Url::parse(value).with_context(|| format!("{value} is not a URL"))
}

fn external_id(value: u64) -> Result<ExternalId> {
    ExternalId::new(value).map_err(|error| anyhow!("{error}"))
}

fn project(scope_id: u64, path: &str) -> Result<Scope> {
    Ok(Scope::Project {
        id: external_id(scope_id)?,
        path: path.to_owned(),
    })
}

fn group(scope_id: u64, path: &str) -> Result<Scope> {
    Ok(Scope::Group {
        id: external_id(scope_id)?,
        path: path.to_owned(),
    })
}

fn path(template: &str, connection_id: &str) -> String {
    template.replace("{connection_id}", connection_id)
}

async fn harness() -> Result<Harness> {
    let path = std::env::temp_dir().join(format!("gridops-gitlab-{}.sqlite", uuid::Uuid::new_v4()));
    let database = gridops_core::connect_database_path(&path).await?;
    // The manager is unreachable: a test that reaches it fails fast.
    let config = Config::from_env()?
        .with_base_url(url(PUBLIC)?)
        .with_manager(url("http://127.0.0.1:9")?, "test-manager-credential");
    let vault = Vault::new([7_u8; 32], b"gitlab-test-session-secret".to_vec());
    let github = GitHubClient::new(config.clone())?;
    let bitbucket = BitbucketClient::new()?;
    let gitlab = FakeGitLab::default();
    let state = AppState::new(config, database.clone(), vault.clone(), github, bitbucket)?
        .with_gitlab(Arc::new(gitlab.clone()));
    let router = crate::session_routes()
        .merge(crate::token_routes())
        .with_state(state);
    let now = gridops_core::now_millis();
    sqlx::query("INSERT INTO installations (id,account_id,account_login,account_type,target_type,repository_selection,created_at,updated_at) VALUES (42,42,'fixture','Organization','Organization','selected',?,?)")
        .bind(now).bind(now).execute(&database).await?;
    // Pools start runners only while the built-in host is on, so D14 is
    // observable: a pool the request starts itself would call the manager.
    sqlx::query("INSERT INTO settings (key,value,updated_at) VALUES (?, 'true', ?) ON CONFLICT(key) DO UPDATE SET value='true'")
        .bind(gridops_core::builtin_host::SETTING_KEY)
        .bind(now)
        .execute(&database)
        .await?;
    let mut harness = Harness {
        router,
        vault,
        gitlab,
        database,
        path,
        admin: String::new(),
        member: String::new(),
    };
    harness.admin = harness.sign_in("admin-1", 1, "admin").await?;
    harness.member = harness.sign_in("member-1", 2, "member").await?;
    Ok(harness)
}

impl Harness {
    /// Seeds a user who administers installation 42, with a session; returns
    /// the session cookie.
    async fn sign_in(&self, user_id: &str, github_id: i64, role: &str) -> Result<String> {
        let now = gridops_core::now_millis();
        sqlx::query("INSERT INTO users (id,github_id,login,role,access_token,last_login_at,created_at,updated_at) VALUES (?,?,?,?,'',?,?,?)")
            .bind(user_id).bind(github_id).bind(user_id).bind(role).bind(now).bind(now).bind(now)
            .execute(&self.database).await?;
        sqlx::query("INSERT INTO user_installations (user_id,installation_id,permission,created_at) VALUES (?,42,'admin',?)")
            .bind(user_id).bind(now).execute(&self.database).await?;
        let token = format!("session-{user_id}");
        sqlx::query("INSERT INTO sessions (id,token_hash,user_id,expires_at,last_seen_at,created_at) VALUES (?,?,?,?,?,?)")
            .bind(format!("session-row-{user_id}"))
            .bind(gridops_core::crypto::hash_token(&token))
            .bind(user_id)
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

    /// Sends a same-origin request; returns the status and the JSON body
    /// (`Value::Null` for an empty body, a string for a non-JSON one).
    async fn send(
        &self,
        method: Method,
        uri: &str,
        cookie: Option<&str>,
        body: Option<Value>,
    ) -> Result<(StatusCode, Value)> {
        let mut request = Request::builder()
            .method(method)
            .uri(uri)
            .header(header::ORIGIN, PUBLIC);
        if let Some(cookie) = cookie {
            request = request.header(header::COOKIE, cookie);
        }
        let request = match body {
            Some(body) => request
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(body.to_string()))?,
            None => request.body(Body::empty())?,
        };
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

    async fn as_admin(
        &self,
        method: Method,
        uri: &str,
        body: Option<Value>,
    ) -> Result<(StatusCode, Value)> {
        self.send(method, uri, Some(self.admin.as_str()), body)
            .await
    }

    /// Creates a connection through the API and returns the 201 body.
    async fn create(&self, base_url: &str, scope: Scope, name: &str) -> Result<Value> {
        self.gitlab.with_scope(&url(base_url)?, scope.clone());
        let (status, body) = self
            .as_admin(
                Method::POST,
                CONNECTIONS,
                Some(json!({
                    "name": name,
                    "baseUrl": base_url,
                    "scopeKind": scope.kind().as_str(),
                    "scopePath": scope.path(),
                    "accessToken": TOKEN,
                })),
            )
            .await?;
        ensure!(
            status == StatusCode::CREATED,
            "create {name}: {status} {body}"
        );
        Ok(body)
    }

    /// Seeds a connection row and its two sealed secrets directly, so pool
    /// and runner tests do not depend on the connection handlers.
    async fn seed_connection(
        &self,
        connection_id: &str,
        base_url: &str,
        scope: Scope,
    ) -> Result<()> {
        let now = gridops_core::now_millis();
        let token_key = format!("test.gitlab.{connection_id}.token");
        let secret_key = format!("test.gitlab.{connection_id}.webhook");
        for (key, value) in [(&token_key, TOKEN), (&secret_key, "seeded-webhook-secret")] {
            sqlx::query(
                "INSERT INTO runtime_secrets (key,value,updated_by,updated_at) VALUES (?,?,NULL,?)",
            )
            .bind(key)
            .bind(self.vault.seal(value)?)
            .bind(now)
            .execute(&self.database)
            .await?;
        }
        sqlx::query("INSERT INTO gitlab_connections (id,name,base_url,scope_kind,scope_id,scope_path,access_token_key,webhook_secret_key,created_by,created_at,updated_at) VALUES (?,?,?,?,?,?,?,?,NULL,?,?)")
            .bind(connection_id)
            .bind(format!("seeded {connection_id}"))
            .bind(base_url)
            .bind(scope.kind().as_str())
            .bind(i64::try_from(scope.id().get()).unwrap_or(i64::MAX))
            .bind(scope.path())
            .bind(&token_key)
            .bind(&secret_key)
            .bind(now)
            .bind(now)
            .execute(&self.database)
            .await?;
        self.gitlab.with_scope(&url(base_url)?, scope);
        Ok(())
    }

    /// Seeds a Docker pool of installation 42 with `scope`, linked to
    /// `connections`, wanting one runner.
    async fn seed_pool(
        &self,
        pool_id: &str,
        name: &str,
        scope: &str,
        connections: &[&str],
    ) -> Result<()> {
        sqlx::query("INSERT INTO runner_pools (id,installation_id,name,scope,provider,providers,labels,image,docker_image,tart_image,autoscaling_enabled,desired_count,max_count,runner_group_id,run_untagged,created_at,updated_at) VALUES (?,42,?,?,'docker','[\"docker\"]','[]','runner:latest','runner:latest','',1,1,4,1,0,1,1)")
            .bind(pool_id).bind(name).bind(scope)
            .execute(&self.database).await?;
        for connection in connections {
            sqlx::query("INSERT INTO runner_pool_gitlab_connections (pool_id,connection_id,created_at) VALUES (?,?,1)")
                .bind(pool_id).bind(connection)
                .execute(&self.database).await?;
        }
        Ok(())
    }

    /// Seeds a live GitLab runner of `pool_id` (no container, so deleting it
    /// never reaches the manager).
    async fn seed_runner(
        &self,
        runner_id: &str,
        pool_id: &str,
        connection_id: Option<&str>,
        gitlab_runner_id: Option<i64>,
    ) -> Result<()> {
        sqlx::query("INSERT INTO runners (id,pool_id,name,provider,status,busy,ci_platform,gitlab_connection_id,gitlab_runner_id,created_at,updated_at) VALUES (?,?,?,'docker','idle',0,'gitlab',?,?,1,1)")
            .bind(runner_id).bind(pool_id).bind(runner_id).bind(connection_id).bind(gitlab_runner_id)
            .execute(&self.database).await?;
        Ok(())
    }

    /// Registers a runner on the fake GitLab, as #139 would have.
    async fn gitlab_runner(&self, base_url: &str, scope: &Scope) -> Result<i64> {
        let created = self
            .gitlab
            .create_runner(
                &GitLabCredentials {
                    base_url: url(base_url)?,
                    access_token: SecretString::from(TOKEN),
                },
                scope,
                &[],
                false,
                "fixture runner",
            )
            .await
            .map_err(|error| anyhow!("{error}"))?;
        Ok(i64::try_from(created.id.get())?)
    }

    fn calls(&self, method: FakeMethod) -> Vec<Url> {
        self.gitlab
            .calls()
            .into_iter()
            .filter(|call| call.method == method)
            .map(|call| call.base_url)
            .collect()
    }

    async fn count(&self, sql: &str, bind: &str) -> Result<i64> {
        Ok(sqlx::query_scalar(sqlx::AssertSqlSafe(sql.to_owned()))
            .bind(bind)
            .fetch_one(&self.database)
            .await?)
    }

    async fn secret(&self, key: &str) -> Result<Option<String>> {
        let sealed: Option<String> =
            sqlx::query_scalar("SELECT value FROM runtime_secrets WHERE key=?")
                .bind(key)
                .fetch_optional(&self.database)
                .await?;
        sealed.map(|value| self.vault.open(&value)).transpose()
    }

    async fn secret_keys(&self, connection_id: &str) -> Result<(String, String)> {
        Ok(sqlx::query_as(
            "SELECT access_token_key,webhook_secret_key FROM gitlab_connections WHERE id=?",
        )
        .bind(connection_id)
        .fetch_one(&self.database)
        .await?)
    }

    async fn audit_count(&self, target_id: &str) -> Result<i64> {
        self.count(
            "SELECT COUNT(*) FROM audit_events WHERE target_id=?",
            target_id,
        )
        .await
    }
}

fn connection_id(created: &Value) -> Result<String> {
    created["connection"]["id"]
        .as_str()
        .map(ToOwned::to_owned)
        .context("no connection.id")
}

fn error_text(body: &Value) -> String {
    body["error"].as_str().unwrap_or_default().to_lowercase()
}

/// A valid GitLab pool create body for installation 42.
fn gitlab_pool_body(name: &str, connections: &[&str]) -> Value {
    json!({
        "installationId": 42,
        "name": name,
        "scope": "gitlab",
        "gitlabConnectionIds": connections,
        "mode": "ephemeral",
        "provider": "docker",
        "providers": ["docker"],
        "labels": [],
        "image": "ghcr.io/actions/actions-runner:latest",
        "dockerImage": "ghcr.io/actions/actions-runner:latest",
        "desiredCount": 1,
        "minCount": 0,
        "maxCount": 4,
        "autoscalingEnabled": true,
        "queueScaleFactor": 1,
        "idleTimeoutMinutes": 5,
        "cpuLimit": 2.0,
        "memoryLimitMb": 2048,
        // Validation still wants a positive value; the handler stores 1.
        "runnerGroupId": 5,
    })
}

/// A pool update body; `extra` is merged in (e.g. `gitlabConnectionIds`).
fn pool_update_body(name: &str, extra: &Value) -> Value {
    let mut body = json!({
        "name": name,
        "mode": "ephemeral",
        "provider": "docker",
        "providers": ["docker"],
        "labels": [],
        "image": "ghcr.io/actions/actions-runner:latest",
        "dockerImage": "ghcr.io/actions/actions-runner:latest",
        "desiredCount": 1,
        "minCount": 0,
        "maxCount": 4,
        "autoscalingEnabled": true,
        "queueScaleFactor": 1,
        "idleTimeoutMinutes": 5,
        "cpuLimit": 2.0,
        "memoryLimitMb": 2048,
        "runnerGroupId": 9,
    });
    for (key, value) in extra.as_object().into_iter().flatten() {
        body[key] = value.clone();
    }
    body
}

// ---------------------------------------------------------------------------
// Connections: create
// ---------------------------------------------------------------------------

#[tokio::test]
async fn creating_a_connection_resolves_the_scope_stores_both_secrets_and_shows_the_secret_once()
-> Result<()> {
    let h = harness().await?;
    h.gitlab.with_scope(&url(GITLAB)?, project(42, "acme/app")?);
    let (status, created) = h
        .as_admin(
            Method::POST,
            CONNECTIONS,
            Some(json!({
                "name": "Acme app",
                "baseUrl": "https://gitlab.example.com/",
                "scopeKind": "project",
                "scopePath": "acme/app",
                "accessToken": TOKEN,
            })),
        )
        .await?;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    openapi_schema::check_response(CONNECTIONS, "post", 201, &created)?;
    let connection = &created["connection"];
    assert_eq!(connection["provider"], "gitlab");
    assert_eq!(connection["name"], "Acme app");
    assert_eq!(
        connection["baseUrl"], GITLAB,
        "normalised: no trailing slash"
    );
    assert_eq!(connection["scopeKind"], "project");
    assert_eq!(connection["scopeId"], 42);
    assert_eq!(connection["scopePath"], "acme/app");
    assert_eq!(connection["canManage"], true);
    assert_eq!(created["webhookUrl"], WEBHOOK_URL);
    let webhook_secret = created["webhookSecret"].as_str().context("webhookSecret")?;
    assert!(
        webhook_secret.len() >= 32,
        "a random secret: {webhook_secret:?}"
    );

    // GitLab was asked to resolve the path, then to check the token's access.
    assert_eq!(
        h.gitlab
            .calls()
            .iter()
            .map(|call| call.method)
            .collect::<Vec<_>>(),
        [FakeMethod::ResolveScope, FakeMethod::VerifyAccess]
    );
    let base = url(GITLAB)?;
    assert!(h.gitlab.calls().iter().all(|call| call.base_url == base));

    // Stored by key, sealed: the token and the secret just shown.
    let id = connection_id(&created)?;
    let (base_url, scope_kind, scope_id, scope_path): (String, String, i64, String) =
        sqlx::query_as(
            "SELECT base_url,scope_kind,scope_id,scope_path FROM gitlab_connections WHERE id=?",
        )
        .bind(&id)
        .fetch_one(&h.database)
        .await?;
    assert_eq!(
        (
            base_url.as_str(),
            scope_kind.as_str(),
            scope_id,
            scope_path.as_str()
        ),
        (GITLAB, "project", 42, "acme/app")
    );
    let (token_key, secret_key) = h.secret_keys(&id).await?;
    assert_ne!(token_key, secret_key);
    assert_eq!(h.secret(&token_key).await?.as_deref(), Some(TOKEN));
    assert_eq!(
        h.secret(&secret_key).await?.as_deref(),
        Some(webhook_secret)
    );
    assert!(h.audit_count(&id).await? >= 1, "the create is audited");

    // The list shows the connection and neither secret.
    let (status, list) = h.as_admin(Method::GET, CONNECTIONS, None).await?;
    assert_eq!(status, StatusCode::OK, "{list}");
    openapi_schema::check_response(CONNECTIONS, "get", 200, &list)?;
    assert_eq!(list["canManage"], true);
    assert_eq!(list["items"].as_array().map(Vec::len), Some(1));
    assert_eq!(list["items"][0], created["connection"]);
    Ok(())
}

#[tokio::test]
async fn a_connection_defaults_to_gitlab_com_and_can_be_a_group() -> Result<()> {
    let h = harness().await?;
    h.gitlab
        .with_scope(&url("https://gitlab.com")?, group(7, "acme")?);
    let (status, created) = h
        .as_admin(
            Method::POST,
            CONNECTIONS,
            Some(json!({
                "name": "Acme group",
                "scopeKind": "group",
                "scopePath": "acme",
                "accessToken": TOKEN,
            })),
        )
        .await?;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    assert_eq!(created["connection"]["baseUrl"], "https://gitlab.com");
    assert_eq!(created["connection"]["scopeKind"], "group");
    assert_eq!(created["connection"]["scopeId"], 7);
    assert_eq!(
        h.calls(FakeMethod::ResolveScope),
        [url("https://gitlab.com")?]
    );
    Ok(())
}

#[tokio::test]
async fn the_base_url_is_normalised_and_anything_but_http_with_a_host_is_refused() -> Result<()> {
    let h = harness().await?;
    for (given, stored) in [
        ("https://GitLab.Example.com/", "https://gitlab.example.com"),
        (
            "https://git.example.com:8443/gitlab/",
            "https://git.example.com:8443/gitlab",
        ),
        ("http://gitlab.internal", "http://gitlab.internal"),
    ] {
        // Seeded with and without a trailing slash: which `Url` the handler
        // builds from the stored value is its own business.
        h.gitlab.with_scope(&url(stored)?, project(42, "acme/app")?);
        h.gitlab
            .with_scope(&url(&format!("{stored}/"))?, project(42, "acme/app")?);
        let (status, body) = h
            .as_admin(
                Method::POST,
                CONNECTIONS,
                Some(json!({
                    "name": format!("normalised {stored}"),
                    "baseUrl": given,
                    "scopeKind": "project",
                    "scopePath": "acme/app",
                    "accessToken": TOKEN,
                })),
            )
            .await?;
        assert_eq!(status, StatusCode::CREATED, "{given}: {body}");
        assert_eq!(body["connection"]["baseUrl"], stored, "{given}");
    }
    let calls_before = h.gitlab.calls().len();
    for refused in [
        "ftp://gitlab.example.com",
        "gitlab.example.com",
        "https://",
        "file:///srv/gitlab",
        "not a url",
    ] {
        let (status, body) = h
            .as_admin(
                Method::POST,
                CONNECTIONS,
                Some(json!({
                    "name": "refused",
                    "baseUrl": refused,
                    "scopeKind": "project",
                    "scopePath": "acme/app",
                    "accessToken": TOKEN,
                })),
            )
            .await?;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{refused}: {body}");
        openapi_schema::check_response(CONNECTIONS, "post", 400, &body)?;
    }
    assert_eq!(
        h.gitlab.calls().len(),
        calls_before,
        "an invalid base URL never reaches GitLab"
    );
    Ok(())
}

#[tokio::test]
async fn invalid_create_input_is_a_400_before_gitlab_is_called() -> Result<()> {
    let h = harness().await?;
    let valid = json!({
        "name": "Acme app",
        "baseUrl": GITLAB,
        "scopeKind": "project",
        "scopePath": "acme/app",
        "accessToken": TOKEN,
    });
    for (field, value) in [
        ("name", json!("")),
        ("name", json!("   ")),
        ("scopeKind", json!("repository")),
        ("scopePath", json!("")),
        ("accessToken", json!("")),
    ] {
        let mut body = valid.clone();
        body[field] = value.clone();
        let (status, response) = h.as_admin(Method::POST, CONNECTIONS, Some(body)).await?;
        assert_eq!(
            status,
            StatusCode::BAD_REQUEST,
            "{field}={value}: {response}"
        );
        openapi_schema::check_response(CONNECTIONS, "post", 400, &response)?;
    }
    assert!(h.gitlab.calls().is_empty(), "{:?}", h.gitlab.calls());
    let rows: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM gitlab_connections")
        .fetch_one(&h.database)
        .await?;
    assert_eq!(rows, 0);
    Ok(())
}

#[tokio::test]
async fn a_duplicate_base_url_and_scope_is_a_409_and_stores_nothing_more() -> Result<()> {
    let h = harness().await?;
    let first = h.create(GITLAB, project(42, "acme/app")?, "first").await?;
    let secrets_before: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM runtime_secrets")
        .fetch_one(&h.database)
        .await?;
    // The same instance written differently, and the same project.
    let (status, body) = h
        .as_admin(
            Method::POST,
            CONNECTIONS,
            Some(json!({
                "name": "second",
                "baseUrl": "https://gitlab.example.com/",
                "scopeKind": "project",
                "scopePath": "acme/app",
                "accessToken": REPLACEMENT_TOKEN,
            })),
        )
        .await?;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    openapi_schema::check_response(CONNECTIONS, "post", 409, &body)?;
    assert!(!body.to_string().contains(REPLACEMENT_TOKEN));
    let ids: Vec<String> = sqlx::query_scalar("SELECT id FROM gitlab_connections")
        .fetch_all(&h.database)
        .await?;
    assert_eq!(ids, [connection_id(&first)?]);
    let secrets_after: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM runtime_secrets")
        .fetch_one(&h.database)
        .await?;
    assert_eq!(
        secrets_after, secrets_before,
        "a refused duplicate stores no secret"
    );
    // The same project on another instance is a different connection.
    h.create(OTHER_GITLAB, project(42, "acme/app")?, "elsewhere")
        .await?;
    Ok(())
}

/// `(case, the failure GitLab answers with, the scope path, what the error
/// names)`.
type RefusalCase = (
    &'static str,
    Option<(FakeMethod, GitLabError)>,
    &'static str,
    &'static str,
);

#[tokio::test]
async fn gitlab_refusals_on_create_are_400s_that_name_the_reason_and_store_nothing() -> Result<()> {
    let h = harness().await?;
    let base = url(GITLAB)?;
    h.gitlab.with_scope(&base, project(42, "acme/app")?);
    // (case, the failure GitLab answers with, the path, what the error names)
    let cases: [RefusalCase; 6] = [
        (
            "bad token",
            Some((FakeMethod::ResolveScope, GitLabError::Unauthorized)),
            "acme/app",
            "rejected the access token",
        ),
        (
            "missing scope",
            Some((
                FakeMethod::VerifyAccess,
                GitLabError::MissingScope("create_runner".into()),
            )),
            "acme/app",
            "create_runner",
        ),
        (
            "missing role",
            Some((FakeMethod::VerifyAccess, GitLabError::MissingRole)),
            "acme/app",
            "maintainer",
        ),
        // Nothing is seeded at this path, so resolve_scope answers NotFound.
        ("unknown path", None, "acme/missing", "acme/missing"),
        (
            "rate limited",
            Some((
                FakeMethod::ResolveScope,
                GitLabError::RateLimited { retry_after: None },
            )),
            "acme/app",
            "later",
        ),
        (
            "upstream",
            Some((
                FakeMethod::VerifyAccess,
                GitLabError::Upstream("socket detail".into()),
            )),
            "acme/app",
            "gitlab request failed",
        ),
    ];
    for (case, failure, scope_path, named) in cases {
        if let Some((method, error)) = failure {
            h.gitlab.fail_next(&base, method, error);
        }
        let (status, body) = h
            .as_admin(
                Method::POST,
                CONNECTIONS,
                Some(json!({
                    "name": case,
                    "baseUrl": GITLAB,
                    "scopeKind": "project",
                    "scopePath": scope_path,
                    "accessToken": TOKEN,
                })),
            )
            .await?;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{case}: {body}");
        openapi_schema::check_response(CONNECTIONS, "post", 400, &body)?;
        assert!(error_text(&body).contains(named), "{case}: {body}");
        assert!(
            !body.to_string().contains(TOKEN),
            "{case}: the token is never echoed"
        );
        if case == "upstream" {
            assert!(
                !body.to_string().contains("socket detail"),
                "upstream detail stays in the logs"
            );
        }
    }
    let rows: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM gitlab_connections")
        .fetch_one(&h.database)
        .await?;
    let secrets: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM runtime_secrets")
        .fetch_one(&h.database)
        .await?;
    assert_eq!((rows, secrets), (0, 0), "a refused create stores nothing");
    Ok(())
}

// ---------------------------------------------------------------------------
// Connections: who may call
// ---------------------------------------------------------------------------

#[tokio::test]
async fn only_a_system_admin_manages_connections_and_a_signed_out_caller_gets_401() -> Result<()> {
    let h = harness().await?;
    let created = h
        .create(GITLAB, project(42, "acme/app")?, "Acme app")
        .await?;
    let id = connection_id(&created)?;
    let calls_before = h.gitlab.calls().len();
    let (token_key, secret_key) = h.secret_keys(&id).await?;
    let secret_before = h.secret(&secret_key).await?;

    let create = json!({"name": "member", "baseUrl": OTHER_GITLAB, "scopeKind": "project", "scopePath": "acme/app", "accessToken": TOKEN});
    let rename = json!({"name": "renamed by a member"});
    let writes: [(Method, String, &str, &str, Option<Value>); 5] = [
        (
            Method::POST,
            CONNECTIONS.to_owned(),
            CONNECTIONS,
            "post",
            Some(create),
        ),
        (
            Method::PATCH,
            path(CONNECTION, &id),
            CONNECTION,
            "patch",
            Some(rename),
        ),
        (
            Method::DELETE,
            path(CONNECTION, &id),
            CONNECTION,
            "delete",
            None,
        ),
        (Method::POST, path(VERIFY, &id), VERIFY, "post", None),
        (Method::POST, path(ROTATE, &id), ROTATE, "post", None),
    ];
    for (method, uri, template, verb, body) in &writes {
        let (status, response) = h
            .send(method.clone(), uri, Some(&h.member), body.clone())
            .await?;
        assert_eq!(
            status,
            StatusCode::FORBIDDEN,
            "member {method} {uri}: {response}"
        );
        openapi_schema::check_response(template, verb, 403, &response)?;
        let (status, response) = h.send(method.clone(), uri, None, body.clone()).await?;
        assert_eq!(
            status,
            StatusCode::UNAUTHORIZED,
            "signed out {method} {uri}: {response}"
        );
        openapi_schema::check_response(template, verb, 401, &response)?;
    }
    assert_eq!(
        h.gitlab.calls().len(),
        calls_before,
        "a refused caller never reaches GitLab"
    );
    let name: String = sqlx::query_scalar("SELECT name FROM gitlab_connections WHERE id=?")
        .bind(&id)
        .fetch_one(&h.database)
        .await?;
    assert_eq!(name, "Acme app");
    assert_eq!(h.secret(&token_key).await?.as_deref(), Some(TOKEN));
    assert_eq!(
        h.secret(&secret_key).await?,
        secret_before,
        "a member cannot rotate"
    );

    // Any signed-in user may list; only an administrator may manage.
    let (status, list) = h
        .send(Method::GET, CONNECTIONS, Some(&h.member), None)
        .await?;
    assert_eq!(status, StatusCode::OK, "{list}");
    openapi_schema::check_response(CONNECTIONS, "get", 200, &list)?;
    assert_eq!(list["canManage"], false);
    assert_eq!(list["items"][0]["canManage"], false);
    let (status, list) = h.send(Method::GET, CONNECTIONS, None, None).await?;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "{list}");
    Ok(())
}

// ---------------------------------------------------------------------------
// Connections: update, verify, rotate
// ---------------------------------------------------------------------------

#[tokio::test]
async fn patch_renames_without_calling_gitlab_and_leaves_the_token() -> Result<()> {
    let h = harness().await?;
    let created = h
        .create(GITLAB, project(42, "acme/app")?, "Acme app")
        .await?;
    let id = connection_id(&created)?;
    let calls_before = h.gitlab.calls().len();
    let audits_before = h.audit_count(&id).await?;
    let (status, updated) = h
        .as_admin(
            Method::PATCH,
            &path(CONNECTION, &id),
            Some(json!({"name": "Acme renamed"})),
        )
        .await?;
    assert_eq!(status, StatusCode::OK, "{updated}");
    openapi_schema::check_response(CONNECTION, "patch", 200, &updated)?;
    assert_eq!(updated["name"], "Acme renamed");
    assert_eq!(updated["id"], id.as_str());
    assert_eq!(updated["scopePath"], "acme/app");
    assert_eq!(
        h.gitlab.calls().len(),
        calls_before,
        "a rename needs no GitLab call"
    );
    let (token_key, _) = h.secret_keys(&id).await?;
    assert_eq!(h.secret(&token_key).await?.as_deref(), Some(TOKEN));
    assert!(
        h.audit_count(&id).await? > audits_before,
        "the update is audited"
    );

    for (body, expected) in [
        (json!({"name": ""}), StatusCode::BAD_REQUEST),
        (json!({"accessToken": ""}), StatusCode::BAD_REQUEST),
    ] {
        let (status, response) = h
            .as_admin(Method::PATCH, &path(CONNECTION, &id), Some(body.clone()))
            .await?;
        assert_eq!(status, expected, "{body}: {response}");
        openapi_schema::check_response(CONNECTION, "patch", 400, &response)?;
    }
    let (status, response) = h
        .as_admin(
            Method::PATCH,
            &path(CONNECTION, UNKNOWN_CONNECTION),
            Some(json!({"name": "nobody"})),
        )
        .await?;
    assert_eq!(status, StatusCode::NOT_FOUND, "{response}");
    openapi_schema::check_response(CONNECTION, "patch", 404, &response)?;
    Ok(())
}

#[tokio::test]
async fn patch_with_a_new_token_verifies_it_against_the_stored_scope_before_storing_it()
-> Result<()> {
    let h = harness().await?;
    let created = h
        .create(GITLAB, project(42, "acme/app")?, "Acme app")
        .await?;
    let id = connection_id(&created)?;
    let (token_key, _) = h.secret_keys(&id).await?;
    let base = url(GITLAB)?;

    // A refused token is not stored.
    h.gitlab.fail_next(
        &base,
        FakeMethod::VerifyAccess,
        GitLabError::MissingScope("create_runner".into()),
    );
    let (status, body) = h
        .as_admin(
            Method::PATCH,
            &path(CONNECTION, &id),
            Some(json!({"accessToken": REPLACEMENT_TOKEN})),
        )
        .await?;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    openapi_schema::check_response(CONNECTION, "patch", 400, &body)?;
    assert!(error_text(&body).contains("create_runner"), "{body}");
    assert_eq!(h.secret(&token_key).await?.as_deref(), Some(TOKEN));

    let resolves_before = h.calls(FakeMethod::ResolveScope).len();
    let verifies_before = h.calls(FakeMethod::VerifyAccess).len();
    let (status, body) = h
        .as_admin(
            Method::PATCH,
            &path(CONNECTION, &id),
            Some(json!({"accessToken": REPLACEMENT_TOKEN})),
        )
        .await?;
    assert_eq!(status, StatusCode::OK, "{body}");
    openapi_schema::check_response(CONNECTION, "patch", 200, &body)?;
    assert_eq!(
        body["name"], "Acme app",
        "a token-only PATCH keeps the name"
    );
    assert_eq!(h.calls(FakeMethod::VerifyAccess).len(), verifies_before + 1);
    assert_eq!(
        h.calls(FakeMethod::ResolveScope).len(),
        resolves_before,
        "the scope is not resolved again"
    );
    let (same_key, _) = h.secret_keys(&id).await?;
    assert_eq!(same_key, token_key, "overwritten under the same key");
    assert_eq!(
        h.secret(&token_key).await?.as_deref(),
        Some(REPLACEMENT_TOKEN)
    );
    Ok(())
}

#[tokio::test]
async fn verify_answers_204_or_a_400_naming_what_the_token_lacks() -> Result<()> {
    let h = harness().await?;
    let created = h
        .create(GITLAB, project(42, "acme/app")?, "Acme app")
        .await?;
    let id = connection_id(&created)?;
    let base = url(GITLAB)?;
    let verifies_before = h.calls(FakeMethod::VerifyAccess).len();
    let (status, body) = h.as_admin(Method::POST, &path(VERIFY, &id), None).await?;
    assert_eq!(status, StatusCode::NO_CONTENT, "{body}");
    openapi_schema::check_response(VERIFY, "post", 204, &body)?;
    let verifies = h.calls(FakeMethod::VerifyAccess);
    assert_eq!(verifies.len(), verifies_before + 1, "verify asks GitLab");
    assert_eq!(verifies.last(), Some(&base), "at the connection's instance");

    for (error, named) in [
        (GitLabError::Unauthorized, "rejected the access token"),
        (GitLabError::MissingScope("read_api".into()), "read_api"),
        (GitLabError::MissingRole, "maintainer"),
    ] {
        h.gitlab
            .fail_next(&base, FakeMethod::VerifyAccess, error.clone());
        let (status, body) = h.as_admin(Method::POST, &path(VERIFY, &id), None).await?;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{error:?}: {body}");
        openapi_schema::check_response(VERIFY, "post", 400, &body)?;
        assert!(error_text(&body).contains(named), "{error:?}: {body}");
    }
    let (status, body) = h
        .as_admin(Method::POST, &path(VERIFY, UNKNOWN_CONNECTION), None)
        .await?;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
    openapi_schema::check_response(VERIFY, "post", 404, &body)?;
    Ok(())
}

#[tokio::test]
async fn rotating_overwrites_the_webhook_secret_so_the_old_one_stops_working() -> Result<()> {
    let h = harness().await?;
    let created = h
        .create(GITLAB, project(42, "acme/app")?, "Acme app")
        .await?;
    let id = connection_id(&created)?;
    let old_secret = created["webhookSecret"]
        .as_str()
        .context("webhookSecret")?
        .to_owned();
    let (_, secret_key) = h.secret_keys(&id).await?;
    let secrets_before: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM runtime_secrets")
        .fetch_one(&h.database)
        .await?;
    let audits_before = h.audit_count(&id).await?;

    let (status, rotated) = h.as_admin(Method::POST, &path(ROTATE, &id), None).await?;
    assert_eq!(status, StatusCode::OK, "{rotated}");
    openapi_schema::check_response(ROTATE, "post", 200, &rotated)?;
    assert_eq!(rotated["webhookUrl"], WEBHOOK_URL);
    let new_secret = rotated["webhookSecret"].as_str().context("webhookSecret")?;
    assert_ne!(new_secret, old_secret);
    assert!(new_secret.len() >= 32);
    let (_, same_key) = h.secret_keys(&id).await?;
    assert_eq!(same_key, secret_key, "#140 reads the secret by this key");
    assert_eq!(h.secret(&secret_key).await?.as_deref(), Some(new_secret));
    let secrets_after: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM runtime_secrets")
        .fetch_one(&h.database)
        .await?;
    assert_eq!(
        secrets_after, secrets_before,
        "the old secret is overwritten, not kept"
    );
    let stored: Vec<String> = sqlx::query_scalar("SELECT value FROM runtime_secrets")
        .fetch_all(&h.database)
        .await?;
    for sealed in stored {
        assert_ne!(
            h.vault.open(&sealed)?,
            old_secret,
            "the previous secret is gone"
        );
    }
    assert!(
        h.audit_count(&id).await? > audits_before,
        "the rotation is audited"
    );

    let (status, body) = h
        .as_admin(Method::POST, &path(ROTATE, UNKNOWN_CONNECTION), None)
        .await?;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
    openapi_schema::check_response(ROTATE, "post", 404, &body)?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Connections: delete
// ---------------------------------------------------------------------------

#[tokio::test]
async fn deleting_a_connection_removes_it_and_both_stored_secrets() -> Result<()> {
    let h = harness().await?;
    let created = h
        .create(GITLAB, project(42, "acme/app")?, "Acme app")
        .await?;
    let id = connection_id(&created)?;
    let (token_key, secret_key) = h.secret_keys(&id).await?;
    let audits_before = h.audit_count(&id).await?;
    let (status, body) = h
        .as_admin(Method::DELETE, &path(CONNECTION, &id), None)
        .await?;
    assert_eq!(status, StatusCode::NO_CONTENT, "{body}");
    openapi_schema::check_response(CONNECTION, "delete", 204, &body)?;
    assert_eq!(
        h.count("SELECT COUNT(*) FROM gitlab_connections WHERE id=?", &id)
            .await?,
        0
    );
    assert_eq!(
        h.secret(&token_key).await?,
        None,
        "the access token is removed"
    );
    assert_eq!(
        h.secret(&secret_key).await?,
        None,
        "the webhook secret is removed"
    );
    assert!(
        h.audit_count(&id).await? > audits_before,
        "the delete is audited"
    );
    let (status, body) = h
        .as_admin(Method::DELETE, &path(CONNECTION, &id), None)
        .await?;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
    openapi_schema::check_response(CONNECTION, "delete", 404, &body)?;
    Ok(())
}

/// Deletes `id`, expecting a 409 that leaves the connection and its
/// secrets in place.
async fn assert_delete_refused(h: &Harness, id: &str, why: &str) -> Result<()> {
    let (token_key, secret_key) = h.secret_keys(id).await?;
    let (status, body) = h
        .as_admin(Method::DELETE, &path(CONNECTION, id), None)
        .await?;
    assert_eq!(status, StatusCode::CONFLICT, "{why}: {body}");
    openapi_schema::check_response(CONNECTION, "delete", 409, &body)?;
    assert_eq!(
        h.count("SELECT COUNT(*) FROM gitlab_connections WHERE id=?", id)
            .await?,
        1,
        "{why}"
    );
    assert_eq!(h.secret(&token_key).await?.as_deref(), Some(TOKEN), "{why}");
    assert!(h.secret(&secret_key).await?.is_some(), "{why}");
    Ok(())
}

#[tokio::test]
async fn a_connection_a_pool_targets_cannot_be_deleted() -> Result<()> {
    let h = harness().await?;
    let created = h
        .create(GITLAB, project(42, "acme/app")?, "Acme app")
        .await?;
    let id = connection_id(&created)?;
    h.seed_pool(GITLAB_POOL, "gitlab-pool", "gitlab", &[&id])
        .await?;
    assert_delete_refused(&h, &id, "a pool targets it").await?;
    assert_eq!(
        h.count(
            "SELECT COUNT(*) FROM runner_pool_gitlab_connections WHERE connection_id=?",
            &id
        )
        .await?,
        1,
        "the pool keeps its target"
    );
    Ok(())
}

#[tokio::test]
async fn a_connection_a_fleet_target_references_is_refused_with_409_not_500() -> Result<()> {
    let h = harness().await?;
    let created = h
        .create(GITLAB, project(42, "acme/app")?, "Acme app")
        .await?;
    let id = connection_id(&created)?;
    gridops_core::fleet::targets::ensure_target(
        &h.database,
        &CiTarget::GitlabProject {
            connection_id: GitlabConnectionId::new(&id).map_err(|error| anyhow!("{error}"))?,
            project_id: external_id(42)?,
        },
    )
    .await
    .map_err(|error| anyhow!("{error}"))?;
    assert_delete_refused(&h, &id, "a fleet target references it").await
}

#[tokio::test]
async fn a_connection_with_a_live_runner_cannot_be_deleted_until_the_runner_is_gone() -> Result<()>
{
    let h = harness().await?;
    let created = h
        .create(GITLAB, project(42, "acme/app")?, "Acme app")
        .await?;
    let id = connection_id(&created)?;
    // The pool no longer targets the connection, but its runner remains.
    h.seed_pool(GITLAB_POOL, "gitlab-pool", "gitlab", &[])
        .await?;
    h.seed_runner("runner-live", GITLAB_POOL, Some(&id), Some(77))
        .await?;
    assert_delete_refused(&h, &id, "a live runner references it").await?;
    sqlx::query("UPDATE runners SET deleted_at=1,status='deleted' WHERE id='runner-live'")
        .execute(&h.database)
        .await?;
    let (status, body) = h
        .as_admin(Method::DELETE, &path(CONNECTION, &id), None)
        .await?;
    assert_eq!(
        status,
        StatusCode::NO_CONTENT,
        "a deleted runner does not hold it: {body}"
    );
    Ok(())
}

// ---------------------------------------------------------------------------
// Connections: secret non-disclosure across every response
// ---------------------------------------------------------------------------

#[tokio::test]
async fn no_response_but_create_and_rotate_carries_a_secret_and_no_audit_event_does() -> Result<()>
{
    let h = harness().await?;
    let created = h
        .create(GITLAB, project(42, "acme/app")?, "Acme app")
        .await?;
    let id = connection_id(&created)?;
    let first_secret = created["webhookSecret"]
        .as_str()
        .context("secret")?
        .to_owned();
    let (_, rotated) = h.as_admin(Method::POST, &path(ROTATE, &id), None).await?;
    let second_secret = rotated["webhookSecret"]
        .as_str()
        .context("secret")?
        .to_owned();
    h.seed_pool(GITLAB_POOL, "gitlab-pool", "gitlab", &[&id])
        .await?;

    let secrets = [
        TOKEN,
        REPLACEMENT_TOKEN,
        first_secret.as_str(),
        second_secret.as_str(),
    ];
    let mut responses = Vec::new();
    responses.push(("list", h.as_admin(Method::GET, CONNECTIONS, None).await?));
    responses.push((
        "rename",
        h.as_admin(
            Method::PATCH,
            &path(CONNECTION, &id),
            Some(json!({"name": "renamed"})),
        )
        .await?,
    ));
    responses.push((
        "replace token",
        h.as_admin(
            Method::PATCH,
            &path(CONNECTION, &id),
            Some(json!({"accessToken": REPLACEMENT_TOKEN})),
        )
        .await?,
    ));
    responses.push((
        "verify",
        h.as_admin(Method::POST, &path(VERIFY, &id), None).await?,
    ));
    responses.push((
        "delete refused",
        h.as_admin(Method::DELETE, &path(CONNECTION, &id), None)
            .await?,
    ));
    responses.push((
        "pool detail",
        h.as_admin(
            Method::GET,
            &format!("/api/v1/runner-pools/{GITLAB_POOL}"),
            None,
        )
        .await?,
    ));
    responses.push((
        "pool options",
        h.as_admin(Method::GET, "/api/v1/runner-pools/options", None)
            .await?,
    ));
    for (what, (status, body)) in &responses {
        assert!(
            status.is_success() || *status == StatusCode::CONFLICT,
            "{what}: {status} {body}"
        );
        let text = body.to_string();
        for secret in secrets {
            assert!(!text.contains(secret), "{what} discloses a secret: {text}");
        }
        assert!(!text.contains("webhookSecret"), "{what}: {text}");
    }
    let audit: Vec<(String, String)> =
        sqlx::query_as("SELECT actor_label,metadata FROM audit_events")
            .fetch_all(&h.database)
            .await?;
    assert_ne!(audit.len(), 0, "the actions above are audited");
    for (label, metadata) in audit {
        for secret in secrets {
            assert!(
                !metadata.contains(secret) && !label.contains(secret),
                "audit: {metadata}"
            );
        }
    }
    let row: String = sqlx::query_scalar(
        "SELECT id||name||base_url||scope_path||access_token_key||webhook_secret_key FROM gitlab_connections WHERE id=?",
    )
    .bind(&id)
    .fetch_one(&h.database)
    .await?;
    for secret in secrets {
        assert!(
            !row.contains(secret),
            "the connection row holds a secret in plain text"
        );
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Runner pools with GitLab targets
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_gitlab_pool_is_created_with_its_targets_and_hands_its_starts_to_the_reconciler()
-> Result<()> {
    let h = harness().await?;
    h.seed_connection(CONNECTION_1, GITLAB, project(42, "acme/app")?)
        .await?;
    let mut body = gitlab_pool_body("gitlab-pool", &[CONNECTION_1]);
    body["runUntagged"] = json!(true);
    let (status, created) = h
        .as_admin(Method::POST, "/api/v1/runner-pools", Some(body))
        .await?;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    let pool_id = created["id"].as_str().context("pool id")?.to_owned();

    // D14: the API never starts a GitLab pool's runners itself.
    assert_eq!(created["handedToReconciler"], 1, "{created}");
    assert_eq!(created["provisioned"], json!([]), "{created}");
    assert_eq!(
        h.count("SELECT COUNT(*) FROM runners WHERE pool_id=?", &pool_id)
            .await?,
        0
    );
    assert!(
        h.gitlab
            .calls()
            .iter()
            .all(|call| call.method != FakeMethod::CreateRunner)
    );

    let (scope, run_untagged, runner_group_id): (String, bool, i64) =
        sqlx::query_as("SELECT scope,run_untagged,runner_group_id FROM runner_pools WHERE id=?")
            .bind(&pool_id)
            .fetch_one(&h.database)
            .await?;
    assert_eq!(scope, "gitlab");
    assert!(run_untagged);
    assert_eq!(
        runner_group_id, 1,
        "a GitLab pool stores runner group 1 whatever is sent"
    );
    let targets: Vec<String> = sqlx::query_scalar(
        "SELECT connection_id FROM runner_pool_gitlab_connections WHERE pool_id=?",
    )
    .bind(&pool_id)
    .fetch_all(&h.database)
    .await?;
    assert_eq!(targets, [CONNECTION_1]);
    let metadata: String = sqlx::query_scalar(
        "SELECT metadata FROM audit_events WHERE action='runner_pool.created' AND target_id=?",
    )
    .bind(&pool_id)
    .fetch_one(&h.database)
    .await?;
    let metadata: Value = serde_json::from_str(&metadata)?;
    assert_eq!(
        metadata["gitlabConnectionIds"],
        json!([CONNECTION_1]),
        "{metadata}"
    );
    assert_eq!(metadata["runUntagged"], true, "{metadata}");

    let (status, detail) = h
        .as_admin(
            Method::GET,
            &format!("/api/v1/runner-pools/{pool_id}"),
            None,
        )
        .await?;
    assert_eq!(status, StatusCode::OK, "{detail}");
    assert_eq!(detail["runUntagged"], true);
    assert_eq!(
        detail["gitlabConnections"],
        json!([{
            "id": CONNECTION_1,
            "name": format!("seeded {CONNECTION_1}"),
            "baseUrl": GITLAB,
            "scopeKind": "project",
            "scopePath": "acme/app",
        }])
    );
    Ok(())
}

#[tokio::test]
async fn creating_a_gitlab_pool_needs_a_system_admin_and_existing_connections() -> Result<()> {
    let h = harness().await?;
    h.seed_connection(CONNECTION_1, GITLAB, project(42, "acme/app")?)
        .await?;
    let (status, body) = h
        .as_admin(
            Method::POST,
            "/api/v1/runner-pools",
            Some(gitlab_pool_body("gitlab-pool", &[CONNECTION_1])),
        )
        .await?;
    assert_eq!(status, StatusCode::CREATED, "the control case: {body}");
    let created = body["id"].as_str().context("pool id")?.to_owned();
    let (status, detail) = h
        .as_admin(
            Method::GET,
            &format!("/api/v1/runner-pools/{created}"),
            None,
        )
        .await?;
    assert_eq!(status, StatusCode::OK, "{detail}");
    assert_eq!(
        detail["runUntagged"], false,
        "runUntagged defaults to false"
    );

    // The member administers installation 42 but is not a system admin.
    let (status, body) = h
        .send(
            Method::POST,
            "/api/v1/runner-pools",
            Some(&h.member),
            Some(gitlab_pool_body("member-pool", &[CONNECTION_1])),
        )
        .await?;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    let (status, body) = h
        .as_admin(
            Method::POST,
            "/api/v1/runner-pools",
            Some(gitlab_pool_body("ghost-pool", &[UNKNOWN_CONNECTION])),
        )
        .await?;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    let pools: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM runner_pools")
        .fetch_one(&h.database)
        .await?;
    assert_eq!(pools, 1, "only the control pool was created");
    Ok(())
}

async fn pool_targets(h: &Harness, pool_id: &str) -> Result<Vec<String>> {
    Ok(sqlx::query_scalar(
        "SELECT connection_id FROM runner_pool_gitlab_connections WHERE pool_id=? ORDER BY connection_id",
    )
    .bind(pool_id)
    .fetch_all(&h.database)
    .await?)
}

/// `(run_untagged, runner_group_id)` of a pool.
async fn pool_flags(h: &Harness, pool_id: &str) -> Result<(bool, i64)> {
    Ok(
        sqlx::query_as("SELECT run_untagged,runner_group_id FROM runner_pools WHERE id=?")
            .bind(pool_id)
            .fetch_one(&h.database)
            .await?,
    )
}

#[tokio::test]
async fn updating_a_gitlab_pool_replaces_its_targets_and_rolls_runners_only_when_they_change()
-> Result<()> {
    let h = harness().await?;
    h.seed_connection(CONNECTION_1, GITLAB, project(42, "acme/app")?)
        .await?;
    h.seed_connection(CONNECTION_2, OTHER_GITLAB, group(7, "acme")?)
        .await?;
    h.seed_pool(GITLAB_POOL, "gitlab-pool", "gitlab", &[CONNECTION_1])
        .await?;
    let uri = format!("/api/v1/runner-pools/{GITLAB_POOL}");

    // (what, extra body fields, expected rollingReplacement)
    let steps: [(&str, Value, bool); 5] = [
        (
            "swap the target",
            json!({"gitlabConnectionIds": [CONNECTION_2], "runUntagged": false}),
            true,
        ),
        (
            "the same again",
            json!({"gitlabConnectionIds": [CONNECTION_2], "runUntagged": false}),
            false,
        ),
        (
            "another runner group",
            json!({"gitlabConnectionIds": [CONNECTION_2], "runUntagged": false, "runnerGroupId": 3}),
            false,
        ),
        (
            "run untagged jobs",
            json!({"gitlabConnectionIds": [CONNECTION_2], "runUntagged": true}),
            true,
        ),
        ("fields left out are kept", json!({}), false),
    ];
    for (what, extra, rolls) in steps {
        let (status, body) = h
            .as_admin(
                Method::PUT,
                &uri,
                Some(pool_update_body("gitlab-pool", &extra)),
            )
            .await?;
        assert_eq!(status, StatusCode::OK, "{what}: {body}");
        assert_eq!(
            pool_targets(&h, GITLAB_POOL).await?,
            [CONNECTION_2],
            "{what}"
        );
        assert_eq!(body["rollingReplacement"], rolls, "{what}: {body}");
        assert_eq!(
            pool_flags(&h, GITLAB_POOL).await?.1,
            1,
            "{what}: runner group stays 1"
        );
    }
    assert!(
        pool_flags(&h, GITLAB_POOL).await?.0,
        "runUntagged left out keeps true"
    );
    let (status, detail) = h.as_admin(Method::GET, &uri, None).await?;
    assert_eq!(status, StatusCode::OK, "{detail}");
    assert_eq!(detail["runUntagged"], true);
    assert_eq!(detail["gitlabConnections"][0]["id"], CONNECTION_2);
    assert_eq!(detail["gitlabConnections"][0]["scopeKind"], "group");
    let metadata: Vec<String> = sqlx::query_scalar(
        "SELECT metadata FROM audit_events WHERE action='runner_pool.updated' AND target_id=?",
    )
    .bind(GITLAB_POOL)
    .fetch_all(&h.database)
    .await?;
    assert!(
        metadata
            .iter()
            .any(|row| row.contains("gitlabConnectionIds") && row.contains("runUntagged")),
        "{metadata:?}"
    );
    Ok(())
}

#[tokio::test]
async fn an_update_cannot_move_a_pool_between_gitlab_and_other_targets() -> Result<()> {
    let h = harness().await?;
    h.seed_connection(CONNECTION_1, GITLAB, project(42, "acme/app")?)
        .await?;
    h.seed_pool(GITLAB_POOL, "gitlab-pool", "gitlab", &[CONNECTION_1])
        .await?;
    h.seed_pool(ORG_POOL, "org-pool", "organization", &[])
        .await?;
    let now = gridops_core::now_millis();
    sqlx::query("INSERT INTO bitbucket_connections (id,name,workspace,workspace_uuid,access_token_key,created_by,created_at,updated_at) VALUES (?,'bb','acme','{11111111-1111-4111-8111-111111111111}','test.bitbucket.token',NULL,?,?)")
        .bind(BITBUCKET_CONNECTION).bind(now).bind(now)
        .execute(&h.database).await?;
    let tart = json!({
        "providers": ["docker", "tart"],
        "tartImage": "ghcr.io/cirruslabs/macos-sequoia-base:latest",
    });
    let with = |extra: Value| {
        let mut merged = tart.clone();
        for (key, value) in extra.as_object().into_iter().flatten() {
            merged[key] = value.clone();
        }
        merged
    };
    let cases = [
        (
            "an organization pool gains a GitLab target",
            ORG_POOL,
            "org-pool",
            json!({"gitlabConnectionIds": [CONNECTION_1]}),
        ),
        (
            "a GitLab pool loses every GitLab target",
            GITLAB_POOL,
            "gitlab-pool",
            json!({"gitlabConnectionIds": []}),
        ),
        (
            "a GitLab pool gains a Bitbucket target",
            GITLAB_POOL,
            "gitlab-pool",
            with(json!({"bitbucketConnectionIds": [BITBUCKET_CONNECTION]})),
        ),
        (
            "a GitLab pool names a connection that does not exist",
            GITLAB_POOL,
            "gitlab-pool",
            json!({"gitlabConnectionIds": [UNKNOWN_CONNECTION]}),
        ),
    ];
    for (what, pool_id, name, extra) in cases {
        let (status, body) = h
            .as_admin(
                Method::PUT,
                &format!("/api/v1/runner-pools/{pool_id}"),
                Some(pool_update_body(name, &extra)),
            )
            .await?;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{what}: {body}");
    }
    assert_eq!(
        h.count(
            "SELECT COUNT(*) FROM runner_pool_gitlab_connections WHERE pool_id=?",
            ORG_POOL
        )
        .await?,
        0
    );
    assert_eq!(pool_targets(&h, GITLAB_POOL).await?, [CONNECTION_1]);
    assert_eq!(
        h.count(
            "SELECT COUNT(*) FROM runner_pool_bitbucket_connections WHERE pool_id=?",
            GITLAB_POOL
        )
        .await?,
        0
    );
    Ok(())
}

#[tokio::test]
async fn reconciling_a_gitlab_pool_hands_its_starts_to_the_reconciler() -> Result<()> {
    let h = harness().await?;
    h.seed_connection(CONNECTION_1, GITLAB, project(42, "acme/app")?)
        .await?;
    h.seed_pool(GITLAB_POOL, "gitlab-pool", "gitlab", &[CONNECTION_1])
        .await?;
    let (status, body) = h
        .as_admin(
            Method::POST,
            &format!("/api/v1/runner-pools/{GITLAB_POOL}/action"),
            Some(json!({"action": "reconcile"})),
        )
        .await?;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["handedToReconciler"], 1, "{body}");
    assert_eq!(body["provisioned"], 0, "{body}");
    assert_eq!(
        h.count("SELECT COUNT(*) FROM runners WHERE pool_id=?", GITLAB_POOL)
            .await?,
        0
    );
    Ok(())
}

// ---------------------------------------------------------------------------
// Runner delete: the GitLab arm (D17)
// ---------------------------------------------------------------------------

/// Deletes `runner_id` through the API and checks it is gone from `GridOps`
/// with GitLab-specific (never GitHub) bookkeeping.
async fn delete_runner_and_check_bookkeeping(h: &Harness, runner_id: &str) -> Result<()> {
    let (status, body) = h
        .as_admin(
            Method::POST,
            &format!("/api/v1/runners/{runner_id}/action"),
            Some(json!({"action": "delete"})),
        )
        .await?;
    assert_eq!(status, StatusCode::OK, "{runner_id}: {body}");
    assert_runner_gone_without_github_bookkeeping(h, runner_id).await
}

async fn assert_runner_gone_without_github_bookkeeping(h: &Harness, runner_id: &str) -> Result<()> {
    let live = h
        .count(
            "SELECT COUNT(*) FROM runners WHERE id=? AND deleted_at IS NULL",
            runner_id,
        )
        .await?;
    assert_eq!(live, 0, "{runner_id} is deleted in GridOps");
    let queued = h
        .count(
            "SELECT COUNT(*) FROM github_runner_cleanup WHERE id=?",
            runner_id,
        )
        .await?;
    assert_eq!(queued, 0, "{runner_id} is never queued for GitHub cleanup");
    let events: Vec<(String, String)> = sqlx::query_as(
        "SELECT message,metadata FROM runner_events WHERE runner_id=? AND event='Runner deleted'",
    )
    .bind(runner_id)
    .fetch_all(&h.database)
    .await?;
    for (message, metadata) in events {
        assert!(!message.contains("GitHub"), "{runner_id}: {message}");
        assert!(
            !metadata.contains("githubCleanup"),
            "{runner_id}: {metadata}"
        );
        assert!(!metadata.contains(TOKEN), "{runner_id}: {metadata}");
    }
    Ok(())
}

#[tokio::test]
async fn deleting_a_gitlab_runner_deletes_it_at_its_gitlab_instance() -> Result<()> {
    let h = harness().await?;
    let scope = project(42, "acme/app")?;
    h.seed_connection(CONNECTION_1, GITLAB, scope.clone())
        .await?;
    h.seed_pool(GITLAB_POOL, "gitlab-pool", "gitlab", &[CONNECTION_1])
        .await?;
    let kept = h.gitlab_runner(GITLAB, &scope).await?;
    let gitlab_id = h.gitlab_runner(GITLAB, &scope).await?;
    h.seed_runner("runner-1", GITLAB_POOL, Some(CONNECTION_1), Some(gitlab_id))
        .await?;
    delete_runner_and_check_bookkeeping(&h, "runner-1").await?;
    assert_eq!(h.calls(FakeMethod::DeleteRunner), [url(GITLAB)?]);
    let left: Vec<i64> = h
        .gitlab
        .runners(&url(GITLAB)?)
        .iter()
        .map(|runner| i64::try_from(runner.id.get()).unwrap_or(-1))
        .collect();
    assert_eq!(left, [kept], "exactly that runner was deleted at GitLab");
    Ok(())
}

#[tokio::test]
async fn a_runner_gitlab_no_longer_knows_is_still_deleted() -> Result<()> {
    let h = harness().await?;
    h.seed_connection(CONNECTION_1, GITLAB, project(42, "acme/app")?)
        .await?;
    h.seed_pool(GITLAB_POOL, "gitlab-pool", "gitlab", &[CONNECTION_1])
        .await?;
    // Nothing registered with this id: the fake answers NotFound.
    h.seed_runner("runner-gone", GITLAB_POOL, Some(CONNECTION_1), Some(999))
        .await?;
    delete_runner_and_check_bookkeeping(&h, "runner-gone").await?;
    assert_eq!(h.calls(FakeMethod::DeleteRunner), [url(GITLAB)?]);
    Ok(())
}

#[tokio::test]
async fn a_runner_is_deleted_even_when_gitlab_is_unreachable() -> Result<()> {
    let h = harness().await?;
    h.seed_connection(CONNECTION_1, GITLAB, project(42, "acme/app")?)
        .await?;
    h.seed_pool(GITLAB_POOL, "gitlab-pool", "gitlab", &[CONNECTION_1])
        .await?;
    h.seed_runner(
        "runner-unreachable",
        GITLAB_POOL,
        Some(CONNECTION_1),
        Some(5),
    )
    .await?;
    h.gitlab.fail_next(
        &url(GITLAB)?,
        FakeMethod::DeleteRunner,
        GitLabError::Upstream("connection refused".into()),
    );
    delete_runner_and_check_bookkeeping(&h, "runner-unreachable").await?;
    assert_eq!(h.calls(FakeMethod::DeleteRunner), [url(GITLAB)?]);
    Ok(())
}

#[tokio::test]
async fn a_runner_whose_connection_is_gone_is_deleted_without_calling_gitlab() -> Result<()> {
    let h = harness().await?;
    h.seed_pool(GITLAB_POOL, "gitlab-pool", "gitlab", &[])
        .await?;
    // `gitlab_connection_id` was SET NULL when its connection was deleted.
    h.seed_runner("runner-orphan", GITLAB_POOL, None, Some(5))
        .await?;
    // And one whose GitLab runner id was never recorded.
    h.seed_connection(CONNECTION_1, GITLAB, project(42, "acme/app")?)
        .await?;
    h.seed_runner("runner-unregistered", GITLAB_POOL, Some(CONNECTION_1), None)
        .await?;
    delete_runner_and_check_bookkeeping(&h, "runner-orphan").await?;
    delete_runner_and_check_bookkeeping(&h, "runner-unregistered").await?;
    assert!(
        h.calls(FakeMethod::DeleteRunner).is_empty(),
        "{:?}",
        h.gitlab.calls()
    );
    Ok(())
}

#[tokio::test]
async fn deleting_a_gitlab_pool_completes_while_gitlab_fails() -> Result<()> {
    let h = harness().await?;
    h.seed_connection(CONNECTION_1, GITLAB, project(42, "acme/app")?)
        .await?;
    h.seed_pool(GITLAB_POOL, "gitlab-pool", "gitlab", &[CONNECTION_1])
        .await?;
    h.seed_runner("runner-in-pool", GITLAB_POOL, Some(CONNECTION_1), Some(5))
        .await?;
    h.gitlab.fail_next(
        &url(GITLAB)?,
        FakeMethod::DeleteRunner,
        GitLabError::Upstream("timed out".into()),
    );
    let (status, body) = h
        .as_admin(
            Method::DELETE,
            &format!("/api/v1/runner-pools/{GITLAB_POOL}"),
            None,
        )
        .await?;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        h.count("SELECT COUNT(*) FROM runner_pools WHERE id=?", GITLAB_POOL)
            .await?,
        0
    );
    assert_eq!(
        h.calls(FakeMethod::DeleteRunner),
        [url(GITLAB)?],
        "GitLab was asked once"
    );
    let queued: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM github_runner_cleanup")
        .fetch_one(&h.database)
        .await?;
    assert_eq!(
        queued, 0,
        "a GitLab runner is never queued for GitHub cleanup"
    );
    Ok(())
}

#[tokio::test]
async fn a_base_url_with_credentials_a_query_or_a_fragment_is_refused_and_a_default_port_dropped()
-> Result<()> {
    let h = harness().await?;
    let calls_before = h.gitlab.calls().len();
    for refused in [
        "https://user:pw@gitlab.example.com",
        "https://user@gitlab.example.com",
        "https://gitlab.example.com/?token=abc",
        "https://gitlab.example.com/#frag",
    ] {
        let (status, body) = h
            .as_admin(
                Method::POST,
                CONNECTIONS,
                Some(json!({
                    "name": "refused",
                    "baseUrl": refused,
                    "scopeKind": "project",
                    "scopePath": "acme/app",
                    "accessToken": TOKEN,
                })),
            )
            .await?;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{refused}: {body}");
        assert!(
            !body.to_string().contains("pw"),
            "{refused}: the refusal does not echo the credentials: {body}"
        );
    }
    assert_eq!(h.gitlab.calls().len(), calls_before);
    let stored: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM gitlab_connections")
        .fetch_one(&h.database)
        .await?;
    assert_eq!(
        stored, 0,
        "no connection with a secret-bearing URL is stored"
    );

    h.gitlab.with_scope(&url(GITLAB)?, project(42, "acme/app")?);
    h.gitlab
        .with_scope(&url(&format!("{GITLAB}/"))?, project(42, "acme/app")?);
    let (status, body) = h
        .as_admin(
            Method::POST,
            CONNECTIONS,
            Some(json!({
                "name": "default port",
                "baseUrl": "https://gitlab.example.com:443/",
                "scopeKind": "project",
                "scopePath": "acme/app",
                "accessToken": TOKEN,
            })),
        )
        .await?;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    assert_eq!(body["connection"]["baseUrl"], GITLAB);
    Ok(())
}

#[tokio::test]
async fn a_connection_name_longer_than_80_characters_is_refused_but_80_is_allowed() -> Result<()> {
    let h = harness().await?;
    h.gitlab.with_scope(&url(GITLAB)?, project(42, "acme/app")?);
    h.gitlab
        .with_scope(&url(&format!("{GITLAB}/"))?, project(42, "acme/app")?);
    let (status, body) = h
        .as_admin(
            Method::POST,
            CONNECTIONS,
            Some(json!({
                "name": "n".repeat(81),
                "baseUrl": GITLAB,
                "scopeKind": "project",
                "scopePath": "acme/app",
                "accessToken": TOKEN,
            })),
        )
        .await?;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    openapi_schema::check_response(CONNECTIONS, "post", 400, &body)?;
    let (status, body) = h
        .as_admin(
            Method::POST,
            CONNECTIONS,
            Some(json!({
                "name": "n".repeat(80),
                "baseUrl": GITLAB,
                "scopeKind": "project",
                "scopePath": "acme/app",
                "accessToken": TOKEN,
            })),
        )
        .await?;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    Ok(())
}

#[tokio::test]
async fn an_empty_patch_returns_the_connection_unchanged_with_no_call_and_no_audit_event()
-> Result<()> {
    let h = harness().await?;
    let created = h
        .create(GITLAB, project(42, "acme/app")?, "Acme app")
        .await?;
    let id = connection_id(&created)?;
    let calls_before = h.gitlab.calls().len();
    let audits_before = h.audit_count(&id).await?;
    let (status, body) = h
        .as_admin(Method::PATCH, &path(CONNECTION, &id), Some(json!({})))
        .await?;
    assert_eq!(status, StatusCode::OK, "{body}");
    openapi_schema::check_response(CONNECTION, "patch", 200, &body)?;
    assert_eq!(body["name"], "Acme app");
    assert_eq!(body["id"], id.as_str());
    assert_eq!(h.gitlab.calls().len(), calls_before, "no GitLab call");
    assert_eq!(h.audit_count(&id).await?, audits_before, "no audit event");
    let (token_key, _) = h.secret_keys(&id).await?;
    assert_eq!(h.secret(&token_key).await?.as_deref(), Some(TOKEN));
    Ok(())
}

#[tokio::test]
async fn a_gitlab_runner_delete_records_whether_the_gitlab_cleanup_completed_or_was_deferred()
-> Result<()> {
    let h = harness().await?;
    let scope = project(42, "acme/app")?;
    h.seed_connection(CONNECTION_1, GITLAB, scope.clone())
        .await?;
    h.seed_pool(GITLAB_POOL, "gitlab-pool", "gitlab", &[CONNECTION_1])
        .await?;
    let registered = h.gitlab_runner(GITLAB, &scope).await?;
    h.seed_runner(
        "runner-done",
        GITLAB_POOL,
        Some(CONNECTION_1),
        Some(registered),
    )
    .await?;
    h.seed_runner("runner-deferred", GITLAB_POOL, Some(CONNECTION_1), Some(5))
        .await?;
    h.gitlab.fail_next(
        &url(GITLAB)?,
        FakeMethod::DeleteRunner,
        GitLabError::Upstream("timed out".into()),
    );
    // The deferred runner is deleted first so that the queued failure is its.
    delete_runner_and_check_bookkeeping(&h, "runner-deferred").await?;
    delete_runner_and_check_bookkeeping(&h, "runner-done").await?;
    for (runner, expected) in [("runner-done", "complete"), ("runner-deferred", "deferred")] {
        let metadata: String = sqlx::query_scalar(
            "SELECT metadata FROM runner_events WHERE runner_id=? AND event='Runner deleted'",
        )
        .bind(runner)
        .fetch_one(&h.database)
        .await?;
        let metadata: Value = serde_json::from_str(&metadata)?;
        assert_eq!(metadata["gitlabCleanup"], expected, "{runner}: {metadata}");
    }
    Ok(())
}

/// #141: the runners list carries what the GitLab runner link
/// (`<baseUrl>/<scopePath>/-/runners/<id>`) is built from: the runner's
/// GitLab id and its connection's address and scope path, and the three
/// fields as explicit nulls for a GitHub runner.
#[tokio::test]
async fn the_runners_list_carries_a_gitlab_runners_link_fields_and_nulls_for_a_github_runner()
-> Result<()> {
    let h = harness().await?;
    h.seed_connection(CONNECTION_1, GITLAB, group(7, "acme/platform")?)
        .await?;
    h.seed_pool(GITLAB_POOL, "gitlab-pool", "gitlab", &[CONNECTION_1])
        .await?;
    h.seed_runner("runner-gitlab", GITLAB_POOL, Some(CONNECTION_1), Some(77))
        .await?;
    h.seed_pool(ORG_POOL, "org-pool", "organization", &[])
        .await?;
    sqlx::query("INSERT INTO runners (id,pool_id,name,provider,status,busy,ci_platform,github_runner_id,created_at,updated_at) VALUES ('runner-github',?,'runner-github','docker','idle',0,'github',5,2,2)")
        .bind(ORG_POOL)
        .execute(&h.database)
        .await?;

    let (status, body) = h.as_admin(Method::GET, "/api/v1/runners", None).await?;
    assert_eq!(status, StatusCode::OK, "{body}");
    let items = body["items"]
        .as_array()
        .context("the runners list has items")?;
    let runner = |id: &str| {
        items
            .iter()
            .find(|item| item["id"] == id)
            .cloned()
            .with_context(|| format!("{id} is listed: {body}"))
    };

    let gitlab = runner("runner-gitlab")?;
    assert_eq!(gitlab["platform"], "gitlab", "{gitlab}");
    assert_eq!(gitlab.get("gitlabRunnerId"), Some(&json!(77)), "{gitlab}");
    assert_eq!(
        gitlab.get("gitlabBaseUrl"),
        Some(&json!(GITLAB)),
        "{gitlab}"
    );
    assert_eq!(
        gitlab.get("gitlabScopePath"),
        Some(&json!("acme/platform")),
        "{gitlab}"
    );

    let github = runner("runner-github")?;
    assert_eq!(github["platform"], "github", "{github}");
    for field in ["gitlabRunnerId", "gitlabBaseUrl", "gitlabScopePath"] {
        assert_eq!(
            github.get(field),
            Some(&Value::Null),
            "{field} is present and null on a GitHub runner: {github}"
        );
    }
    Ok(())
}

/// #159: the pool list carries each pool's GitLab connections, so the list
/// row can show a GitLab pool's destination without fetching each detail.
/// They are the detail's projection in the detail's order
/// (`membership.created_at, connection.id`), grouped per pool from one load
/// over the page's pool ids, and `[]` for a pool with none.
#[tokio::test]
async fn the_pool_list_carries_each_pools_gitlab_connections_in_detail_order_and_none_for_a_github_pool()
-> Result<()> {
    const SECOND_GITLAB_POOL: &str = "c1000000-0000-4000-8000-000000000003";
    let h = harness().await?;
    h.seed_connection(CONNECTION_1, GITLAB, project(42, "acme/app")?)
        .await?;
    h.seed_connection(CONNECTION_2, OTHER_GITLAB, group(7, "acme/platform")?)
        .await?;
    h.seed_pool(GITLAB_POOL, "gitlab-pool", "gitlab", &[])
        .await?;
    h.seed_pool(SECOND_GITLAB_POOL, "gitlab-second", "gitlab", &[])
        .await?;
    h.seed_pool(ORG_POOL, "org-pool", "organization", &[])
        .await?;
    // Added second, so the detail lists CONNECTION_2 first: the order is the
    // membership's, not the connection id's.
    for (pool_id, connection_id, created_at) in [
        (GITLAB_POOL, CONNECTION_1, 20_i64),
        (GITLAB_POOL, CONNECTION_2, 10),
        (SECOND_GITLAB_POOL, CONNECTION_1, 10),
    ] {
        sqlx::query("INSERT INTO runner_pool_gitlab_connections (pool_id,connection_id,created_at) VALUES (?,?,?)")
            .bind(pool_id)
            .bind(connection_id)
            .bind(created_at)
            .execute(&h.database)
            .await?;
    }

    let (status, body) = h
        .as_admin(Method::GET, "/api/v1/runner-pools", None)
        .await?;
    assert_eq!(status, StatusCode::OK, "{body}");
    let items = body["items"]
        .as_array()
        .context("the pool list has items")?;
    let listed = |id: &str| {
        items
            .iter()
            .find(|item| item["id"] == id)
            .cloned()
            .with_context(|| format!("{id} is listed: {body}"))
    };

    let connection = |id: &str, name: String, base_url: &str, kind: &str, path: &str| json!({ "id": id, "name": name, "baseUrl": base_url, "scopeKind": kind, "scopePath": path });
    let first = connection(
        CONNECTION_1,
        format!("seeded {CONNECTION_1}"),
        GITLAB,
        "project",
        "acme/app",
    );
    let second = connection(
        CONNECTION_2,
        format!("seeded {CONNECTION_2}"),
        OTHER_GITLAB,
        "group",
        "acme/platform",
    );

    let gitlab = listed(GITLAB_POOL)?;
    assert_eq!(
        gitlab.get("gitlabConnections"),
        Some(&json!([second, first])),
        "{gitlab}"
    );
    let (status, detail) = h
        .as_admin(
            Method::GET,
            &format!("/api/v1/runner-pools/{GITLAB_POOL}"),
            None,
        )
        .await?;
    assert_eq!(status, StatusCode::OK, "{detail}");
    assert_eq!(
        gitlab.get("gitlabConnections"),
        detail.get("gitlabConnections"),
        "the list and the detail agree on names and order"
    );

    let second_pool = listed(SECOND_GITLAB_POOL)?;
    assert_eq!(
        second_pool.get("gitlabConnections"),
        Some(&json!([first])),
        "each pool gets only its own connections: {second_pool}"
    );

    let github = listed(ORG_POOL)?;
    assert_eq!(
        github.get("gitlabConnections"),
        Some(&json!([])),
        "a pool with no GitLab connection lists an empty array: {github}"
    );
    Ok(())
}
