//! The manager's start locks, end to end through the real router, with a
//! stand-in Docker engine (`stand_in_docker`) and a stand-in Tart agent that
//! holds every runner start until the test lets it finish. A Tart start the
//! agent is holding must not stop Docker work, and two Tart starts must still
//! take turns.

use std::sync::atomic::AtomicUsize;

use tokio::{sync::watch, task::JoinHandle};

use super::*;
use crate::stand_in_docker::{self, CONTAINER_ID};

const TOKEN: &str = "provision-lock-test-token";
const SANDBOX_ID: &str = "sandbox-lock-test";
/// Longer than any request here takes when nothing blocks it, much shorter
/// than a Tart start the agent holds until the test ends.
const NOT_BLOCKED: Duration = Duration::from_secs(15);

/// A stand-in Tart agent that holds every runner start until `open` is sent.
struct StandInTart {
    url: Url,
    open: watch::Sender<bool>,
    /// Starts that reached the agent.
    arrived: Arc<AtomicUsize>,
    /// The most starts the agent held at once.
    most_at_once: Arc<AtomicUsize>,
}

async fn stand_in_tart() -> anyhow::Result<StandInTart> {
    let (open, opened) = watch::channel(false);
    let arrived = Arc::new(AtomicUsize::new(0));
    let holding = Arc::new(AtomicUsize::new(0));
    let most_at_once = Arc::new(AtomicUsize::new(0));
    let start = {
        let arrived = Arc::clone(&arrived);
        let most_at_once = Arc::clone(&most_at_once);
        move || {
            let mut opened = opened.clone();
            let arrived = Arc::clone(&arrived);
            let holding = Arc::clone(&holding);
            let most_at_once = Arc::clone(&most_at_once);
            async move {
                arrived.fetch_add(1, Ordering::SeqCst);
                let now = holding.fetch_add(1, Ordering::SeqCst) + 1;
                most_at_once.fetch_max(now, Ordering::SeqCst);
                let _ = opened.wait_for(|open| *open).await;
                holding.fetch_sub(1, Ordering::SeqCst);
                (
                    StatusCode::CREATED,
                    Json(json!({ "id": "tart-runner", "name": "tart-runner", "state": "running" })),
                )
            }
        }
    };
    let app = Router::new().route("/v1/runners", post(start));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let url = Url::parse(&format!("http://{}/", listener.local_addr()?))?;
    tokio::spawn(async move { axum::serve(listener, app).await });
    Ok(StandInTart {
        url,
        open,
        arrived,
        most_at_once,
    })
}

fn tart_lease(runner_id: &str) -> CapacityReservation {
    CapacityReservation {
        runner_id: runner_id.into(),
        pool_id: "pool-mac".into(),
        provider: "tart".into(),
        cpu_limit: 4.0,
        memory_limit_mb: 4_096,
        vm_slot: true,
        expires_at: Instant::now() + Duration::from_mins(5),
    }
}

