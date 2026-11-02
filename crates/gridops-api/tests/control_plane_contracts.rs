use std::{
    env, fs,
    net::TcpListener,
    path::PathBuf,
    process::{Child, Command, Stdio},
    sync::Arc,
    time::Duration,
};

use anyhow::{Context, Result, anyhow};
use axum::{
    Json, Router,
    extract::{Path, State},
    routing::{get, post},
};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use gridops_core::{
    compatible_runner_provider, connect_database_path,
    crypto::{Vault, hash_token},
    models::CreateRunnerPool,
    runner_system_labels, scale_up_target,
};
use hmac::{Hmac, Mac as _};
use reqwest::{Client, StatusCode, header};
use rsa::pkcs1::{EncodeRsaPrivateKey as _, LineEnding};
use serde_json::{Value, json};
use sha2::Sha256;
use sqlx::{Row as _, SqlitePool};
use tokio::{sync::Mutex, time::sleep};
use uuid::Uuid;

type HmacSha256 = Hmac<Sha256>;

const SESSION_SECRET: &str = "integration-session-secret";
const WEBHOOK_SECRET: &str = "integration-webhook-secret";
const ENCRYPTION_KEY: &str = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=";

struct ApiFixture {
    child: Child,
    client: Client,
    base_url: String,
    database_path: PathBuf,
}

impl ApiFixture {
    async fn start() -> Result<Self> {
        Self::start_with(&[]).await
    }

    async fn start_with(extra_env: &[(&str, String)]) -> Result<Self> {
        let port = TcpListener::bind(("127.0.0.1", 0))?.local_addr()?.port();
        let database_path =
            env::temp_dir().join(format!("gridops-api-test-{}.sqlite", Uuid::new_v4()));
        let base_url = format!("http://127.0.0.1:{port}");
        let binary = env!("CARGO_BIN_EXE_gridops-api");
        let child = Command::new(binary)
            .env("GRIDOPS_BASE_URL", &base_url)
            .env("GRIDOPS_FLEET_ALLOW_LOOPBACK_HTTP", "true")
            .env("GRIDOPS_API_BIND", format!("127.0.0.1:{port}"))
            .env("GRIDOPS_DATABASE_PATH", &database_path)
            .env("GRIDOPS_SESSION_SECRET", SESSION_SECRET)
            .env("GRIDOPS_ENCRYPTION_KEY", ENCRYPTION_KEY)
            .env("GITHUB_CLIENT_ID", "integration-client")
            .env("GITHUB_CLIENT_SECRET", "integration-secret")
            .env("GITHUB_WEBHOOK_SECRET", WEBHOOK_SECRET)
            .env("GRIDOPS_WEBHOOK_ACTIVE", "false")
            .env("GRIDOPS_MANAGER_URL", "http://127.0.0.1:9")
            .envs(extra_env.iter().map(|(name, value)| (*name, value)))
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .context("failed to start the API fixture")?;
        let client = Client::builder().build()?;
        let fixture = Self {
            child,
            client,
            base_url,
            database_path,
        };
        fixture.wait_until_healthy().await?;
        fixture.seed_identity("member").await?;
        Ok(fixture)
    }

    async fn wait_until_healthy(&self) -> Result<()> {
        for _ in 0..60 {
            if let Ok(response) = self
                .client
                .get(format!("{}/api/health", self.base_url))
                .send()
                .await
                && response.status().is_success()
            {
                return Ok(());
            }
            sleep(Duration::from_millis(50)).await;
        }
        Err(anyhow!("API fixture did not become healthy"))
    }

    async fn database(&self) -> Result<SqlitePool> {
        connect_database_path(&self.database_path).await
    }

    async fn seed_identity(&self, role: &str) -> Result<()> {
        let database = self.database().await?;
        let now = gridops_core::now_millis();
        sqlx::query("INSERT INTO users (id,github_id,login,role,access_token,last_login_at,created_at,updated_at) VALUES ('user-1',1,'fixture',?,?, ?,?,?)")
            .bind(role)
            .bind("")
            .bind(now)
            .bind(now)
            .bind(now)
            .execute(&database)
            .await?;
        sqlx::query("INSERT INTO installations (id,account_id,account_login,account_type,target_type,repository_selection,created_at,updated_at) VALUES (42,42,'fixture','Organization','Organization','selected',?,?)")
            .bind(now)
            .bind(now)
            .execute(&database)
            .await?;
        sqlx::query("INSERT INTO user_installations (user_id,installation_id,permission,created_at) VALUES ('user-1',42,'admin',?)")
            .bind(now)
            .execute(&database)
            .await?;
        let token = "fixture-session-token";
        sqlx::query("INSERT INTO sessions (id,token_hash,user_id,expires_at,last_seen_at,created_at) VALUES ('session-1',?,?, ?,?,?)")
            .bind(hash_token(token))
            .bind("user-1")
            .bind(now + 3_600_000)
            .bind(now)
            .bind(now)
            .execute(&database)
            .await?;
        Ok(())
    }

    fn session_cookie() -> Result<String> {
        let token = "fixture-session-token";
        let mut mac = HmacSha256::new_from_slice(SESSION_SECRET.as_bytes())?;
        mac.update(token.as_bytes());
        Ok(format!(
            "gridops_session={token}.{}",
            URL_SAFE_NO_PAD.encode(mac.finalize().into_bytes())
        ))
    }

    fn webhook_signature(body: &str) -> Result<String> {
        let mut mac = HmacSha256::new_from_slice(WEBHOOK_SECRET.as_bytes())?;
        mac.update(body.as_bytes());
        Ok(format!(
            "sha256={}",
            hex::encode(mac.finalize().into_bytes())
        ))
    }

    async fn post_webhook(
        &self,
        delivery: &str,
        body: &str,
        signature: &str,
    ) -> Result<reqwest::Response> {
        self.post_webhook_event(delivery, "ping", body, signature)
            .await
    }

    async fn post_webhook_event(
        &self,
        delivery: &str,
        event: &str,
        body: &str,
        signature: &str,
    ) -> Result<reqwest::Response> {
        Ok(self
            .client
            .post(format!("{}/api/v1/webhooks/github", self.base_url))
            .header("x-github-delivery", delivery)
            .header("x-github-event", event)
            .header("x-hub-signature-256", signature)
            .body(body.to_owned())
            .send()
            .await?)
    }
}

impl Drop for ApiFixture {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = fs::remove_file(&self.database_path);
        let _ = fs::remove_file(self.database_path.with_extension("sqlite-shm"));
        let _ = fs::remove_file(self.database_path.with_extension("sqlite-wal"));
    }
}

#[tokio::test]
async fn api_routes_enforce_authentication_and_admin_boundaries() -> Result<()> {
    let fixture = ApiFixture::start().await?;

    let unauthenticated = fixture
        .client
        .get(format!("{}/api/v1/backups/database", fixture.base_url))
        .send()
        .await?;
    assert_eq!(unauthenticated.status(), StatusCode::UNAUTHORIZED);

    let member_settings = fixture
        .client
        .get(format!("{}/api/v1/backups/database", fixture.base_url))
        .header(header::COOKIE, ApiFixture::session_cookie()?)
        .send()
        .await?;
    assert_eq!(member_settings.status(), StatusCode::FORBIDDEN);
    Ok(())
}

#[tokio::test]
async fn health_is_public_and_names_the_version_and_the_build() -> Result<()> {
    let fixture = ApiFixture::start().await?;
    let health: Value = fixture
        .client
        .get(format!("{}/api/health", fixture.base_url))
        .send()
        .await?
        .json()
        .await?;
    assert_eq!(health["status"], "ok");
    assert_eq!(health["version"], env!("CARGO_PKG_VERSION"));
    assert_eq!(
        health["buildNumber"],
        json!(gridops_core::build_info::build_number()),
        "{health}"
    );
    assert_eq!(
        health["commit"],
        json!(gridops_core::build_info::commit()),
        "{health}"
    );
    Ok(())
}

#[tokio::test]
async fn installer_is_public_and_points_at_this_deployment() -> Result<()> {
    let fixture = ApiFixture::start().await?;

    for path in ["install.sh"] {
        let response = fixture
            .client
            .get(format!("{}/{path}", fixture.base_url))
            .send()
            .await?;
        assert_eq!(response.status(), StatusCode::OK, "{path}");
        let script = response.text().await?;
        assert!(
            !script.contains("__GRIDOPS_"),
            "{path} has a placeholder left"
        );
        let quoted = |value: &str| [format!("\"{value}\""), format!("'{value}'")];
        assert!(
            quoted(&fixture.base_url)
                .iter()
                .any(|v| script.contains(v.as_str())),
            "{path} doesn't carry the address"
        );
        assert!(
            quoted(env!("CARGO_PKG_VERSION"))
                .iter()
                .any(|v| script.contains(v.as_str())),
            "{path} doesn't carry the version"
        );
        // Installers pin the release version, which names a release asset; the build number never goes into it.
        assert!(!script.contains("+build"), "{path} carries a build number");
    }
    // Windows is not a deployment target.
    let windows = fixture
        .client
        .get(format!("{}/install.ps1", fixture.base_url))
        .send()
        .await?;
    assert_eq!(windows.status(), StatusCode::NOT_FOUND);
    Ok(())
}

