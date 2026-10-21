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
    routing::post,
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
        Ok(self
            .client
            .post(format!("{}/api/v1/webhooks/github", self.base_url))
            .header("x-github-delivery", delivery)
            .header("x-github-event", "ping")
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
async fn installers_are_public_and_point_at_this_deployment() -> Result<()> {
    let fixture = ApiFixture::start().await?;

    for path in ["install.sh", "install.ps1"] {
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
    }
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

/// Stands in for api.github.com: mints installation tokens and records the
/// token each run cancellation arrives with. It refuses the user's token for
/// run 3, as GitHub does when the person can't write to the repository.
async fn fake_github() -> Result<(String, SeenTokens)> {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let app = Router::new()
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