/// The real manager router in front of `docker` and `tart`, holding a Tart
/// capacity lease `lease-<runner>` for each of `tart_runners`.
async fn manager(docker: &FsPath, tart: &Url, tart_runners: &[&str]) -> anyhow::Result<String> {
    let state = ManagerState {
        docker: Docker::connect_with_socket(
            &docker.to_string_lossy(),
            10,
            bollard::API_DEFAULT_VERSION,
        )?,
        tart: Some(TartAgent {
            base_url: tart.clone(),
            token: SecretString::from("agent-token"),
            http: reqwest::Client::new(),
        }),
        token: SecretString::from(TOKEN),
        limits: ManagerLimits {
            host_name: "docker-host".into(),
            available_cpus: 8.0,
            total_memory_mb: 16_384,
            cpu_budget: 6.0,
            memory_budget_mb: 12_288,
            max_runners: 3,
            // No disk floor: these tests are not about the host's disk.
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
        reservations: Arc::new(Mutex::new(
            tart_runners
                .iter()
                .map(|runner| (format!("lease-{runner}"), tart_lease(runner)))
                .collect(),
        )),
        provisioning_paused: Arc::new(AtomicBool::new(false)),
        builtin_host: builtin_host::BuiltinHost::new(true, Some(true)),
        provision_locks: ProvisionLocks::default(),
        vm_slots: Arc::default(),
    };
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let base_url = format!("http://{}", listener.local_addr()?);
    let app = router(state);
    tokio::spawn(async move { axum::serve(listener, app).await });
    Ok(base_url)
}

fn request(manager: &str, method: reqwest::Method, path: &str) -> reqwest::RequestBuilder {
    reqwest::Client::new()
        .request(method, format!("{manager}{path}"))
        .bearer_auth(TOKEN)
}

/// Starts the Tart runner `runner` in the background, as the reconciler does.
fn start_tart_runner(
    manager: &str,
    runner: &str,
) -> JoinHandle<anyhow::Result<(StatusCode, Value)>> {
    let request = request(manager, reqwest::Method::POST, "/v1/runners").json(&json!({
        "runnerId": runner,
        "poolId": "pool-mac",
        "name": format!("mac-{runner}"),
        "image": "gridops-macos-tahoe-base",
        "mode": "ephemeral",
        "provider": "tart",
        "platform": "github",
        "jitConfig": "j".repeat(64),
        "cpuLimit": 4.0,
        "memoryLimitMb": 4_096,
        "network": "gridops-runners",
        "capacityLease": format!("lease-{runner}"),
    }));
    tokio::spawn(async move {
        let response = request.send().await?;
        let status = response.status();
        Ok((
            status,
            response.json::<Value>().await.unwrap_or(Value::Null),
        ))
    })
}

/// Sends `request` and gives its status and JSON body, failing the test if
/// the manager has not answered within `NOT_BLOCKED`.
async fn answer_to(what: &str, request: reqwest::RequestBuilder) -> anyhow::Result<(u16, Value)> {
    let response = tokio::time::timeout(NOT_BLOCKED, request.send())
        .await
        .map_err(|_| {
            anyhow::anyhow!("{what} was still waiting after {NOT_BLOCKED:?} behind the Tart start")
        })??;
    let status = response.status().as_u16();
    Ok((
        status,
        response.json::<Value>().await.unwrap_or(Value::Null),
    ))
}

async fn eventually(what: &str, condition: impl Fn() -> bool) -> anyhow::Result<()> {
    let deadline = tokio::time::Instant::now() + NOT_BLOCKED;
    while !condition() {
        anyhow::ensure!(
            tokio::time::Instant::now() < deadline,
            "timed out waiting until {what}"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    Ok(())
}

#[tokio::test]
async fn docker_work_goes_ahead_while_a_tart_start_is_held() -> anyhow::Result<()> {
    let docker = stand_in_docker::start(stand_in_docker::Images::default())?;
    let tart = stand_in_tart().await?;
    let manager = manager(&docker, &tart.url, &["runner-mac-1"]).await?;

    let tart_start = start_tart_runner(&manager, "runner-mac-1");
    eventually("the Tart start reached the agent", || {
        tart.arrived.load(Ordering::SeqCst) == 1
    })
    .await?;

    // A Docker runner: admission, then the start with its lease.
    let (status, admitted) = answer_to(
        "a Docker admission",
        request(&manager, reqwest::Method::POST, "/v1/admissions").json(&json!({
            "runnerId": "runner-docker-1", "poolId": "pool-docker",
            "provider": "docker", "cpuLimit": 2.0, "memoryLimitMb": 2_048,
        })),
    )
    .await?;
    assert_eq!(status, 201, "{admitted}");
    let lease = admitted["leaseId"].as_str().unwrap_or_default().to_owned();
    let (status, started) = answer_to(
        "a Docker runner start",
        request(&manager, reqwest::Method::POST, "/v1/runners").json(&json!({
            "runnerId": "runner-docker-1", "poolId": "pool-docker",
            "name": "pool-docker-abcd1234", "image": "runner:fixture", "mode": "ephemeral",
            "provider": "docker", "platform": "github",
            "jitConfig": "fake-jit-credential-123456", "labels": ["self-hosted"],
            "cpuLimit": 2.0, "memoryLimitMb": 2_048, "network": "gridops-runners",
            "capacityLease": lease,
        })),
    )
    .await?;
    assert_eq!(status, 201, "{started}");
    assert_eq!(started["id"], CONTAINER_ID, "{started}");

    // Starting an existing runner container takes the same lock.
    let (status, controlled) = answer_to(
        "a Docker container start",
        request(
            &manager,
            reqwest::Method::POST,
            &format!("/v1/runners/{CONTAINER_ID}/start"),
        ),
    )
    .await?;
    assert_eq!(status, 200, "{controlled}");
    assert_eq!(controlled["status"], "running", "{controlled}");

    // An agent sandbox: admission under the sandbox pool, then creation.
    let (status, admitted) = answer_to(
        "a sandbox admission",
        request(&manager, reqwest::Method::POST, "/v1/admissions").json(&json!({
            "runnerId": SANDBOX_ID, "poolId": sandbox::SANDBOX_POOL_ID,
            "provider": "docker", "cpuLimit": 1.0, "memoryLimitMb": 512,
        })),
    )
    .await?;
    assert_eq!(status, 201, "{admitted}");
    let (status, created) = answer_to(
        "a sandbox creation",
        request(&manager, reqwest::Method::POST, "/v1/sandboxes").json(&json!({
            "sandboxId": SANDBOX_ID, "image": "sandbox:fixture",
            "cpuLimit": 1.0, "memoryLimitMb": 512, "network": "gridops-runners",
            "capacityLease": admitted["leaseId"],
        })),
    )
    .await?;
    assert_eq!(status, 201, "{created}");

    // All of that happened while the agent still held the Tart start.
    assert!(!tart_start.is_finished(), "the Tart start was not held");
    tart.open.send_replace(true);
    let (status, body) = tokio::time::timeout(NOT_BLOCKED, tart_start).await???;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    let _ = std::fs::remove_file(&docker);
    Ok(())
}

#[tokio::test]
async fn two_tart_starts_still_take_turns() -> anyhow::Result<()> {
    let docker = stand_in_docker::start(stand_in_docker::Images::default())?;
    let tart = stand_in_tart().await?;
    let manager = manager(&docker, &tart.url, &["runner-mac-1", "runner-mac-2"]).await?;

    let first = start_tart_runner(&manager, "runner-mac-1");
    eventually("the first Tart start reached the agent", || {
        tart.arrived.load(Ordering::SeqCst) == 1
    })
    .await?;
    let second = start_tart_runner(&manager, "runner-mac-2");
    // Long enough for the second start to reach the agent if nothing held it.
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert_eq!(
        tart.arrived.load(Ordering::SeqCst),
        1,
        "the second Tart start reached the agent while the first was still running"
    );
    assert!(!second.is_finished());

    tart.open.send_replace(true);
    for start in [first, second] {
        let (status, body) = tokio::time::timeout(NOT_BLOCKED, start).await???;
        assert_eq!(status, StatusCode::CREATED, "{body}");
    }
    assert_eq!(tart.arrived.load(Ordering::SeqCst), 2);
    assert_eq!(tart.most_at_once.load(Ordering::SeqCst), 1);
    let _ = std::fs::remove_file(&docker);
    Ok(())
}