#[tokio::test]
async fn a_queued_run_with_a_running_job_shows_as_in_progress_with_its_waiting_jobs() -> Result<()>
{
    let fixture = ApiFixture::start().await?;
    let database = fixture.database().await?;
    // GitHub still lists run 100 as queued: one job runs while two wait for a
    // runner. Run 101 finished but left a job listed as queued.
    sqlx::raw_sql(
        r#"
        INSERT INTO repositories (id,installation_id,owner,name,full_name,private,default_branch,html_url,last_synced_at,created_at,updated_at)
          VALUES (7,42,'fixture','app','fixture/app',0,'main','https://github.com/fixture/app',1,1,1);
        INSERT INTO workflow_runs (id,repository_id,workflow_name,run_number,event,status,head_sha,html_url,github_created_at,github_updated_at,created_at,updated_at)
          VALUES
          (100,7,'CI',2,'push','queued','abc','https://github.com/fixture/app/actions/runs/100',2,2,2,2),
          (101,7,'Release',1,'push','completed','abd','https://github.com/fixture/app/actions/runs/101',1,1,1,1);
        INSERT INTO workflow_jobs (id,run_id,name,status,labels,html_url,created_at,updated_at)
          VALUES
          (1000,100,'build','in_progress','["self-hosted"]','https://github.com/fixture/app/actions/runs/100/job/1000',2,2),
          (1001,100,'test','queued','["self-hosted"]','https://github.com/fixture/app/actions/runs/100/job/1001',2,2),
          (1002,100,'lint','queued','["self-hosted"]','https://github.com/fixture/app/actions/runs/100/job/1002',2,2),
          (1010,101,'publish','queued','["self-hosted"]','https://github.com/fixture/app/actions/runs/101/job/1010',1,1);
        "#,
    )
    .execute(&database)
    .await?;

    let get = |path: &'static str| {
        let request = fixture
            .client
            .get(format!("{}{path}", fixture.base_url))
            .header(
                header::COOKIE,
                ApiFixture::session_cookie().unwrap_or_default(),
            );
        async move {
            let response = request.send().await?;
            assert_eq!(response.status(), StatusCode::OK, "{path}");
            Ok::<Value, anyhow::Error>(response.json().await?)
        }
    };
    let shown = |run: &Value| (run["status"].clone(), run["waitingJobs"].clone());
    let running = (Value::from("in_progress"), Value::from(2));
    let finished = (Value::from("completed"), Value::from(0));

    let list = get("/api/v1/workflow-runs").await?;
    let by_id = |id: i64| {
        list["items"]
            .as_array()
            .and_then(|items| items.iter().find(|run| run["id"] == id))
            .cloned()
            .unwrap_or_default()
    };
    assert_eq!(shown(&by_id(100)), running);
    assert_eq!(shown(&by_id(101)), finished);
    assert_eq!(shown(&get("/api/v1/workflow-runs/100").await?), running);
    let overview = get("/api/v1/overview").await?;
    let recent = overview["runs"]
        .as_array()
        .and_then(|runs| runs.iter().find(|run| run["id"] == 100))
        .cloned()
        .unwrap_or_default();
    assert_eq!(shown(&recent), running);
    Ok(())
}

#[tokio::test]
async fn webhook_signature_idempotency_and_rejection_are_persistent() -> Result<()> {
    let fixture = ApiFixture::start().await?;
    let body = r#"{"zen":"fixture"}"#;
    let signature = ApiFixture::webhook_signature(body)?;

    let accepted = fixture.post_webhook("delivery-1", body, &signature).await?;
    assert_eq!(accepted.status(), StatusCode::ACCEPTED);
    assert_eq!(accepted.json::<Value>().await?["accepted"], true);

    let duplicate = fixture.post_webhook("delivery-1", body, &signature).await?;
    assert_eq!(duplicate.status(), StatusCode::ACCEPTED);
    assert_eq!(duplicate.json::<Value>().await?["duplicate"], true);

    let rejected = fixture
        .post_webhook("delivery-2", body, "sha256=invalid")
        .await?;
    assert_eq!(rejected.status(), StatusCode::UNAUTHORIZED);
    let database = fixture.database().await?;
    let status = sqlx::query_scalar::<_, String>(
        "SELECT status FROM webhook_deliveries WHERE id='delivery-2'",
    )
    .fetch_one(&database)
    .await?;
    assert_eq!(status, "rejected");
    let deliveries = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM webhook_deliveries WHERE id='delivery-1'",
    )
    .fetch_one(&database)
    .await?;
    assert_eq!(deliveries, 1);
    Ok(())
}

#[tokio::test]
async fn webhook_retry_requires_admin_and_reprocesses_verified_delivery() -> Result<()> {
    let fixture = ApiFixture::start().await?;
    let database = fixture.database().await?;
    let now = gridops_core::now_millis();
    sqlx::query("INSERT INTO webhook_deliveries (id,event,signature_valid,status,payload,received_at) VALUES ('retry-1','ping',1,'failed',?,?)")
        .bind(r#"{"zen":"retry"}"#)
        .bind(now)
        .execute(&database)
        .await?;

    let denied = fixture
        .client
        .post(format!(
            "{}/api/v1/webhooks/retry-1/retry",
            fixture.base_url
        ))
        .header(header::COOKIE, ApiFixture::session_cookie()?)
        .header(header::ORIGIN, &fixture.base_url)
        .send()
        .await?;
    assert_eq!(denied.status(), StatusCode::FORBIDDEN);

    sqlx::query("UPDATE users SET role='admin' WHERE id='user-1'")
        .execute(&database)
        .await?;
    let retried = fixture
        .client
        .post(format!(
            "{}/api/v1/webhooks/retry-1/retry",
            fixture.base_url
        ))
        .header(header::COOKIE, ApiFixture::session_cookie()?)
        .header(header::ORIGIN, &fixture.base_url)
        .send()
        .await?;
    assert_eq!(retried.status(), StatusCode::OK);
    assert_eq!(
        sqlx::query_scalar::<_, String>("SELECT status FROM webhook_deliveries WHERE id='retry-1'")
            .fetch_one(&database)
            .await?,
        "processed"
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM audit_events WHERE action='webhook.retried'"
        )
        .fetch_one(&database)
        .await?,
        1
    );
    Ok(())
}

/// Each run cancellation's run id and `Authorization` header.
type SeenTokens = Arc<Mutex<Vec<(i64, String)>>>;

/// Stands in for api.github.com: reports the App renamed to `gridopsbot` and
/// moved to that organization, mints installation tokens and records the
/// token each run cancellation arrives with. It refuses the user's token for
/// run 3, as GitHub does when the person can't write to the repository.
async fn fake_github() -> Result<(String, SeenTokens)> {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let app = Router::new()
        .route(
            "/app",
            get(|| async {
                Json(json!({
                    "slug": "gridopsbot",
                    "owner": { "login": "gridopsbot", "type": "Organization" },
                }))
            }),
        )
        .route(
            "/app/installations/{id}/access_tokens",
            post(|| async {
                Json(json!({
                    "token": "installation-token",
                    "expires_at": (chrono::Utc::now() + chrono::Duration::hours(1)).to_rfc3339(),
                }))
            }),
        )
        .route(
            "/repos/{owner}/{repo}/actions/runs/{run_id}/cancel",
            post(
                |State(seen): State<SeenTokens>,
                 Path((_, _, run_id)): Path<(String, String, i64)>,
                 headers: axum::http::HeaderMap| async move {
                    let token = headers
                        .get(header::AUTHORIZATION)
                        .and_then(|value| value.to_str().ok())
                        .unwrap_or_default()
                        .to_owned();
                    let refused = run_id == 3 && token == "Bearer user-token";
                    seen.lock().await.push((run_id, token));
                    if refused {
                        axum::http::StatusCode::FORBIDDEN
                    } else {
                        axum::http::StatusCode::ACCEPTED
                    }
                },
            ),
        )
        .with_state(Arc::clone(&seen));
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0)).await?;
    let url = format!("http://{}", listener.local_addr()?);
    tokio::spawn(async move { axum::serve(listener, app).await });
    Ok((url, seen))
}

fn app_private_key() -> Result<String> {
    let key = rsa::RsaPrivateKey::new(&mut rsa::rand_core::OsRng, 2048)?;
    Ok(key.to_pkcs1_pem(LineEnding::LF)?.to_string())
}

#[tokio::test]
async fn links_follow_the_github_app_to_its_new_name_and_owner() -> Result<()> {
    let (github_url, _) = fake_github().await?;
    let fixture = ApiFixture::start_with(&[
        ("GRIDOPS_GITHUB_API_URL", github_url),
        ("GITHUB_APP_ID", "1".into()),
        ("GITHUB_APP_PRIVATE_KEY", app_private_key()?),
        ("GITHUB_APP_SLUG", "gridops-self-hosted".into()),
    ])
    .await?;
    let read = |path: &str| {
        fixture
            .client
            .get(format!("{}{path}", fixture.base_url))
            .header(
                header::COOKIE,
                ApiFixture::session_cookie().unwrap_or_default(),
            )
            .send()
    };

    // The API asks GitHub for the App's name as it starts.
    let mut install_url = Value::Null;
    for _ in 0..60 {
        install_url = read("/api/v1/runner-pools/options")
            .await?
            .json::<Value>()
            .await?["installUrl"]
            .clone();
        if install_url != "https://github.com/apps/gridops-self-hosted/installations/new" {
            break;
        }
        sleep(Duration::from_millis(50)).await;
    }
    assert_eq!(
        install_url,
        "https://github.com/apps/gridopsbot/installations/new"
    );
    let settings = read("/api/v1/settings/ai").await?.json::<Value>().await?;
    assert_eq!(
        settings["github"]["appSettingsUrl"],
        "https://github.com/organizations/gridopsbot/settings/apps/gridopsbot/permissions"
    );
    Ok(())
}

