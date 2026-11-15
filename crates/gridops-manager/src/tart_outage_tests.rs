//! The manager while the Tart (macOS) agent is out of reach: the Mac it runs
//! on sleeps, or its agent is down. Docker runners have nothing to do with
//! the Mac, so listing them must not fail with it, and a call that only
//! reads from the agent must give up within seconds instead of holding the
//! reconciler's pass for minutes. End to end through the real router, with a
//! stand-in Docker engine (`stand_in_docker`) that lists one runner, and a
//! stand-in Tart agent that either answers or, asleep, never does.
//!
//! The list answers `200` with the runners it could see and names each
//! provider it could not ask under `unavailableProviders`: an empty array
//! when every provider answered. A field beside `runners`, not an error
//! status and not a marker on each runner, because there is no runner to
//! mark (the agent sent none), and because the reconciler and the API must
//! tell "the Mac has no runners" apart from "the Mac could not be asked"
//! before deciding a runner is gone.

use std::path::PathBuf;

use super::*;
use crate::{
    route_timeout_tests::{Holding, on_test_clock},
    stand_in_docker::{self, CONTAINER_ID},
};

const TOKEN: &str = "tart-outage-test-token";
const TART_RUNNER: &str = "tart-0123456789abcdef0123456789abcdef";
/// How long a read from an agent that does not answer may hold a request.
/// The Mac being asleep must cost a reconcile pass seconds, not minutes.
const FAIL_FAST: Duration = Duration::from_secs(10);

/// A stand-in Tart agent. Awake, it lists `TART_RUNNER` and reports its
/// capacity; asleep, it takes every request and never answers, as a Mac that
/// went to sleep with the connection open does.
struct StandInTart {
    url: Url,
    asleep: Arc<AtomicBool>,
    /// Requests it is holding while asleep, for `on_test_clock`.
    holding: Holding,
}

/// Counts a held request until it is answered or abandoned.
struct Held(Holding);

impl Held {
    fn new(holding: &Holding) -> Self {
        holding.fetch_add(1, Ordering::SeqCst);
        Self(Arc::clone(holding))
    }
}

