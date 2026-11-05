//! Runner starts as the reconciler runs them (#95 phase 2, #104): Tart starts
//! beside the pass, and a freed macOS VM slot handed to the head of the line
//! at once. Through a stand-in runner manager and GitHub, and the real
//! database.

use super::*;
use gridops_core::connect_database_path;
use rsa::pkcs1::{EncodeRsaPrivateKey as _, LineEnding};
use std::fs;
use std::sync::{
    Mutex,
    atomic::{AtomicBool, AtomicU64},
};
use tokio::sync::Semaphore;

/// What the stand-in manager was asked and how it answers.
struct ManagerLog {
    /// Admissions asked for: pool, provider and the pass identifier sent.
    admissions: Mutex<Vec<(String, String, i64)>>,
    /// Runner starts asked for: pool and provider.
    starts: Mutex<Vec<(String, String)>>,
    /// The runners it started, as `GET /v1/runners` lists them.
    running: Mutex<Vec<Value>>,
    /// Refuse Tart admissions for want of a VM slot.
    refuse_tart: AtomicBool,
    /// A Tart start waits for a permit here, so a test decides when it ends.
    tart_gate: Semaphore,
    /// A Tart start that ends fails.
    tart_fails: AtomicBool,
    /// The slots in use and free, as `GET /v1/vm-slots` reports them.
    slots_in_use: AtomicU64,
    slots_free: AtomicU64,
}

impl Default for ManagerLog {
    fn default() -> Self {
        Self {
            admissions: Mutex::default(),
            starts: Mutex::default(),
            running: Mutex::default(),
            refuse_tart: AtomicBool::default(),
            tart_gate: Semaphore::new(0),
            tart_fails: AtomicBool::default(),
            slots_in_use: AtomicU64::default(),
            slots_free: AtomicU64::default(),
        }
    }
}

impl ManagerLog {
    fn admissions(&self, provider: &str) -> Vec<(String, i64)> {
        self.admissions
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .iter()
            .filter(|(_, asked, _)| asked == provider)
            .map(|(pool, _, pass)| (pool.clone(), *pass))
            .collect()
    }

    fn starts(&self, provider: &str) -> Vec<String> {
        self.starts
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .iter()
            .filter(|(_, asked)| asked == provider)
            .map(|(pool, _)| pool.clone())
            .collect()
    }
}

async fn fake_manager() -> Result<(String, Arc<ManagerLog>)> {
    use axum::{
        Json, Router,
        http::StatusCode as Status,
        routing::{get, post, put},
    };
    let log = Arc::new(ManagerLog::default());
    let admissions = Arc::clone(&log);
    let starts = Arc::clone(&log);
    let listing = Arc::clone(&log);
    let slots = Arc::clone(&log);
    let app = Router::new()
        .route("/v1/policy", put(|| async { Json(json!({})) }))
        .route(
            "/v1/runners",
            get(move || {
                let log = Arc::clone(&listing);
                async move {
                    let running = log
                        .running
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                        .clone();
                    Json(json!({ "runners": running }))
                }
            })
            .post(move |Json(body): Json<Value>| {
                let log = Arc::clone(&starts);
                async move {
                    let provider = body["provider"].as_str().unwrap_or_default().to_owned();
                    let runner_id = body["runnerId"].as_str().unwrap_or_default().to_owned();
                    log.starts
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                        .push((
                            body["poolId"].as_str().unwrap_or_default().to_owned(),
                            provider.clone(),
                        ));
                    if provider == "tart" {
                        if let Ok(permit) = log.tart_gate.acquire().await {
                            permit.forget();
                        }
                        if log.tart_fails.load(Ordering::SeqCst) {
                            return (
                                Status::BAD_GATEWAY,
                                Json(json!({
                                    "code": "tart_agent_error",
                                    "error": "the Tart agent could not clone the base VM",
                                })),
                            );
                        }
                    }
                    let id = format!("ctr-{runner_id}");
                    log.running
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                        .push(json!({
                            "id": id, "state": "running",
                            "labels": { "io.gridops.managed": "true", "io.gridops.runner-id": runner_id },
                        }));
                    (
                        Status::CREATED,
                        Json(json!({ "id": id, "name": body["name"], "state": "running" })),
                    )
                }
            }),
        )
        .route(
            "/v1/admissions",
            post(move |Json(body): Json<Value>| {
                let log = Arc::clone(&admissions);
                async move {
                    let provider = body["provider"].as_str().unwrap_or_default().to_owned();
                    log.admissions
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                        .push((
                            body["poolId"].as_str().unwrap_or_default().to_owned(),
                            provider.clone(),
                            body["reconcilePass"].as_i64().unwrap_or_default(),
                        ));
                    if provider == "tart" && log.refuse_tart.load(Ordering::SeqCst) {
                        return (
                            Status::TOO_MANY_REQUESTS,
                            Json(json!({
                                "code": "vm_slot_busy",
                                "error": "Waiting for a macOS VM slot: position 1 of 1 in the queue. 2 of 2 slots in use (GridOps runners: 0; other software: 2 — runner-warm-1 (ci), runner-warm-2 (ci)).",
                            })),
                        );
                    }
                    (
                        Status::OK,
                        Json(json!({ "leaseId": uuid::Uuid::new_v4().to_string() })),
                    )
                }
            }),
        )
        .route(
            "/v1/vm-slots",
            get(move || {
                let log = Arc::clone(&slots);
                async move {
                    Json(json!({ "vmSlots": {
                        "limit": 2,
                        "inUse": log.slots_in_use.load(Ordering::SeqCst),
                        "free": log.slots_free.load(Ordering::SeqCst),
                    }}))
                }
            }),
        )
        // Lease releases and anything else: done.
        .fallback(|| async { Status::NO_CONTENT });
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0)).await?;
    let url = format!("http://{}/", listener.local_addr()?);
    tokio::spawn(async move { axum::serve(listener, app).await });
    Ok((url, log))
}