#[tokio::test]
async fn run_controls_act_as_the_signed_in_user() -> Result<()> {
    let (github_url, seen) = fake_github().await?;
    let fixture = ApiFixture::start_with(&[
        ("GRIDOPS_GITHUB_API_URL", github_url),
        ("GITHUB_APP_ID", "1".into()),
        ("GITHUB_APP_PRIVATE_KEY", app_private_key()?),
    ])
    .await?;
    let database = fixture.database().await?;
    let now = gridops_core::now_millis();
    sqlx::query("INSERT INTO repositories (id,installation_id,owner,name,full_name,private,default_branch,html_url,last_synced_at,created_at,updated_at) VALUES (7,42,'fixture','app','fixture/app',0,'main','https://github.com/fixture/app',?,?,?)")
        .bind(now)
        .bind(now)
        .bind(now)
        .execute(&database)
        .await?;
    for run_id in 1..=3_i64 {
        sqlx::query("INSERT INTO workflow_runs (id,repository_id,workflow_name,run_number,event,status,head_sha,html_url,github_created_at,github_updated_at,created_at,updated_at) VALUES (?,7,'CI',?,'push','queued','abc','https://github.com/fixture/app/actions/runs',?,?,?,?)")
            .bind(run_id)
            .bind(run_id)
            .bind(now)
            .bind(now)
            .bind(now)
            .bind(now)
            .execute(&database)
            .await?;
    }
    let cancel = |run_id: i64| {
        fixture
            .client
            .post(format!(
                "{}/api/v1/workflow-runs/{run_id}/action",
                fixture.base_url
            ))
            .header(
                header::COOKIE,
                ApiFixture::session_cookie().unwrap_or_default(),
            )
            .header(header::ORIGIN, &fixture.base_url)
            .json(&json!({ "action": "cancel" }))
            .send()
    };

    // The fixture user holds no GitHub token of their own, so the App acts.
    assert_eq!(cancel(1).await?.status(), StatusCode::OK);
    let vault = Vault::new([0; 32], SESSION_SECRET.as_bytes().to_vec());
    sqlx::query("UPDATE users SET access_token=? WHERE id='user-1'")
        .bind(vault.seal("user-token")?)
        .execute(&database)
        .await?;
    assert_eq!(cancel(2).await?.status(), StatusCode::OK);
    assert_eq!(cancel(3).await?.status(), StatusCode::OK);

    assert_eq!(
        *seen.lock().await,
        [
            (1, "Bearer installation-token".to_owned()),
            (2, "Bearer user-token".to_owned()),
            (3, "Bearer user-token".to_owned()),
            (3, "Bearer installation-token".to_owned()),
        ]
    );
    let actors = sqlx::query_scalar::<_, String>(
        "SELECT json_extract(metadata,'$.githubActor') FROM audit_events WHERE action='workflow_run.cancel' ORDER BY target_id",
    )
    .fetch_all(&database)
    .await?;
    assert_eq!(actors, ["app", "user", "app"]);
    Ok(())
}

#[tokio::test]
async fn backup_route_is_authenticated_and_migrations_preserve_contract_columns() -> Result<()> {
    let fixture = ApiFixture::start().await?;
    let unauthenticated = fixture
        .client
        .get(format!("{}/api/v1/backups/database", fixture.base_url))
        .send()
        .await?;
    assert_eq!(unauthenticated.status(), StatusCode::UNAUTHORIZED);

    let database = fixture.database().await?;
    let columns = sqlx::query("SELECT name FROM pragma_table_info('runner_pools') WHERE name IN ('provider','providers','configuration_version','provision_circuit_open') ORDER BY name")
        .fetch_all(&database)
        .await?
        .into_iter()
        .map(|row| row.get::<String, _>("name"))
        .collect::<Vec<_>>();
    assert_eq!(
        columns,
        [
            "configuration_version",
            "provider",
            "providers",
            "provision_circuit_open"
        ]
    );

    sqlx::query("UPDATE users SET role='admin' WHERE id='user-1'")
        .execute(&database)
        .await?;
    let backup = fixture
        .client
        .get(format!("{}/api/v1/backups/database", fixture.base_url))
        .header(header::COOKIE, ApiFixture::session_cookie()?)
        .send()
        .await?;
    assert_eq!(backup.status(), StatusCode::OK);
    assert_eq!(
        backup.bytes().await?.get(..16),
        Some(&b"SQLite format 3\0"[..])
    );
    Ok(())
}

#[test]
fn shared_provisioning_contract_keeps_provider_labels_and_scale_targets_consistent() -> Result<()> {
    assert_eq!(scale_up_target(2, 1, 5, 1, 10), 6);
    assert_eq!(scale_up_target(2, 1, 5, 2, 10), 10);
    let docker_labels = runner_system_labels("docker")
        .into_iter()
        .map(str::to_owned)
        .collect::<Vec<_>>();
    let tart_labels = runner_system_labels("tart")
        .into_iter()
        .map(str::to_owned)
        .collect::<Vec<_>>();
    assert_eq!(
        compatible_runner_provider(
            &["docker".into(), "tart".into()],
            &docker_labels,
            &["gridops".into()]
        ),
        Some("docker".into())
    );
    assert_eq!(
        compatible_runner_provider(
            &["docker".into(), "tart".into()],
            &tart_labels,
            &["gridops".into()]
        ),
        Some("tart".into())
    );
    assert_eq!(
        compatible_runner_provider(
            &["docker".into()],
            &["runs-on:macos".into()],
            &["gridops".into()]
        ),
        None
    );

    let input = CreateRunnerPool {
        installation_id: 42,
        repository_id: None,
        repository_ids: vec![7],
        bitbucket_connection_ids: vec![],
        name: "fixture-pool".into(),
        scope: "repository".into(),
        mode: "ephemeral".into(),
        provider: "docker".into(),
        providers: vec!["docker".into(), "tart".into()],
        labels: vec!["gridops".into()],
        image: "fixture-image".into(),
        docker_image: "fixture-image".into(),
        tart_image: "fixture-tart".into(),
        macos_runtime: "vm".into(),
        desired_count: 2,
        min_count: 0,
        max_count: 4,
        autoscaling_enabled: true,
        queue_scale_factor: 1,
        idle_timeout_minutes: 5,
        cpu_limit: 2.0,
        memory_limit_mb: 2_048,
        runner_group_id: 1,
    };
    input.validate().map_err(|error| anyhow!(error))?;
    Ok(())
}

#[tokio::test]
async fn a_workflow_job_webhook_syncs_an_org_scope_repository_instead_of_rejecting_it() -> Result<()>
{
    // `fixture/unsynced-app` is never seeded into `repositories`: no pool has ever selected it
    // individually (an org-scope pool has no repository picker to do that), and GitHub never
    // sends `installation_repositories` for an "all repositories" install. Before this fix, every
    // `workflow_job`/`workflow_run` webhook for such a repo failed forever with "Repository ...
    // is not synced yet", because the handler only checked for the row instead of creating it
    // from the payload it already received.
    let fixture = ApiFixture::start().await?;
    let database = fixture.database().await?;

    let body = json!({
        "action": "queued",
        "installation": { "id": 42 },
        "repository": {
            "id": 99,
            "name": "unsynced-app",
            "full_name": "fixture/unsynced-app",
            "private": false,
            "archived": false,
            "default_branch": "main",
            "html_url": "https://github.com/fixture/unsynced-app",
        },
        "workflow_job": {
            "id": 5000,
            "run_id": 500,
            "run_attempt": 1,
            "workflow_name": "CI",
            "head_branch": "main",
            "head_sha": "abc123",
            "name": "build",
            "status": "queued",
            "labels": ["self-hosted-linux"],
            "html_url": "https://github.com/fixture/unsynced-app/actions/runs/500/job/5000",
            "created_at": "2026-10-25T00:00:00Z",
        },
    })
    .to_string();
    let signature = ApiFixture::webhook_signature(&body)?;

    let response = fixture
        .post_webhook_event("delivery-unsynced", "workflow_job", &body, &signature)
        .await?;
    assert_eq!(response.status(), StatusCode::ACCEPTED);

    let delivery_status = sqlx::query_scalar::<_, String>(
        "SELECT status FROM webhook_deliveries WHERE id='delivery-unsynced'",
    )
    .fetch_one(&database)
    .await?;
    assert_eq!(delivery_status, "processed");

    let full_name =
        sqlx::query_scalar::<_, String>("SELECT full_name FROM repositories WHERE id=99")
            .fetch_one(&database)
            .await?;
    assert_eq!(full_name, "fixture/unsynced-app");

    let job_status =
        sqlx::query_scalar::<_, String>("SELECT status FROM workflow_jobs WHERE id=5000")
            .fetch_one(&database)
            .await?;
    assert_eq!(job_status, "queued");

    Ok(())
}

const DAY_MS: i64 = 24 * 60 * 60 * 1_000;