impl Drop for Held {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

async fn stand_in_tart(asleep: bool) -> anyhow::Result<StandInTart> {
    let asleep = Arc::new(AtomicBool::new(asleep));
    let holding = Holding::default();
    let answer = |body: Value| {
        let asleep = Arc::clone(&asleep);
        let holding = Arc::clone(&holding);
        move || {
            let asleep = Arc::clone(&asleep);
            let holding = Arc::clone(&holding);
            let body = body.clone();
            async move {
                if asleep.load(Ordering::SeqCst) {
                    let _held = Held::new(&holding);
                    tokio::time::sleep(Duration::from_hours(24)).await;
                }
                Json(body)
            }
        }
    };
    let app = Router::new()
        .route(
            "/v1/runners",
            get(answer(json!({ "runners": [{
                "id": TART_RUNNER,
                "names": ["mac-abcd1234"],
                "image": "gridops-macos-tahoe-base",
                "state": "running",
                "status": "running",
                "labels": {
                    "io.gridops.managed": "true",
                    "io.gridops.runner-id": "runner-mac-1",
                    "io.gridops.pool-id": "pool-mac",
                },
                "createdAt": "2026-11-15T00:00:00Z",
                "provider": "tart",
            }]}))),
        )
        .route(
            "/v1/health",
            get(answer(json!({
                "status": "ok",
                "capacity": {
                    "cpuBudget": 8.0,
                    "memoryBudgetMb": 16_384,
                    "maxRunners": 2,
                    "active": { "activeRunners": 0, "cpu": 0.0, "memoryMb": 0 },
                },
            }))),
        );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let url = Url::parse(&format!("http://{}/", listener.local_addr()?))?;
    tokio::spawn(async move { axum::serve(listener, app).await });
    Ok(StandInTart {
        url,
        asleep,
        holding,
    })
}

/// An address on which nothing listens: connecting is refused at once, as
/// it is when the Mac is awake but its agent is not running.
fn refused_url() -> anyhow::Result<Url> {
    let listener = std::net::TcpListener::bind("127.0.0.1:0")?;
    let address = listener.local_addr()?;
    drop(listener);
    Ok(Url::parse(&format!("http://{address}/"))?)
}

/// The manager's client for the agent, configured as `main` configures it
/// today: one client for every call, with a 45-minute limit so a VM start
/// is never cut short. A faster failure for reads therefore has to come per
/// call, not from this client.
fn production_tart_client() -> anyhow::Result<reqwest::Client> {
    Ok(reqwest::Client::builder()
        .user_agent("GridOps manager/0.1")
        .timeout(Duration::from_mins(45))
        .build()?)
}

/// The real manager router in front of the Tart agent at `tart` and, when
/// given, the stand-in Docker engine at `docker` (otherwise the manager runs
/// without Docker access).
async fn manager(docker: Option<&FsPath>, tart: &Url) -> anyhow::Result<String> {
    let (docker, docker_available) = if let Some(socket) = docker {
        (socket.to_path_buf(), true)
    } else {
        // Nothing listens here, and nothing asks.
        let socket =
            std::env::temp_dir().join(format!("gridops-no-docker-{}.sock", uuid::Uuid::new_v4()));
        std::fs::write(&socket, "")?;
        (socket, false)
    };
    let state = ManagerState {
        docker: Docker::connect_with_socket(
            &docker.to_string_lossy(),
            10,
            bollard::API_DEFAULT_VERSION,
        )?,
        tart: Some(TartAgent {
            base_url: tart.clone(),
            token: SecretString::from("tart-agent-token"),
            http: production_tart_client()?,
        }),
        token: SecretString::from(TOKEN),
        limits: ManagerLimits {
            host_name: "docker-host".into(),
            available_cpus: 8.0,
            total_memory_mb: 16_384,
            cpu_budget: 6.0,
            memory_budget_mb: 12_288,
            max_runners: 3,
            min_free_disk_mb: 0,
            min_free_disk_percent: 0,
            runner_network: "gridops-runners".into(),
            log_max_size: "20m".into(),
            log_max_files: 5,
            runner_pids_limit: 1_024,
            runner_swap_percent: 100,
            budget_origin: BudgetOrigin::default(),
        },
        shared_docker_socket: None,
        reservations: Arc::default(),
        provisioning_paused: Arc::new(AtomicBool::new(false)),
        builtin_host: builtin_host::BuiltinHost::new(docker_available, Some(true)),
        provision_locks: ProvisionLocks::default(),
        vm_slots: Arc::default(),
    };
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let base_url = format!("http://{}", listener.local_addr()?);
    let app = router(state);
    tokio::spawn(async move { axum::serve(listener, app).await });
    Ok(base_url)
}

fn listed_docker() -> anyhow::Result<PathBuf> {
    stand_in_docker::start(stand_in_docker::Images {
        listed: true,
        ..Default::default()
    })
}

/// Sends `request` on the test clock and gives its status, JSON body (null
/// when it has none) and how long it took on that clock.
async fn timed(
    holding: &Holding,
    request: reqwest::RequestBuilder,
) -> anyhow::Result<(StatusCode, Value, Duration)> {
    let started = tokio::time::Instant::now();
    let (status, text) = on_test_clock(holding, async {
        let response = request.send().await?;
        let status = response.status();
        anyhow::Ok((status, response.text().await?))
    })
    .await?;
    Ok((
        status,
        serde_json::from_str(&text).unwrap_or(Value::Null),
        started.elapsed(),
    ))
}

fn get_runners(manager: &str) -> reqwest::RequestBuilder {
    reqwest::Client::new()
        .get(format!("{manager}/v1/runners"))
        .bearer_auth(TOKEN)
}

/// The IDs and providers of the runners a list holds.
fn listed(body: &Value) -> Vec<(String, String)> {
    body["runners"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|runner| {
            (
                runner["id"].as_str().unwrap_or_default().to_owned(),
                runner["provider"].as_str().unwrap_or_default().to_owned(),
            )
        })
        .collect()
}

fn docker_runner_only() -> Vec<(String, String)> {
    vec![(CONTAINER_ID.to_owned(), "docker".to_owned())]
}

/// Requirement 1: the agent refuses the connection (the Mac is awake, its
/// agent is not running). The Docker runners are still listed, and the list
/// says Tart could not be asked; it is not an error.
#[tokio::test]
async fn a_refused_tart_agent_leaves_the_docker_runners_listed_and_names_tart_unavailable()
-> anyhow::Result<()> {
    let docker = listed_docker()?;
    let manager = manager(Some(&docker), &refused_url()?).await?;
    let response = get_runners(&manager).send().await?;
    let status = response.status();
    let body = response.json::<Value>().await.unwrap_or(Value::Null);
    let _ = std::fs::remove_file(&docker);
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(listed(&body), docker_runner_only(), "{body}");
    assert_eq!(body["unavailableProviders"], json!(["tart"]), "{body}");
    Ok(())
}

/// Requirements 1 and 4: the Mac went to sleep with the connection open, so
/// the agent never answers. The list still comes back, without Tart's
/// runners and saying so, within seconds rather than at the route's
/// three-minute limit.
#[tokio::test(start_paused = true)]
async fn a_hung_tart_agent_drops_out_of_the_runner_list_within_seconds() -> anyhow::Result<()> {
    let docker = listed_docker()?;
    let tart = stand_in_tart(true).await?;
    let manager = manager(Some(&docker), &tart.url).await?;
    let (status, body, took) = timed(&tart.holding, get_runners(&manager)).await?;
    let _ = std::fs::remove_file(&docker);
    assert_eq!(status, StatusCode::OK, "after {took:?}: {body}");
    assert!(took <= FAIL_FAST, "the list waited {took:?} for the agent");
    assert_eq!(listed(&body), docker_runner_only(), "{body}");
    assert_eq!(body["unavailableProviders"], json!(["tart"]), "{body}");
    Ok(())
}

/// Requirement 4: health asks the agent too. Against an agent that never
/// answers it reports Tart unavailable within seconds.
#[tokio::test(start_paused = true)]
async fn health_reports_a_hung_tart_agent_unavailable_within_seconds() -> anyhow::Result<()> {
    let tart = stand_in_tart(true).await?;
    let manager = manager(None, &tart.url).await?;
    let (status, body, took) = timed(
        &tart.holding,
        reqwest::Client::new()
            .get(format!("{manager}/v1/health"))
            .bearer_auth(TOKEN),
    )
    .await?;
    assert_eq!(status, StatusCode::OK, "after {took:?}: {body}");
    assert!(took <= FAIL_FAST, "health waited {took:?} for the agent");
    assert_eq!(body["providers"]["tart"]["available"], false, "{body}");
    Ok(())
}

/// Requirement 5 (a sibling of the list): a Tart admission reads the agent's
/// health inside the reconciler's pass, before anything is started. Against
/// an agent that never answers it is refused as unavailable within seconds,
/// so it holds the pass no longer than the list does.
#[tokio::test(start_paused = true)]
async fn a_tart_admission_against_a_hung_agent_is_refused_within_seconds() -> anyhow::Result<()> {
    let tart = stand_in_tart(true).await?;
    let manager = manager(None, &tart.url).await?;
    let (status, body, took) = timed(
        &tart.holding,
        reqwest::Client::new()
            .post(format!("{manager}/v1/admissions"))
            .bearer_auth(TOKEN)
            .json(&json!({
                "runnerId": "runner-mac-2",
                "poolId": "pool-mac",
                "provider": "tart",
                "cpuLimit": 4.0,
                "memoryLimitMb": 4_096,
            })),
    )
    .await?;
    assert_eq!(
        status,
        StatusCode::SERVICE_UNAVAILABLE,
        "after {took:?}: {body}"
    );
    assert!(
        took <= FAIL_FAST,
        "the admission waited {took:?} for the agent"
    );
    assert_eq!(body["code"], "tart_agent_unavailable", "{body}");
    Ok(())
}

/// Requirement 6: once the agent answers again, the next list holds its
/// runners and names no provider unavailable. Nothing about the outage is
/// remembered past it.
#[tokio::test(start_paused = true)]
async fn the_tart_runners_are_listed_again_as_soon_as_the_agent_answers() -> anyhow::Result<()> {
    let docker = listed_docker()?;
    let tart = stand_in_tart(true).await?;
    let manager = manager(Some(&docker), &tart.url).await?;
    let (status, asleep, _) = timed(&tart.holding, get_runners(&manager)).await?;
    assert_eq!(status, StatusCode::OK, "while asleep: {asleep}");
    assert_eq!(asleep["unavailableProviders"], json!(["tart"]), "{asleep}");

    tart.asleep.store(false, Ordering::SeqCst);
    let (status, awake, _) = timed(&tart.holding, get_runners(&manager)).await?;
    let _ = std::fs::remove_file(&docker);
    assert_eq!(status, StatusCode::OK, "{awake}");
    let mut runners = listed(&awake);
    runners.sort();
    assert_eq!(
        runners,
        [
            (CONTAINER_ID.to_owned(), "docker".to_owned()),
            (TART_RUNNER.to_owned(), "tart".to_owned()),
        ],
        "{awake}"
    );
    assert_eq!(awake["unavailableProviders"], json!([]), "{awake}");
    Ok(())
}