/// A fake GitHub that hands out installation tokens and just-in-time runner
/// configurations.
async fn fake_github() -> Result<String> {
    use axum::{Json, Router, routing::post};
    let next_runner = Arc::new(AtomicU64::new(1));
    let app = Router::new()
        .route(
            "/app/installations/{id}/access_tokens",
            post(|| async {
                Json(json!({ "token": "installation-token", "expires_at": "2099-01-01T00:00:00Z" }))
            }),
        )
        .route(
            "/orgs/{org}/actions/runners/generate-jitconfig",
            post(move |Json(body): Json<Value>| {
                let id = next_runner.fetch_add(1, Ordering::SeqCst);
                async move {
                    Json(json!({
                        "runner": { "id": id, "name": body["name"], "status": "offline", "busy": false },
                        "encoded_jit_config": "jit",
                    }))
                }
            }),
        );
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0)).await?;
    let url = format!("http://{}", listener.local_addr()?);
    tokio::spawn(async move { axum::serve(listener, app).await });
    Ok(url)
}

struct Setup {
    app: Reconciler,
    manager: Arc<ManagerLog>,
    directory: std::path::PathBuf,
}

impl Setup {
    /// A Tart pool (`pool-mac`, visited first) and a Docker pool
    /// (`pool-linux`), each wanting one runner, plus the pools in `extra`
    /// (SQL), with the built-in host on.
    async fn new(extra: &str, watch_interval: Duration) -> Result<Self> {
        let directory =
            std::env::temp_dir().join(format!("gridops-starts-test-{}", uuid::Uuid::new_v4()));
        let database = connect_database_path(&directory.join("gridops.sqlite")).await?;
        let (manager_url, manager) = fake_manager().await?;
        let github_url = fake_github().await?;
        let config = Config::from_env()?
            .with_github_api_url(reqwest::Url::parse(&github_url)?)
            .with_manager(reqwest::Url::parse(&manager_url)?, "manager-token")
            .with_log_directory(directory.join("logs"));
        let vault = Vault::new([0; 32], b"reconciler-test".to_vec());
        let key = rsa::RsaPrivateKey::new(&mut rsa::rand_core::OsRng, 2048)?;
        let key = key.to_pkcs1_pem(LineEnding::LF)?.to_string();
        sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
            r#"
            INSERT OR REPLACE INTO settings (key,value,updated_at) VALUES ('{builtin}','true',1);
            INSERT INTO installations (id,account_id,account_login,account_type,target_type,repository_selection,created_at,updated_at)
              VALUES (1,1,'octo-org','Organization','Organization','all',1,1);
            INSERT INTO runner_pools (id,installation_id,name,scope,image,provider,providers,docker_image,tart_image,desired_count,autoscaling_enabled,created_at,updated_at)
              VALUES ('pool-mac',1,'mac','organization','base','tart','["tart"]','','base',1,0,1,1),
                     ('pool-linux',1,'linux','organization','runner:latest','docker','["docker"]','runner:latest','',1,0,2,2);
            {extra}
            "#,
            builtin = gridops_core::builtin_host::SETTING_KEY,
        )))
        .execute(&database)
        .await?;
        for (name, value) in [("github.app_id", "1"), ("github.app_private_key", &key)] {
            sqlx::query("INSERT INTO runtime_secrets (key,value,updated_at) VALUES (?,?,1)")
                .bind(name)
                .bind(vault.seal(value)?)
                .execute(&database)
                .await?;
        }
        let app = Reconciler {
            github: GitHubClient::new(config.clone())?,
            config,
            database,
            bitbucket: BitbucketClient::new()?,
            vault,
            http: reqwest::Client::new(),
            pass: Arc::default(),
            tart: Arc::new(tart_lane::TartLane::new(Arc::default(), watch_interval)),
        };
        Ok(Self {
            app,
            manager,
            directory,
        })
    }

    /// The pool's runners that are not deleted: status, and whether the
    /// manager has started them.
    async fn runners(&self, pool_id: &str) -> Result<Vec<(String, bool)>> {
        Ok(sqlx::query_as::<_, (String, bool)>(
            "SELECT status,container_id IS NOT NULL FROM runners WHERE pool_id=? AND deleted_at IS NULL ORDER BY created_at",
        )
        .bind(pool_id)
        .fetch_all(&self.app.database)
        .await?)
    }

    /// Waits, at most five seconds, for the Tart start the pass handed over
    /// to reach the manager: the pass does not wait for it.
    async fn tart_start_reached_manager(&self) -> Result<()> {
        tokio::time::timeout(Duration::from_secs(5), async {
            while self.manager.starts("tart").is_empty() {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .context("the Tart start did not reach the manager")
    }

    /// Waits, at most five seconds, for the Tart start under way to end.
    async fn tart_start_ended(&self) -> Result<()> {
        tokio::time::timeout(Duration::from_secs(5), async {
            while self.app.tart.claimed() {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .context("the Tart start did not end")
    }

    async fn finish(self) -> Result<()> {
        self.app.database.close().await;
        fs::remove_dir_all(self.directory)?;
        Ok(())
    }
}

/// A pass may take this long here; it would take as long as the Tart start
/// if it waited for it, and that start never ends unless the test lets it.
const PASS_LIMIT: Duration = Duration::from_secs(10);

/// Part 1 of the task: a Tart start no longer holds up the pass. The pools
/// after it, Docker's among them, get their runners while it runs, and the
/// next pass neither starts the Tart pool again nor asks for a Tart runner
/// for another pool while it runs.
#[tokio::test]
async fn a_tart_start_runs_beside_the_pass_and_is_started_once() -> Result<()> {
    let setup = Setup::new(
        r#"INSERT INTO runner_pools (id,installation_id,name,scope,image,provider,providers,docker_image,tart_image,desired_count,autoscaling_enabled,created_at,updated_at)
             VALUES ('pool-mac-2',1,'mac-2','organization','base','tart','["tart"]','','base',1,0,3,3);"#,
        tart_lane::SLOT_WATCH_INTERVAL,
    )
    .await?;

    let first_pass = tokio::time::timeout(PASS_LIMIT, run_pass(&setup.app)).await;
    assert!(
        first_pass.is_ok(),
        "the pass waited for the Tart start, which has not ended"
    );
    setup.tart_start_reached_manager().await?;
    assert_eq!(setup.manager.starts("tart"), ["pool-mac"]);
    assert_eq!(
        setup.runners("pool-mac").await?,
        [("starting".to_owned(), false)],
        "the Tart runner is recorded, starting"
    );
    assert_eq!(
        setup.runners("pool-linux").await?,
        [("online".to_owned(), true)],
        "the Docker pool after it got its runner in the same pass"
    );
    assert_eq!(
        setup.manager.admissions("tart"),
        [(
            "pool-mac".to_owned(),
            setup.app.pass.load(Ordering::Relaxed)
        )],
        "no second Tart runner is asked for while one starts"
    );
    let round = setup.app.pass.load(Ordering::Relaxed);

    // The next pass, the start still under way: nothing is started again.
    tokio::time::timeout(PASS_LIMIT, run_pass(&setup.app)).await?;
    assert_eq!(setup.manager.starts("tart"), ["pool-mac"], "started once");
    assert_eq!(setup.manager.admissions("tart").len(), 1);
    assert_eq!(setup.runners("pool-mac").await?.len(), 1);
    assert_eq!(setup.runners("pool-mac-2").await?, []);
    assert_eq!(setup.manager.starts("docker"), ["pool-linux"]);

    // The start ends; the pass is woken, records it, and asks for the pool
    // held back, in the round that was open when it was held back.
    let woken = setup.app.tart.wake().notified();
    setup.manager.tart_gate.add_permits(1);
    setup.tart_start_ended().await?;
    tokio::time::timeout(Duration::from_secs(1), woken)
        .await
        .context("the end of the Tart start wakes the pass")?;
    tokio::time::timeout(PASS_LIMIT, run_pass(&setup.app)).await?;
    assert_eq!(
        setup.runners("pool-mac").await?,
        [("online".to_owned(), true)]
    );
    assert_eq!(
        setup.manager.admissions("tart"),
        [
            ("pool-mac".to_owned(), round),
            ("pool-mac-2".to_owned(), round)
        ],
        "a pool held back keeps the round, so it keeps its place in the line"
    );
    let reconciled: String = sqlx::query_scalar(
        "SELECT metadata FROM runner_events WHERE pool_id='pool-mac' AND event='Pool reconciled'",
    )
    .fetch_one(&setup.app.database)
    .await?;
    assert_eq!(
        serde_json::from_str::<Value>(&reconciled)?["provisioned"],
        1,
        "the pass that records the start counts it"
    );
    setup.manager.tart_gate.add_permits(1);
    setup.tart_start_ended().await?;
    setup.finish().await
}

/// A Tart start that fails in the background counts towards its pool's
/// provisioning circuit, and is reported, exactly as a failed start in the
/// pass was: here the third failure in a row opens the circuit.
#[tokio::test]
async fn a_failed_background_tart_start_opens_the_circuit_as_before() -> Result<()> {
    let setup = Setup::new(
        "UPDATE runner_pools SET provision_failure_count=2 WHERE id='pool-mac';",
        tart_lane::SLOT_WATCH_INTERVAL,
    )
    .await?;
    setup.manager.tart_fails.store(true, Ordering::SeqCst);
    tokio::time::timeout(PASS_LIMIT, run_pass(&setup.app)).await?;
    setup.manager.tart_gate.add_permits(1);
    setup.tart_start_ended().await?;
    tokio::time::timeout(PASS_LIMIT, run_pass(&setup.app)).await?;

    let (state, failures, circuit_open, retry_at): (String, i64, bool, Option<i64>) =
        sqlx::query_as(
            "SELECT state,provision_failure_count,provision_circuit_open,provision_retry_at FROM runner_pools WHERE id='pool-mac'",
        )
        .fetch_one(&setup.app.database)
        .await?;
    assert_eq!((failures, circuit_open), (3, true));
    assert_eq!(state, "blocked");
    assert!(retry_at.is_some_and(|at| at > now_millis() + 14 * 60_000));
    let events = sqlx::query_scalar::<_, String>(
        "SELECT event FROM runner_events WHERE pool_id='pool-mac' ORDER BY created_at,rowid",
    )
    .fetch_all(&setup.app.database)
    .await?;
    for event in [
        "Runner provisioning failed",
        "Provisioning circuit opened",
        "Pool reconciliation failed",
    ] {
        assert!(
            events.iter().any(|seen| seen == event),
            "{event}: {events:?}"
        );
    }
    let failed: Vec<(String, Option<String>)> =
        sqlx::query_as("SELECT status,failure_reason FROM runners WHERE pool_id='pool-mac'")
            .fetch_all(&setup.app.database)
            .await?;
    assert_eq!(failed.len(), 1);
    // Marked failed when the start failed; the pass that recorded it has
    // already cleared it away, as it clears every failed runner.
    assert!(
        ["failed", "deleted"].contains(&failed[0].0.as_str()),
        "{failed:?}"
    );
    assert!(
        failed[0]
            .1
            .as_deref()
            .is_some_and(|reason| reason.contains("[tart_agent_error]")),
        "{failed:?}"
    );
    // The open circuit holds the next start back.
    tokio::time::timeout(PASS_LIMIT, run_pass(&setup.app)).await?;
    assert_eq!(setup.manager.starts("tart").len(), 1);
    setup.finish().await
}

/// Part 2 of the task: a macOS VM slot that frees while a pool waits in the
/// line for one goes to that pool at once, not on the next scheduled pass
/// (here an hour away).
#[tokio::test]
async fn a_freed_slot_starts_the_head_of_the_line_without_waiting_for_the_next_pass() -> Result<()>
{
    let setup = Setup::new(
        "INSERT OR REPLACE INTO settings (key,value,updated_at) VALUES ('reconcileIntervalSeconds','3600',1);",
        Duration::from_millis(50),
    )
    .await?;
    setup.manager.refuse_tart.store(true, Ordering::SeqCst);
    setup.manager.slots_in_use.store(2, Ordering::SeqCst);
    let passes = tokio::spawn(run_passes(setup.app.clone()));
    let watcher = tokio::spawn(watch_vm_slots(setup.app.clone()));

    let refused = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let waiting: Option<String> =
                sqlx::query_scalar("SELECT state FROM runner_pools WHERE id='pool-mac'")
                    .fetch_optional(&setup.app.database)
                    .await?;
            if waiting.as_deref() == Some("waiting") {
                return Result::<()>::Ok(());
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await;
    assert!(
        matches!(refused, Ok(Ok(()))),
        "the first pass finds no slot"
    );
    assert_eq!(setup.manager.admissions("tart").len(), 1);
    // Several looks at the full slots wake nothing.
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(setup.manager.admissions("tart").len(), 1, "nothing freed");

    // A VM of other software stops.
    setup.manager.refuse_tart.store(false, Ordering::SeqCst);
    setup.manager.slots_free.store(1, Ordering::SeqCst);
    setup.manager.slots_in_use.store(1, Ordering::SeqCst);
    setup.manager.tart_gate.add_permits(1);
    let started = tokio::time::timeout(Duration::from_secs(5), async {
        while setup.runners("pool-mac").await? != [("online".to_owned(), true)] {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        Result::<()>::Ok(())
    })
    .await;
    assert!(
        matches!(started, Ok(Ok(()))),
        "the head of the line started at once, not after the hour's interval"
    );
    assert_eq!(setup.manager.admissions("tart").len(), 2);
    passes.abort();
    watcher.abort();
    setup.finish().await
}

/// A runner whose Tart start runs beside the pass has no VM yet, so a pass
/// that no longer wants it (here its pool was paused) leaves it until the
/// start has ended, and removes it then. Removed earlier, the VM the start
/// was about to boot would run on with no runner to account for it.
#[tokio::test]
async fn a_runner_whose_tart_start_is_under_way_is_removed_only_once_it_has_ended() -> Result<()> {
    let setup = Setup::new("", tart_lane::SLOT_WATCH_INTERVAL).await?;
    tokio::time::timeout(PASS_LIMIT, run_pass(&setup.app)).await?;
    sqlx::query("UPDATE runner_pools SET paused=1 WHERE id='pool-mac'")
        .execute(&setup.app.database)
        .await?;
    tokio::time::timeout(PASS_LIMIT, run_pass(&setup.app)).await?;
    assert_eq!(
        setup.runners("pool-mac").await?,
        [("starting".to_owned(), false)],
        "left alone while its start runs"
    );
    setup.manager.tart_gate.add_permits(1);
    setup.tart_start_ended().await?;
    tokio::time::timeout(PASS_LIMIT, run_pass(&setup.app)).await?;
    assert_eq!(setup.runners("pool-mac").await?, [], "removed once started");
    setup.finish().await
}