/// Inserts a repository under `installation_id`, plus one completed workflow
/// run per `(id, conclusion, completed_days_ago)` entry.
async fn seed_completed_runs(
    database: &SqlitePool,
    repository_id: i64,
    installation_id: i64,
    runs: &[(i64, &str, i64)],
) -> Result<()> {
    let now = gridops_core::now_millis();
    sqlx::query("INSERT INTO repositories (id,installation_id,owner,name,full_name,private,default_branch,html_url,last_synced_at,created_at,updated_at) VALUES (?,?,'fixture',?,?,0,'main','https://github.com/fixture/app',?,?,?)")
        .bind(repository_id)
        .bind(installation_id)
        .bind(format!("app-{repository_id}"))
        .bind(format!("fixture/app-{repository_id}"))
        .bind(now)
        .bind(now)
        .bind(now)
        .execute(database)
        .await?;
    let statements = runs
        .iter()
        .map(|(id, conclusion, completed_ago_days)| {
            format!(
                "INSERT INTO workflow_runs (id,repository_id,workflow_name,run_number,event,status,conclusion,head_sha,html_url,github_created_at,github_updated_at,created_at,updated_at,completed_at) VALUES ({id},{repository_id},'CI',{id},'push','completed','{conclusion}','abc','u',{now},{now},{now},{now},{completed});",
                completed = now - completed_ago_days * DAY_MS - 60_000,
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    sqlx::raw_sql(sqlx::AssertSqlSafe(statements))
        .execute(database)
        .await?;
    Ok(())
}

async fn overview_metrics(fixture: &ApiFixture) -> Result<Value> {
    let response = fixture
        .client
        .get(format!("{}/api/v1/overview", fixture.base_url))
        .header(header::COOKIE, ApiFixture::session_cookie()?)
        .send()
        .await?;
    assert_eq!(response.status(), StatusCode::OK);
    let body: Value = response.json().await?;
    Ok(body["metrics"].clone())
}

#[tokio::test]
async fn success_rate_ignores_skipped_cancelled_and_other_outcomes() -> Result<()> {
    let fixture = ApiFixture::start().await?;
    let database = fixture.database().await?;
    // Counted: 2 success, 1 failure. Everything else is not a verdict on the
    // runners or the workflows and must not move the number.
    seed_completed_runs(
        &database,
        7,
        42,
        &[
            (100, "success", 1),
            (101, "success", 2),
            (102, "failure", 3),
            (103, "skipped", 1),
            (104, "skipped", 2),
            (105, "cancelled", 1),
            (106, "cancelled", 4),
            (107, "action_required", 1),
            (108, "neutral", 1),
            (109, "stale", 1),
        ],
    )
    .await?;
    let metrics = overview_metrics(&fixture).await?;
    assert_eq!(metrics["successRate"], Value::from(66.7));
    assert_eq!(metrics["successRateRuns"], Value::from(3));
    assert_eq!(metrics["successRateWindowDays"], Value::from(7));
    Ok(())
}

#[tokio::test]
async fn success_rate_counts_failure_startup_failure_and_timed_out_against_it() -> Result<()> {
    let fixture = ApiFixture::start().await?;
    let database = fixture.database().await?;
    seed_completed_runs(
        &database,
        7,
        42,
        &[
            (100, "success", 1),
            (101, "failure", 1),
            (102, "startup_failure", 1),
            (103, "timed_out", 1),
        ],
    )
    .await?;
    let metrics = overview_metrics(&fixture).await?;
    assert_eq!(metrics["successRate"], Value::from(25.0));
    assert_eq!(metrics["successRateRuns"], Value::from(4));
    Ok(())
}

#[tokio::test]
async fn success_rate_window_is_seven_days_of_completed_at() -> Result<()> {
    let fixture = ApiFixture::start().await?;
    let database = fixture.database().await?;
    // 6 days old is in, 8 days old is out. The old successes would lift
    // the rate to 80% if the window leaked.
    seed_completed_runs(
        &database,
        7,
        42,
        &[
            (100, "success", 6),
            (101, "failure", 6),
            (102, "success", 8),
            (103, "success", 8),
            (104, "success", 8),
            (105, "success", 200),
        ],
    )
    .await?;
    let metrics = overview_metrics(&fixture).await?;
    assert_eq!(metrics["successRate"], Value::from(50.0));
    assert_eq!(metrics["successRateRuns"], Value::from(2));
    assert_eq!(metrics["successRateWindowDays"], Value::from(7));
    Ok(())
}

#[tokio::test]
async fn success_rate_is_null_when_no_runs_count() -> Result<()> {
    let fixture = ApiFixture::start().await?;
    let database = fixture.database().await?;
    // Only skipped, cancelled and out-of-window runs: nothing to report.
    seed_completed_runs(
        &database,
        7,
        42,
        &[
            (100, "skipped", 1),
            (101, "cancelled", 1),
            (102, "success", 45),
            (103, "failure", 45),
        ],
    )
    .await?;
    let metrics = overview_metrics(&fixture).await?;
    assert_eq!(metrics["successRate"], Value::Null);
    assert_eq!(metrics["successRateRuns"], Value::from(0));
    assert_eq!(metrics["successRateWindowDays"], Value::from(7));
    Ok(())
}

#[tokio::test]
async fn success_rate_only_counts_installations_the_user_can_access() -> Result<()> {
    let fixture = ApiFixture::start().await?;
    let database = fixture.database().await?;
    let now = gridops_core::now_millis();
    sqlx::query("INSERT INTO installations (id,account_id,account_login,account_type,target_type,repository_selection,created_at,updated_at) VALUES (43,43,'other','Organization','Organization','selected',?,?)")
        .bind(now)
        .bind(now)
        .execute(&database)
        .await?;
    seed_completed_runs(
        &database,
        7,
        42,
        &[(100, "success", 1), (101, "failure", 1)],
    )
    .await?;
    // The signed-in user has no access to installation 43; its runs must not
    // appear in either the rate or the run count.
    seed_completed_runs(
        &database,
        8,
        43,
        &[
            (200, "failure", 1),
            (201, "failure", 1),
            (202, "failure", 1),
        ],
    )
    .await?;
    let metrics = overview_metrics(&fixture).await?;
    assert_eq!(metrics["successRate"], Value::from(50.0));
    assert_eq!(metrics["successRateRuns"], Value::from(2));
    Ok(())
}

#[tokio::test]
async fn signed_out_overview_names_the_success_rate_window() -> Result<()> {
    let fixture = ApiFixture::start().await?;
    let response = fixture
        .client
        .get(format!("{}/api/v1/overview", fixture.base_url))
        .send()
        .await?;
    assert_eq!(response.status(), StatusCode::OK);
    let body: Value = response.json().await?;
    assert_eq!(body["metrics"]["successRate"], Value::Null);
    assert_eq!(body["metrics"]["successRateRuns"], Value::from(0));
    assert_eq!(body["metrics"]["successRateWindowDays"], Value::from(7));
    Ok(())
}

/// Minutes before now, in epoch milliseconds.
fn minutes_ago(minutes: i64) -> i64 {
    gridops_core::now_millis() - minutes * 60_000
}

/// An organization pool on installation 42 carrying one label.
fn queued_jobs_pool(id: &str, label: &str, created_at: i64, max_count: i64) -> String {
    format!(
        "INSERT INTO runner_pools (id,installation_id,repository_id,name,scope,labels,image,max_count,autoscaling_enabled,created_at,updated_at)
          VALUES ('{id}',42,NULL,'{id}','organization','[\"{label}\"]','runner:latest',{max_count},1,{created_at},{created_at});"
    )
}

fn queued_jobs_runner(pool: &str, status: &str, busy: bool) -> String {
    format!(
        "INSERT INTO runners (id,pool_id,name,status,busy,created_at,updated_at)
          VALUES ('{pool}-{status}','{pool}','{pool}-{status}','{status}',{},1,1);",
        i64::from(busy)
    )
}

fn queued_job(id: i64, run: i64, name: &str, labels: &str, queued_minutes_ago: i64) -> String {
    let at = minutes_ago(queued_minutes_ago);
    format!(
        "INSERT INTO workflow_jobs (id,run_id,name,status,labels,html_url,created_at,updated_at)
          VALUES ({id},{run},'{name}','queued','{labels}','https://github.com/fixture/app/actions/runs/{run}/job/{id}',{at},{at});"
    )
}

async fn queued_jobs_page(fixture: &ApiFixture, query: &str) -> Result<Value> {
    let response = fixture
        .client
        .get(format!(
            "{}/api/v1/overview/queued-jobs{query}",
            fixture.base_url
        ))
        .header(header::COOKIE, ApiFixture::session_cookie()?)
        .send()
        .await?;
    assert_eq!(response.status(), StatusCode::OK, "{query}");
    Ok(response.json().await?)
}

fn ids(page: &Value) -> Vec<i64> {
    page["items"]
        .as_array()
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item["id"].as_i64())
                .collect()
        })
        .unwrap_or_default()
}

/// Repository 7 is the viewer's. Run 100 is unfinished, run 101 finished with a
/// job still listed as queued, run 102 belongs to an installation the viewer
/// can't see.
const QUEUED_JOBS_BASE: &str = r#"
    INSERT INTO installations (id,account_id,account_login,account_type,target_type,repository_selection,created_at,updated_at)
      VALUES (43,43,'stranger','Organization','Organization','selected',1,1);
    INSERT INTO repositories (id,installation_id,owner,name,full_name,private,default_branch,html_url,last_synced_at,created_at,updated_at)
      VALUES
      (7,42,'fixture','app','fixture/app',0,'main','https://github.com/fixture/app',1,1,1),
      (8,43,'stranger','secret','stranger/secret',0,'main','https://github.com/stranger/secret',1,1,1);
    INSERT INTO workflow_runs (id,repository_id,workflow_name,run_number,event,status,head_sha,html_url,github_created_at,github_updated_at,created_at,updated_at)
      VALUES
      (100,7,'CI',2,'push','queued','abc','https://github.com/fixture/app/actions/runs/100',2,2,2,2),
      (101,7,'Release',1,'push','completed','abd','https://github.com/fixture/app/actions/runs/101',1,1,1,1),
      (102,8,'Other',1,'push','queued','abe','https://github.com/stranger/secret/actions/runs/102',1,1,1,1);
"#;

#[tokio::test]
async fn queued_jobs_are_listed_with_the_pool_the_scaler_owns_them_for() -> Result<()> {
    let fixture = ApiFixture::start().await?;
    let database = fixture.database().await?;
    // 'full' (older) is at its limit, so jobs for 'shared' pass to 'spare'.
    // 'lane' has a runner starting, 'quiet' has room and nothing running, and
    // 'tight' has one pool at its limit with nowhere else to send its job.
    let pools = [
        queued_jobs_pool("full", "shared", 1, 1),
        queued_jobs_pool("spare", "shared", 2, 5),
        queued_jobs_pool("lane", "lane", 3, 5),
        queued_jobs_pool("quiet", "quiet", 4, 5),
        queued_jobs_pool("tight", "tight", 5, 1),
        queued_jobs_runner("full", "busy", true),
        queued_jobs_runner("lane", "starting", false),
        queued_jobs_runner("tight", "busy", true),
    ]
    .concat();
    let jobs = [
        queued_job(1, 100, "on-spare", r#"["self-hosted","shared"]"#, 120),
        queued_job(2, 100, "no-pool", r#"["self-hosted","gridops-ci"]"#, 90),
        queued_job(3, 100, "starting", r#"["self-hosted","lane"]"#, 60),
        queued_job(4, 100, "idle-room", r#"["self-hosted","quiet"]"#, 30),
        queued_job(5, 100, "stuck", r#"["self-hosted","tight"]"#, 10),
        // None of these count: a finished run, another installation's job,
        // and a job that is already running.
        queued_job(6, 101, "orphan", r#"["self-hosted","shared"]"#, 600),
        queued_job(7, 102, "foreign", r#"["self-hosted","shared"]"#, 600),
        "INSERT INTO workflow_jobs (id,run_id,name,status,labels,html_url,created_at,updated_at)
          VALUES (8,100,'running','in_progress','[\"self-hosted\"]','u',1,1);"
            .to_owned(),
    ]
    .concat();
    sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
        "{QUEUED_JOBS_BASE}{pools}{jobs}"
    )))
    .execute(&database)
    .await?;

    let page = queued_jobs_page(&fixture, "").await?;
    assert_eq!(page["authenticated"], true);
    // Longest-waiting first.
    assert_eq!(ids(&page), vec![1, 2, 3, 4, 5]);
    assert_eq!(page["total"], 5);
    let item = |id: i64| {
        page["items"]
            .as_array()
            .and_then(|items| items.iter().find(|item| item["id"] == id))
            .cloned()
            .unwrap_or_default()
    };
    let pool_and_reason = |id: i64| {
        let item = item(id);
        (
            item["matchedPool"]["id"].as_str().map(str::to_owned),
            item["waitingReason"]
                .as_str()
                .unwrap_or_default()
                .to_owned(),
        )
    };
    let expected = |pool: Option<&str>, reason: &str| (pool.map(str::to_owned), reason.to_owned());
    // Owned by 'spare' because 'full' has no room left.
    assert_eq!(
        pool_and_reason(1),
        expected(Some("spare"), "waiting_for_runner")
    );
    // No pool offers the label: listed, with no pool.
    assert_eq!(pool_and_reason(2), expected(None, "no_matching_pool"));
    assert!(item(2)["matchedPool"].is_null());
    assert_eq!(
        pool_and_reason(3),
        expected(Some("lane"), "runner_starting")
    );
    assert_eq!(
        pool_and_reason(4),
        expected(Some("quiet"), "waiting_for_runner")
    );
    // 'tight' has a busy runner at its maximum of one: no room for job 5.
    assert_eq!(
        pool_and_reason(5),
        expected(Some("tight"), "pool_at_capacity")
    );

    let first = item(1);
    assert_eq!(first["name"], "on-spare");
    assert_eq!(first["repository"], "fixture/app");
    assert_eq!(first["workflow"], "CI");
    assert_eq!(first["runId"], 100);
    assert_eq!(
        first["runUrl"],
        "https://github.com/fixture/app/actions/runs/100"
    );
    assert_eq!(first["labels"], json!(["self-hosted", "shared"]));
    assert_eq!(first["matchedPool"]["name"], "spare");
    let age = first["ageSeconds"].as_i64().unwrap_or_default();
    assert!((7_200..7_260).contains(&age), "age {age}");

    // The headline count and the list use one filter, so they agree.
    let overview = fixture
        .client
        .get(format!("{}/api/v1/overview", fixture.base_url))
        .header(header::COOKIE, ApiFixture::session_cookie()?)
        .send()
        .await?
        .json::<Value>()
        .await?;
    assert_eq!(overview["metrics"]["queuedJobs"], page["total"]);
    assert_eq!(overview["metrics"]["queuedJobs"], 5);
    assert_eq!(
        overview["slo"]["alerts"][0]["href"], "/queued-jobs",
        "the waiting alert opens this list"
    );
    Ok(())
}

#[tokio::test]
async fn queued_jobs_page_through_and_belong_to_the_viewers_installations() -> Result<()> {
    let fixture = ApiFixture::start().await?;
    let database = fixture.database().await?;
    let jobs = (1..=5)
        .map(|id| {
            queued_job(
                id,
                100,
                &format!("job-{id}"),
                r#"["self-hosted"]"#,
                100 - id * 10,
            )
        })
        .collect::<String>();
    sqlx::raw_sql(sqlx::AssertSqlSafe(format!("{QUEUED_JOBS_BASE}{jobs}")))
        .execute(&database)
        .await?;

    let first = queued_jobs_page(&fixture, "?perPage=2").await?;
    assert_eq!(
        (ids(&first), first["total"].clone()),
        (vec![1, 2], json!(5))
    );
    assert_eq!(
        (first["page"].clone(), first["perPage"].clone()),
        (json!(1), json!(2))
    );
    let second = queued_jobs_page(&fixture, "?perPage=2&page=2").await?;
    assert_eq!(ids(&second), vec![3, 4]);
    let last = queued_jobs_page(&fixture, "?perPage=2&page=3").await?;
    assert_eq!(ids(&last), vec![5]);
    // A page past the end shows the last one.
    let beyond = queued_jobs_page(&fixture, "?perPage=2&page=99").await?;
    assert_eq!((ids(&beyond), beyond["page"].clone()), (vec![5], json!(3)));

    // Without access to the installation the jobs are not listed or counted.
    sqlx::query("DELETE FROM user_installations WHERE user_id='user-1'")
        .execute(&database)
        .await?;
    let hidden = queued_jobs_page(&fixture, "").await?;
    assert_eq!(
        (hidden["total"].clone(), ids(&hidden)),
        (json!(0), Vec::new())
    );
    let overview = fixture
        .client
        .get(format!("{}/api/v1/overview", fixture.base_url))
        .header(header::COOKIE, ApiFixture::session_cookie()?)
        .send()
        .await?
        .json::<Value>()
        .await?;
    assert_eq!(overview["metrics"]["queuedJobs"], 0);
    Ok(())
}

#[tokio::test]
async fn queued_jobs_need_a_session_and_an_empty_queue_lists_nothing() -> Result<()> {
    let fixture = ApiFixture::start().await?;

    let signed_out = fixture
        .client
        .get(format!("{}/api/v1/overview/queued-jobs", fixture.base_url))
        .send()
        .await?;
    assert_eq!(signed_out.status(), StatusCode::OK);
    let body: Value = signed_out.json().await?;
    assert_eq!(body["authenticated"], false);
    assert_eq!(
        (body["items"].clone(), body["total"].clone()),
        (json!([]), json!(0))
    );

    // A cookie that doesn't verify is not a session.
    let forged = fixture
        .client
        .get(format!("{}/api/v1/overview/queued-jobs", fixture.base_url))
        .header(header::COOKIE, "gridops_session=forged.token")
        .send()
        .await?;
    assert_eq!(forged.json::<Value>().await?["authenticated"], false);

    let empty = queued_jobs_page(&fixture, "").await?;
    assert_eq!(empty["authenticated"], true);
    assert_eq!(
        (empty["items"].clone(), empty["total"].clone()),
        (json!([]), json!(0))
    );
    Ok(())
}

#[tokio::test]
async fn a_queued_jobs_pool_the_viewer_cannot_fully_see_is_not_named() -> Result<()> {
    let fixture = ApiFixture::start().await?;
    let database = fixture.database().await?;
    // 'mixed' serves repository 7 and also repository 8, which belongs to an
    // installation the viewer can't see.
    let setup = format!(
        "{QUEUED_JOBS_BASE}{}{}",
        "INSERT INTO runner_pools (id,installation_id,repository_id,name,scope,labels,image,max_count,autoscaling_enabled,created_at,updated_at)
          VALUES ('mixed',42,7,'mixed','repository','[\"mixed\"]','runner:latest',5,1,1,1);
         INSERT INTO runner_pool_repositories (pool_id,repository_id,created_at) VALUES ('mixed',7,1),('mixed',8,1);",
        queued_job(1, 100, "build", r#"["self-hosted","mixed"]"#, 5),
    );
    sqlx::raw_sql(sqlx::AssertSqlSafe(setup))
        .execute(&database)
        .await?;

    let page = queued_jobs_page(&fixture, "").await?;
    assert_eq!(ids(&page), vec![1]);
    assert!(page["items"][0]["matchedPool"].is_null());
    assert_eq!(page["items"][0]["waitingReason"], "pool_not_visible");
    Ok(())
}

/// What the Overview tiles are built from; see [`partitions`].
async fn overview_of(fixture: &ApiFixture) -> Result<Value> {
    let response = fixture
        .client
        .get(format!("{}/api/v1/overview", fixture.base_url))
        .header(header::COOKIE, ApiFixture::session_cookie()?)
        .send()
        .await?;
    assert_eq!(response.status(), StatusCode::OK);
    Ok(response.json().await?)
}

fn count(value: &Value) -> i64 {
    value.as_i64().unwrap_or(-1_000_000)
}

/// The invariants the Overview tiles hold for any data: the runner states
/// add up to the Runners headline, waiting plus manual plus unmatched add up
/// to the Queued headline, and the pool rows plus the unmatched line add up to
/// it too. A job only a manual-mode pool can run is in that pool's row, as the
/// Queued jobs list names that pool for it.
fn assert_tiles_partition(body: &Value) {
    let metrics = &body["metrics"];
    assert_eq!(
        count(&metrics["runners"]),
        count(&metrics["busy"])
            + count(&metrics["idle"])
            + count(&metrics["starting"])
            + count(&metrics["unavailable"]),
        "runners = busy + idle + starting + unavailable: {metrics}"
    );
    assert_eq!(
        count(&metrics["online"]),
        count(&metrics["busy"]) + count(&metrics["idle"]),
        "online keeps meaning busy + idle: {metrics}"
    );
    assert_eq!(
        count(&metrics["queuedJobs"]),
        count(&metrics["queuedWaiting"])
            + count(&metrics["queuedManual"])
            + count(&metrics["queuedUnmatched"]),
        "queued = waiting + manual + unmatched: {metrics}"
    );
    let in_pool_rows: i64 = body["pools"]
        .as_array()
        .map(|pools| pools.iter().map(|pool| count(&pool["queue"])).sum())
        .unwrap_or(-1_000_000);
    assert_eq!(
        in_pool_rows + count(&body["queue"]["otherPools"]),
        count(&metrics["queuedWaiting"]) + count(&metrics["queuedManual"]),
        "pool rows hold every job some pool can run: {body}"
    );
    assert_eq!(
        count(&body["queue"]["unmatched"]),
        count(&metrics["queuedUnmatched"]),
        "the unmatched line is the unmatched hint"
    );
    assert_eq!(
        in_pool_rows + count(&body["queue"]["otherPools"]) + count(&body["queue"]["unmatched"]),
        count(&metrics["queuedJobs"]),
        "served + unmatched = the Queued tile"
    );
}

const DASHBOARD_REPOSITORIES: &str = r#"
    INSERT INTO repositories (id,installation_id,owner,name,full_name,private,default_branch,html_url,last_synced_at,created_at,updated_at)
      VALUES (7,42,'fixture','app','fixture/app',0,'main','https://github.com/fixture/app',1,1,1);
"#;

/// An organization-scoped Docker pool of installation 42.
fn org_pool(id: &str, labels: &str, extra_columns: &str) -> String {
    format!(
        "INSERT INTO runner_pools (id,installation_id,name,scope,provider,providers,labels,image,docker_image,tart_image,autoscaling_enabled,max_count,created_at,updated_at{columns_header})
           VALUES ('{id}',42,'{id}','organization','docker','[\"docker\"]','{labels}','runner:latest','runner:latest','base',1,10,1,1{values});",
        columns_header = if extra_columns.is_empty() { "" } else { ",paused,provision_circuit_open" },
        values = if extra_columns.is_empty() { String::new() } else { format!(",{extra_columns}") },
    )
}

fn runner(id: &str, pool: &str, status: &str, busy: i64, deleted: bool) -> String {
    format!(
        "INSERT INTO runners (id,pool_id,name,provider,status,busy,deleted_at,created_at,updated_at)
           VALUES ('{id}','{pool}','{id}','docker','{status}',{busy},{deleted},1,1);",
        deleted = if deleted { "5" } else { "NULL" },
    )
}

fn job(id: i64, run: i64, status: &str, labels: &str, created_at: i64) -> String {
    format!(
        "INSERT INTO workflow_jobs (id,run_id,name,status,labels,html_url,created_at,updated_at)
           VALUES ({id},{run},'job-{id}','{status}','{labels}','https://github.com/fixture/app/actions/runs/{run}/job/{id}',{created_at},{created_at});"
    )
}

const QUEUED_RUN: &str = "INSERT INTO workflow_runs (id,repository_id,workflow_name,run_number,event,status,head_sha,html_url,github_created_at,github_updated_at,created_at,updated_at)
  VALUES (100,7,'CI',1,'push','queued','abc','https://github.com/fixture/app/actions/runs/100',2,2,2,2);";

async fn seed(database: &SqlitePool, statements: &[String]) -> Result<()> {
    let sql = statements.join("\n");
    sqlx::raw_sql(sqlx::AssertSqlSafe(sql))
        .execute(database)
        .await?;
    Ok(())
}

#[tokio::test]
async fn overview_tiles_split_a_live_shaped_fleet_into_parts_that_add_up() -> Result<()> {
    let fixture = ApiFixture::start().await?;
    let database = fixture.database().await?;
    // Nine runners that count: four busy, one idle, one starting, and three
    // that cannot take a job. A tenth was deleted and counts nowhere.
    // Eight queued jobs: four ask for a label no pool has, four for the
    // platform label every Linux runner takes.
    let mut statements = vec![
        DASHBOARD_REPOSITORIES.to_owned(),
        org_pool("linux", "[\"linux-pool\"]", ""),
        QUEUED_RUN.to_owned(),
    ];
    for index in 0..4 {
        statements.push(runner(&format!("busy-{index}"), "linux", "busy", 1, false));
    }
    statements.push(runner("idle", "linux", "online", 0, false));
    statements.push(runner("starting", "linux", "starting", 0, false));
    statements.push(runner("paused", "linux", "paused", 0, false));
    statements.push(runner("stopped", "linux", "stopped", 0, false));
    statements.push(runner("failed", "linux", "failed", 0, false));
    statements.push(runner("gone", "linux", "deleted", 0, true));
    for index in 0..4 {
        statements.push(job(
            1000 + index,
            100,
            "queued",
            "[\"self-hosted\",\"linux\",\"gridops-ci\"]",
            10 + index,
        ));
        statements.push(job(
            2000 + index,
            100,
            "queued",
            "[\"self-hosted-linux\"]",
            100 + index,
        ));
    }
    seed(&database, &statements).await?;

    let body = overview_of(&fixture).await?;
    assert_tiles_partition(&body);
    let metrics = &body["metrics"];
    assert_eq!(metrics["runners"], 9);
    assert_eq!(metrics["busy"], 4);
    assert_eq!(metrics["idle"], 1);
    assert_eq!(metrics["starting"], 1);
    assert_eq!(metrics["unavailable"], 3);
    assert_eq!(metrics["online"], 5);
    assert_eq!(metrics["queuedJobs"], 8);
    assert_eq!(metrics["queuedWaiting"], 4);
    assert_eq!(metrics["queuedUnmatched"], 4);
    assert_eq!(body["pools"][0]["queue"], 4);
    assert_eq!(body["queue"]["unmatched"], 4);
    assert_eq!(
        body["queue"]["missingLabels"],
        json!([{ "labels": ["gridops-ci"], "jobs": 4 }])
    );
    let alerts = body["slo"]["alerts"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let titles = alerts
        .iter()
        .map(|alert| alert["title"].as_str().unwrap_or_default().to_owned())
        .collect::<Vec<_>>();
    assert!(
        titles.contains(&"4 jobs need `gridops-ci`; no pool has it".to_owned()),
        "{titles:?}"
    );
    let unmatched = alerts
        .iter()
        .find(|alert| {
            alert["title"]
                .as_str()
                .is_some_and(|title| title.contains("gridops-ci"))
        })
        .cloned()
        .unwrap_or_default();
    assert_eq!(unmatched["level"], "error");
    assert_eq!(unmatched["href"], "/queued-jobs");

    // The Queued jobs page splits the same way: as many rows as the headline,
    // of which as many have no matching pool as the hint says.
    let list: Value = fixture
        .client
        .get(format!(
            "{}/api/v1/overview/queued-jobs?perPage=100",
            fixture.base_url
        ))
        .header(header::COOKIE, ApiFixture::session_cookie()?)
        .send()
        .await?
        .json()
        .await?;
    let rows = list["items"].as_array().cloned().unwrap_or_default();
    assert_eq!(count(&list["total"]), count(&metrics["queuedJobs"]));
    assert_eq!(
        count(&Value::from(rows.len())),
        count(&metrics["queuedJobs"])
    );
    let without_pool = rows
        .iter()
        .filter(|row| row["waitingReason"] == "no_matching_pool")
        .count();
    assert_eq!(
        count(&Value::from(without_pool)),
        count(&metrics["queuedUnmatched"])
    );
    Ok(())
}

#[tokio::test]
async fn a_job_asking_for_a_platform_no_pool_runs_is_flagged_with_that_label() -> Result<()> {
    // Issue #32: a job asking for `self-hosted-macos` when only a Linux pool
    // exists can never run. It is named with the label it is missing, in the
    // Overview and in the queued-jobs list, and no pool scales for it.
    let fixture = ApiFixture::start().await?;
    let database = fixture.database().await?;
    seed(
        &database,
        &[
            DASHBOARD_REPOSITORIES.to_owned(),
            org_pool("linux", "[\"linux-pool\"]", ""),
            // Carries `paused-only`, but is paused, so it serves nothing.
            org_pool("stopped", "[\"paused-only\"]", "1,0"),
            QUEUED_RUN.to_owned(),
            job(
                3001,
                100,
                "queued",
                "[\"self-hosted\",\"self-hosted-macos\"]",
                10,
            ),
            job(3002, 100, "queued", "[\"self-hosted\",\"Windows\"]", 20),
            job(
                3003,
                100,
                "queued",
                "[\"self-hosted\",\"linux\",\"linux-pool\"]",
                30,
            ),
            job(3004, 100, "queued", "[\"self-hosted\",\"paused-only\"]", 40),
        ],
    )
    .await?;

    let body = overview_of(&fixture).await?;
    assert_tiles_partition(&body);
    assert_eq!(body["metrics"]["queuedJobs"], 4);
    assert_eq!(body["metrics"]["queuedWaiting"], 1);
    assert_eq!(body["metrics"]["queuedUnmatched"], 3);
    assert_eq!(
        body["queue"]["missingLabels"],
        // Ties are ordered by label, upper case first.
        json!([
            { "labels": ["Windows"], "jobs": 1 },
            { "labels": ["self-hosted-macos"], "jobs": 1 },
        ])
    );
    let titles = body["slo"]["alerts"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .iter()
        .map(|alert| alert["title"].as_str().unwrap_or_default().to_owned())
        .collect::<Vec<_>>();
    for expected in [
        "1 job needs `self-hosted-macos`; no pool has it",
        "1 job needs `Windows`; no pool has it",
        // The paused pool's label exists, so that job only has no pool free.
        "1 job has no available pool",
    ] {
        assert!(titles.contains(&expected.to_owned()), "{titles:?}");
    }

    let list = queued_jobs_page(&fixture, "?perPage=100").await?;
    let item = |id: i64| {
        list["items"]
            .as_array()
            .and_then(|items| items.iter().find(|item| item["id"] == id))
            .cloned()
            .unwrap_or_default()
    };
    assert_eq!(item(3001)["waitingReason"], "no_matching_pool");
    assert_eq!(item(3001)["missingLabels"], json!(["self-hosted-macos"]));
    assert_eq!(item(3002)["waitingReason"], "no_matching_pool");
    assert_eq!(item(3002)["missingLabels"], json!(["Windows"]));
    // A job a pool serves is missing nothing.
    assert_eq!(item(3003)["matchedPool"]["id"], "linux");
    assert_eq!(item(3003)["missingLabels"], json!([]));
    // No pool free, but no label missing either.
    assert_eq!(item(3004)["waitingReason"], "no_matching_pool");
    assert_eq!(item(3004)["missingLabels"], json!([]));

    // Autoscaling ignores the unmatched jobs: the Linux pool counts only the
    // one job it can run, and the paused pool counts none.
    assert_eq!(
        gridops_core::assigned_queued_jobs(&database, "linux").await?,
        1
    );
    assert_eq!(
        gridops_core::assigned_queued_jobs(&database, "stopped").await?,
        0
    );
    Ok(())
}

#[tokio::test]
async fn busy_follows_the_runner_status_not_the_busy_flag() -> Result<()> {
    let fixture = ApiFixture::start().await?;
    let database = fixture.database().await?;
    seed(
        &database,
        &[
            org_pool("linux", "[]", ""),
            // A paused runner that still carries the busy flag is unavailable.
            runner("paused-busy", "linux", "paused", 1, false),
            runner("failed-busy", "linux", "failed", 1, false),
            runner("busy", "linux", "busy", 1, false),
            // The two online spellings split by the busy flag.
            runner("online-working", "linux", "online", 1, false),
            runner("online-free", "linux", "online", 0, false),
            runner("idle-free", "linux", "idle", 0, false),
            runner("starting", "linux", "starting", 0, false),
            runner("odd", "linux", "something-new", 0, false),
        ],
    )
    .await?;

    let body = overview_of(&fixture).await?;
    assert_tiles_partition(&body);
    let metrics = &body["metrics"];
    assert_eq!(metrics["runners"], 8);
    assert_eq!(metrics["busy"], 2);
    assert_eq!(metrics["idle"], 2);
    assert_eq!(metrics["starting"], 1);
    assert_eq!(metrics["unavailable"], 3);
    Ok(())
}

#[tokio::test]
async fn two_org_pools_share_the_queue_instead_of_each_counting_all_of_it() -> Result<()> {
    let fixture = ApiFixture::start().await?;
    let database = fixture.database().await?;
    let mut statements = vec![
        DASHBOARD_REPOSITORIES.to_owned(),
        org_pool("alpha", "[\"shared\"]", ""),
        org_pool("beta", "[\"shared\"]", ""),
        QUEUED_RUN.to_owned(),
    ];
    for index in 0..3 {
        statements.push(job(1000 + index, 100, "queued", "[\"shared\"]", 10 + index));
    }
    seed(&database, &statements).await?;

    let body = overview_of(&fixture).await?;
    assert_tiles_partition(&body);
    assert_eq!(body["metrics"]["queuedJobs"], 3);
    assert_eq!(body["metrics"]["queuedWaiting"], 3);
    assert_eq!(body["metrics"]["queuedUnmatched"], 0);
    let per_pool = body["pools"]
        .as_array()
        .map(|pools| pools.iter().map(|pool| count(&pool["queue"])).sum::<i64>());
    assert_eq!(per_pool, Some(3), "{}", body["pools"]);
    Ok(())
}

#[tokio::test]
async fn jobs_whose_only_matching_pools_are_paused_or_tripped_have_no_matching_pool() -> Result<()>
{
    let fixture = ApiFixture::start().await?;
    let database = fixture.database().await?;
    seed(
        &database,
        &[
            DASHBOARD_REPOSITORIES.to_owned(),
            org_pool("paused-pool", "[\"needs-paused\"]", "1,0"),
            org_pool("tripped-pool", "[\"needs-tripped\"]", "0,1"),
            org_pool("live-pool", "[\"needs-live\"]", "0,0"),
            QUEUED_RUN.to_owned(),
            job(1000, 100, "queued", "[\"needs-paused\"]", 10),
            job(1001, 100, "queued", "[\"needs-paused\"]", 11),
            job(1002, 100, "queued", "[\"needs-tripped\"]", 12),
            job(1003, 100, "queued", "[\"needs-live\"]", 13),
        ],
    )
    .await?;

    let body = overview_of(&fixture).await?;
    assert_tiles_partition(&body);
    let metrics = &body["metrics"];
    assert_eq!(metrics["queuedJobs"], 4);
    assert_eq!(metrics["queuedWaiting"], 1);
    assert_eq!(metrics["queuedUnmatched"], 3);
    let live = body["pools"]
        .as_array()
        .and_then(|pools| pools.iter().find(|pool| pool["id"] == "live-pool"))
        .cloned()
        .unwrap_or_default();
    assert_eq!(live["queue"], 1);
    // Every label exists on some pool, so none is reported as missing; the
    // alert says the jobs have no available pool instead.
    assert_eq!(body["queue"]["missingLabels"], json!([]));
    let titles = body["slo"]["alerts"]
        .as_array()
        .map(|alerts| {
            alerts
                .iter()
                .filter_map(|alert| alert["title"].as_str())
                .collect::<Vec<_>>()
                .join("|")
        })
        .unwrap_or_default();
    assert!(titles.contains("3 jobs have no available pool"), "{titles}");
    Ok(())
}

/// The run page's word on each pool for a queued job, by pool id: its status
/// and whether it is the pool the job is counted for.
fn run_page_pools(run: &Value, job_id: i64) -> std::collections::BTreeMap<String, (String, bool)> {
    run["jobs"]
        .as_array()
        .and_then(|jobs| jobs.iter().find(|job| job["id"] == job_id))
        .and_then(|job| job["diagnosis"]["candidates"].as_array())
        .map(|candidates| {
            candidates
                .iter()
                .map(|candidate| {
                    (
                        candidate["id"].as_str().unwrap_or_default().to_owned(),
                        (
                            candidate["status"].as_str().unwrap_or_default().to_owned(),
                            candidate["owner"].as_bool().unwrap_or(false),
                        ),
                    )
                })
                .collect()
        })
        .unwrap_or_default()
}

/// The run page and the Queued jobs list name the same pool for every queued
/// job (or both name none), and the run page never calls a pool that doesn't
/// count the job ready.
fn assert_run_page_agrees_with_queue(run: &Value, list: &Value) {
    for item in list["items"].as_array().cloned().unwrap_or_default() {
        let id = item["id"].as_i64().unwrap_or_default();
        let pools = run_page_pools(run, id);
        let owners = pools
            .iter()
            .filter(|(_, (_, owner))| *owner)
            .map(|(pool, _)| pool.clone())
            .collect::<Vec<_>>();
        let listed = item["matchedPool"]["id"]
            .as_str()
            .map(ToOwned::to_owned)
            .into_iter()
            .collect::<Vec<_>>();
        assert_eq!(owners, listed, "job {id}: run page {pools:?}, list {item}");
        for (pool, (status, owner)) in &pools {
            assert!(
                *owner || status != "ready",
                "job {id}: {pool} is called ready but doesn't count the job: {pools:?}"
            );
        }
    }
}

async fn run_detail(fixture: &ApiFixture, run_id: i64) -> Result<Value> {
    let response = fixture
        .client
        .get(format!(
            "{}/api/v1/workflow-runs/{run_id}",
            fixture.base_url
        ))
        .header(header::COOKIE, ApiFixture::session_cookie()?)
        .send()
        .await?;
    assert_eq!(response.status(), StatusCode::OK);
    Ok(response.json().await?)
}

/// #63: a manual-mode pool's labels match a job, but nothing starts a runner
/// for it. The run page said the pool matched and had headroom; the Overview
/// counted the job as having no matching pool. Both now say what happens: the
/// job is the manual pool's, and it waits for one of that pool's own runners.
#[tokio::test]
async fn a_job_only_a_manual_pool_can_run_is_neither_unmatched_nor_promised_a_runner() -> Result<()>
{
    let fixture = ApiFixture::start().await?;
    let database = fixture.database().await?;
    seed(
        &database,
        &[
            DASHBOARD_REPOSITORIES.to_owned(),
            org_pool("manual-pool", "[\"needs-manual\",\"shared\"]", ""),
            org_pool("live-pool", "[\"needs-live\",\"shared\"]", ""),
            org_pool("paused-pool", "[\"needs-paused\"]", "1,0"),
            org_pool("tripped-pool", "[\"needs-tripped\"]", "0,1"),
            "UPDATE runner_pools SET autoscaling_enabled=0 WHERE id='manual-pool';".to_owned(),
            QUEUED_RUN.to_owned(),
            job(
                1000,
                100,
                "queued",
                "[\"self-hosted\",\"needs-manual\"]",
                10,
            ),
            job(1001, 100, "queued", "[\"needs-live\"]", 11),
            job(1002, 100, "queued", "[\"nobody-has-this\"]", 12),
            job(1003, 100, "queued", "[\"needs-paused\"]", 13),
            job(1004, 100, "queued", "[\"needs-tripped\"]", 14),
            // Both a manual and an autoscaling pool can run it: the
            // autoscaling one starts a runner, so it is that pool's.
            job(1005, 100, "queued", "[\"shared\"]", 15),
        ],
    )
    .await?;

    let body = overview_of(&fixture).await?;
    assert_tiles_partition(&body);
    let metrics = &body["metrics"];
    assert_eq!(metrics["queuedJobs"], 6);
    assert_eq!(metrics["queuedWaiting"], 2);
    assert_eq!(metrics["queuedManual"], 1);
    assert_eq!(metrics["queuedUnmatched"], 3);
    let row = |id: &str| {
        body["pools"]
            .as_array()
            .and_then(|pools| pools.iter().find(|pool| pool["id"] == id))
            .map(|pool| pool["queue"].clone())
            .unwrap_or_default()
    };
    assert_eq!(row("manual-pool"), 1);
    assert_eq!(row("live-pool"), 2);
    let titles = body["slo"]["alerts"]
        .as_array()
        .map(|alerts| {
            alerts
                .iter()
                .filter_map(|alert| alert["title"].as_str())
                .collect::<Vec<_>>()
                .join("|")
        })
        .unwrap_or_default();
    // The paused and tripped pools' jobs; the manual pool's job is not one.
    assert!(titles.contains("2 jobs have no available pool"), "{titles}");

    let list = queued_jobs_page(&fixture, "?perPage=100").await?;
    let item = |id: i64| {
        list["items"]
            .as_array()
            .and_then(|items| items.iter().find(|item| item["id"] == id))
            .cloned()
            .unwrap_or_default()
    };
    assert_eq!(item(1000)["matchedPool"]["id"], "manual-pool");
    assert_eq!(item(1000)["waitingReason"], "manual_pool");
    assert_eq!(item(1005)["matchedPool"]["id"], "live-pool");
    assert_eq!(item(1002)["waitingReason"], "no_matching_pool");

    let run = run_detail(&fixture, 100).await?;
    assert_run_page_agrees_with_queue(&run, &list);
    let status = |job: i64, pool: &str| {
        run_page_pools(&run, job)
            .get(pool)
            .map(|(status, _)| status.clone())
            .unwrap_or_default()
    };
    assert_eq!(status(1000, "manual-pool"), "manual");
    assert_eq!(status(1000, "live-pool"), "incompatible");
    assert_eq!(status(1001, "live-pool"), "ready");
    assert_eq!(status(1001, "manual-pool"), "incompatible");
    assert_eq!(status(1003, "paused-pool"), "paused");
    assert_eq!(status(1004, "tripped-pool"), "circuit_open");
    assert_eq!(status(1005, "live-pool"), "ready");
    assert_eq!(status(1005, "manual-pool"), "manual");
    let summary = |job: i64| {
        run["jobs"]
            .as_array()
            .and_then(|jobs| jobs.iter().find(|item| item["id"] == job))
            .and_then(|item| item["diagnosis"]["summary"].as_str())
            .unwrap_or_default()
            .to_owned()
    };
    let manual = summary(1000);
    assert!(manual.contains("manual"), "{manual}");
    assert!(manual.contains("does not start"), "{manual}");
    assert!(!manual.contains("headroom"), "{manual}");
    Ok(())
}

#[tokio::test]
async fn the_queued_tile_counts_every_waiting_status_and_skips_finished_runs() -> Result<()> {
    let fixture = ApiFixture::start().await?;
    let database = fixture.database().await?;
    seed(
        &database,
        &[
            DASHBOARD_REPOSITORIES.to_owned(),
            org_pool("linux", "[\"shared\"]", ""),
            QUEUED_RUN.to_owned(),
            "INSERT INTO workflow_runs (id,repository_id,workflow_name,run_number,event,status,conclusion,head_sha,html_url,github_created_at,github_updated_at,created_at,updated_at)
               VALUES (101,7,'Release',2,'push','completed','success','abd','https://github.com/fixture/app/actions/runs/101',1,1,1,1);".to_owned(),
            job(1000, 100, "queued", "[\"shared\"]", 10),
            job(1001, 100, "waiting", "[\"shared\"]", 11),
            job(1002, 100, "pending", "[\"shared\"]", 12),
            job(1003, 100, "requested", "[\"shared\"]", 13),
            job(1004, 100, "in_progress", "[\"shared\"]", 14),
            job(1005, 100, "completed", "[\"shared\"]", 15),
            // A job still listed as queued in a finished run never runs.
            job(1010, 101, "queued", "[\"shared\"]", 16),
        ],
    )
    .await?;

    let body = overview_of(&fixture).await?;
    assert_tiles_partition(&body);
    assert_eq!(body["metrics"]["queuedJobs"], 4);
    assert_eq!(body["metrics"]["queuedWaiting"], 4);
    assert_eq!(body["metrics"]["queuedUnmatched"], 0);
    assert_eq!(body["pools"][0]["queue"], 4);
    Ok(())
}

#[tokio::test]
async fn the_tiles_only_count_what_the_viewer_has_access_to() -> Result<()> {
    let fixture = ApiFixture::start().await?;
    let database = fixture.database().await?;
    // Installation 43 belongs to someone else: its pool, runners and jobs
    // must stay out of every number.
    seed(
        &database,
        &[
            DASHBOARD_REPOSITORIES.to_owned(),
            org_pool("mine", "[\"shared\"]", ""),
            runner("mine-idle", "mine", "online", 0, false),
            QUEUED_RUN.to_owned(),
            job(1000, 100, "queued", "[\"shared\"]", 10),
            "INSERT INTO installations (id,account_id,account_login,account_type,target_type,repository_selection,created_at,updated_at)
               VALUES (43,43,'other','Organization','Organization','selected',1,1);
             INSERT INTO repositories (id,installation_id,owner,name,full_name,private,default_branch,html_url,last_synced_at,created_at,updated_at)
               VALUES (8,43,'other','app','other/app',0,'main','https://github.com/other/app',1,1,1);
             INSERT INTO runner_pools (id,installation_id,name,scope,provider,providers,labels,image,docker_image,tart_image,autoscaling_enabled,max_count,created_at,updated_at)
               VALUES ('theirs',43,'theirs','organization','docker','[\"docker\"]','[\"shared\"]','runner:latest','runner:latest','base',1,10,1,1);
             INSERT INTO runners (id,pool_id,name,provider,status,busy,created_at,updated_at)
               VALUES ('theirs-busy','theirs','theirs-busy','docker','busy',1,1,1);
             INSERT INTO workflow_runs (id,repository_id,workflow_name,run_number,event,status,head_sha,html_url,github_created_at,github_updated_at,created_at,updated_at)
               VALUES (200,8,'CI',1,'push','queued','abc','https://github.com/other/app/actions/runs/200',2,2,2,2);".to_owned(),
            job(2000, 200, "queued", "[\"shared\"]", 10),
            job(2001, 200, "queued", "[\"nobody-has-this\"]", 11),
        ],
    )
    .await?;

    let body = overview_of(&fixture).await?;
    assert_tiles_partition(&body);
    let metrics = &body["metrics"];
    assert_eq!(metrics["runners"], 1);
    assert_eq!(metrics["idle"], 1);
    assert_eq!(metrics["busy"], 0);
    assert_eq!(metrics["queuedJobs"], 1);
    assert_eq!(metrics["queuedWaiting"], 1);
    assert_eq!(metrics["queuedUnmatched"], 0);
    assert_eq!(body["pools"].as_array().map(Vec::len), Some(1));
    Ok(())
}

#[tokio::test]
async fn an_empty_instance_partitions_into_zeros() -> Result<()> {
    let fixture = ApiFixture::start().await?;
    let body = overview_of(&fixture).await?;
    assert_tiles_partition(&body);
    assert_eq!(body["metrics"]["runners"], 0);
    assert_eq!(body["metrics"]["queuedJobs"], 0);
    assert_eq!(body["queue"]["missingLabels"], json!([]));
    Ok(())
}
