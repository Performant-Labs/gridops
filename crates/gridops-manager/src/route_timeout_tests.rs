//! The runner manager's time limits, end to end through the real router, with
//! a stand-in Tart agent that answers only after a set delay. Time is paused,
//! so the minutes these tests wait out pass at once.

use std::{future::Future, sync::atomic::AtomicUsize};

use super::*;

/// Requests the stand-in agent is holding: received, not yet answered.
type Holding = Arc<AtomicUsize>;

/// Runs `work` on paused time that moves, a second at a time, only while the
/// stand-in agent holds a request, so time passes only for the delay under
/// test and never while a request is still on its way through a socket.
///
/// Left to itself, tokio jumps a paused clock straight to the next timer
/// whenever no task is ready, even while loopback I/O is in flight; the
/// manager's time limit would then fire before the request even reached the
/// stand-in. A running blocking task stops those jumps, so the clock moves
/// only here. Between steps a real millisecond lets sockets settle.
async fn on_test_clock<F: Future>(holding: &Holding, work: F) -> F::Output {
    let (release, held) = std::sync::mpsc::channel::<()>();
    let hold = tokio::task::spawn_blocking(move || held.recv());
    let mut work = std::pin::pin!(work);
    let output = loop {
        tokio::select! {
            output = &mut work => break output,
            _ = tokio::task::spawn_blocking(|| std::thread::sleep(Duration::from_millis(1))) => {}
        }
        if holding.load(Ordering::SeqCst) > 0 {
            tokio::time::advance(Duration::from_secs(1)).await;
        }
    };
    drop(release);
    let _ = hold.await;
    output
}

const TOKEN: &str = "route-timeout-test-token";
const LEASE: &str = "lease-tart-1";

/// What the Tart agent answers when no VM slot freed up within its wait.
const NO_SLOT: &str = "no macOS VM slot became free within 180 seconds; the Mac allows two VMs at once, including VMs of other software";

/// A stand-in Tart agent: `POST /v1/runners` and `GET /v1/runners` answer
/// `status` with `body` after `delay`.
async fn stand_in_agent(
    delay: Duration,
    status: StatusCode,
    body: String,
) -> anyhow::Result<(Url, Holding)> {
    let holding = Holding::default();
    let answer = {
        let holding = holding.clone();
        move || {
            let body = body.clone();
            let holding = holding.clone();
            async move {
                holding.fetch_add(1, Ordering::SeqCst);
                tokio::time::sleep(delay).await;
                holding.fetch_sub(1, Ordering::SeqCst);
                (status, [(header::CONTENT_TYPE, "application/json")], body)
            }
        }
    };
    let app = Router::new().route("/v1/runners", get(answer.clone()).post(answer));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let url = Url::parse(&format!("http://{}/", listener.local_addr()?))?;
    tokio::spawn(async move { axum::serve(listener, app).await });
    Ok((url, holding))
}

/// The real manager router in front of `agent`, holding a capacity lease for
/// the Tart runner that `tart_runner_request` starts.
async fn manager_in_front_of(agent: Url) -> anyhow::Result<String> {
    // Nothing listens here: these tests never reach Docker.
    let socket = std::env::temp_dir().join(format!(
        "gridops-route-timeout-test-{}.sock",
        uuid::Uuid::new_v4()
    ));
    std::fs::write(&socket, "")?;
    let docker =
        Docker::connect_with_socket(&socket.to_string_lossy(), 2, bollard::API_DEFAULT_VERSION)?;
    let state = ManagerState {
        docker,
        tart: Some(TartAgent {
            base_url: agent,
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
            min_free_disk_mb: 1_024,
            min_free_disk_percent: 10,
            runner_network: "gridops-runners".into(),
            log_max_size: "20m".into(),
            log_max_files: 5,
            runner_pids_limit: 1_024,
            runner_swap_percent: 100,
            budget_origin: BudgetOrigin::default(),
        },
        shared_docker_socket: None,
        reservations: Arc::new(Mutex::new(HashMap::from([(
            LEASE.to_owned(),
            CapacityReservation {
                runner_id: "runner-1".into(),
                pool_id: "pool-1".into(),
                provider: "tart".into(),
                cpu_limit: 4.0,
                memory_limit_mb: 4_096,
                vm_slot: true,
                // Wall-clock time, which paused tokio time does not move.
                expires_at: Instant::now() + Duration::from_mins(5),
            },
        )]))),
        provisioning_paused: Arc::new(AtomicBool::new(false)),
        builtin_host: builtin_host::BuiltinHost::new(false, None),
        provision_locks: ProvisionLocks::default(),
        vm_slots: Arc::default(),
    };
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let base_url = format!("http://{}", listener.local_addr()?);
    let app = router(state);
    tokio::spawn(async move { axum::serve(listener, app).await });
    Ok(base_url)
}

fn tart_runner_request() -> Value {
    json!({
        "runnerId": "runner-1",
        "poolId": "pool-1",
        "name": "homelab-abcd1234",
        "image": "gridops-macos-tahoe-base",
        "mode": "ephemeral",
        "provider": "tart",
        "platform": "github",
        "jitConfig": "j".repeat(64),
        "cpuLimit": 4.0,
        "memoryLimitMb": 4_096,
        "network": "gridops-runners",
        "capacityLease": LEASE,
    })
}

/// Starts the Tart runner through a manager whose agent answers after
/// `agent_delay`, as the reconciler does, and returns the manager's status,
/// its JSON body (`null` when it has none) and how long it took.
async fn start_tart_runner(
    agent_delay: Duration,
    agent_status: StatusCode,
    agent_body: String,
) -> anyhow::Result<(StatusCode, Value, Duration)> {
    let (agent, holding) = stand_in_agent(agent_delay, agent_status, agent_body).await?;
    let manager = manager_in_front_of(agent).await?;
    let started = tokio::time::Instant::now();
    let (status, text) = on_test_clock(&holding, async {
        let response = reqwest::Client::new()
            .post(format!("{manager}/v1/runners"))
            .bearer_auth(TOKEN)
            .json(&tart_runner_request())
            .send()
            .await?;
        let status = response.status();
        anyhow::Ok((status, response.text().await?))
    })
    .await?;
    let body = serde_json::from_str(&text).unwrap_or(Value::Null);
    Ok((status, body, started.elapsed()))
}

fn no_slot_body() -> String {
    json!({ "code": "vm_slot_unavailable", "error": NO_SLOT }).to_string()
}

#[tokio::test(start_paused = true)]
async fn the_default_slot_wait_reaches_the_caller_as_vm_slot_unavailable() -> anyhow::Result<()> {
    // What a real run did: the clone took about 4 s, then the agent waited
    // 180 s for a slot (its default then; 60 s since 0.2.0, so the default
    // now fails sooner than this), counted from the start request, and spent
    // a few seconds on its last `tart run`. The manager must still be
    // listening when the agent says why the start failed.
    let (status, body, _) = start_tart_runner(
        Duration::from_secs(186),
        StatusCode::SERVICE_UNAVAILABLE,
        no_slot_body(),
    )
    .await?;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{body}");
    assert_eq!(body["code"], "vm_slot_unavailable");
    assert_eq!(body["error"], NO_SLOT);
    Ok(())
}

#[tokio::test(start_paused = true)]
async fn the_longest_start_the_tart_agent_allows_still_fits() -> anyhow::Result<()> {
    // The agent's longest slot wait (600 s, counted from the start request),
    // then a full guest boot (180 s) and a minute to start the runner:
    // 14 minutes.
    let (status, body, _) = start_tart_runner(
        Duration::from_mins(14),
        StatusCode::SERVICE_UNAVAILABLE,
        no_slot_body(),
    )
    .await?;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{body}");
    assert_eq!(body["code"], "vm_slot_unavailable");
    Ok(())
}

#[tokio::test(start_paused = true)]
async fn a_tart_start_past_the_ceiling_is_a_408_that_says_what_timed_out() -> anyhow::Result<()> {
    let (status, body, took) = start_tart_runner(
        Duration::from_mins(20),
        StatusCode::SERVICE_UNAVAILABLE,
        no_slot_body(),
    )
    .await?;
    assert_eq!(status, StatusCode::REQUEST_TIMEOUT, "{body}");
    assert_eq!(body["code"], "tart_start_timeout", "{body}");
    let message = body["error"].as_str().unwrap_or_default();
    assert!(
        message.contains("Tart agent") && message.contains("900 seconds"),
        "{message}"
    );
    // The clock moves in whole seconds.
    assert!((900..=901).contains(&took.as_secs()), "{took:?}");
    Ok(())
}

#[tokio::test(start_paused = true)]
async fn an_agent_error_without_a_message_is_still_explained() -> anyhow::Result<()> {
    let (status, body, took) = start_tart_runner(
        Duration::ZERO,
        StatusCode::INTERNAL_SERVER_ERROR,
        String::new(),
    )
    .await?;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "{body}");
    assert_eq!(body["code"], "tart_agent_error");
    let message = body["error"].as_str().unwrap_or_default();
    assert!(
        message.contains("Tart agent") && message.contains("500"),
        "{message}"
    );
    assert_eq!(took, Duration::ZERO);
    Ok(())
}

#[tokio::test(start_paused = true)]
async fn any_timed_out_request_says_what_timed_out() -> anyhow::Result<()> {
    // Listing runners asks the Tart agent too, under the standard limit.
    let (agent, holding) = stand_in_agent(
        Duration::from_mins(5),
        StatusCode::OK,
        json!({ "runners": [] }).to_string(),
    )
    .await?;
    let manager = manager_in_front_of(agent).await?;
    let response = on_test_clock(
        &holding,
        reqwest::Client::new()
            .get(format!("{manager}/v1/runners"))
            .bearer_auth(TOKEN)
            .send(),
    )
    .await?;
    assert_eq!(response.status(), StatusCode::REQUEST_TIMEOUT);
    let body = response.json::<Value>().await?;
    assert_eq!(body["code"], "request_timeout", "{body}");
    let message = body["error"].as_str().unwrap_or_default();
    assert!(
        message.contains("GET /v1/runners") && message.contains("180 seconds"),
        "{message}"
    );
    Ok(())
}

#[test]
fn a_docker_runner_start_has_time_for_its_image_pull() {
    // The pull may take `IMAGE_PULL_TIMEOUT`, and the container start
    // follows it.
    assert_eq!(IMAGE_PULL_TIMEOUT, Duration::from_mins(10));
    assert_eq!(provision_time_limit("docker"), Duration::from_mins(12));
    assert!(provision_time_limit("docker") > IMAGE_PULL_TIMEOUT);
    assert_eq!(SANDBOX_CREATE_TIMEOUT, provision_time_limit("docker"));
    assert_eq!(provision_time_limit("tart"), Duration::from_mins(15));
    // The route's backstop leaves the handler time to say what timed out.
    assert_eq!(PROVISION_ROUTE_TIMEOUT, Duration::from_mins(16));
}

const DOCKER_LEASE: &str = "lease-docker-1";

/// The real manager router in front of `docker`, a stand-in engine, holding a
/// capacity lease for the Docker runner that `docker_runner_request` starts.
async fn manager_in_front_of_docker(docker: &std::path::Path) -> anyhow::Result<String> {
    let state = ManagerState {
        docker: Docker::connect_with_socket(
            &docker.to_string_lossy(),
            120,
            bollard::API_DEFAULT_VERSION,
        )?,
        tart: None,
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
        reservations: Arc::new(Mutex::new(HashMap::from([(
            DOCKER_LEASE.to_owned(),
            CapacityReservation {
                runner_id: "runner-docker-1".into(),
                pool_id: "pool-docker".into(),
                provider: "docker".into(),
                cpu_limit: 2.0,
                memory_limit_mb: 2_048,
                vm_slot: false,
                // Wall-clock time, which paused tokio time does not move.
                expires_at: Instant::now() + Duration::from_mins(5),
            },
        )]))),
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

fn docker_runner_request() -> Value {
    json!({
        "runnerId": "runner-docker-1",
        "poolId": "pool-docker",
        "name": "pool-docker-abcd1234",
        "image": "runner:fixture",
        "mode": "ephemeral",
        "provider": "docker",
        "platform": "github",
        "jitConfig": "j".repeat(64),
        "cpuLimit": 2.0,
        "memoryLimitMb": 2_048,
        "network": "gridops-runners",
        "capacityLease": DOCKER_LEASE,
    })
}

#[tokio::test(start_paused = true)]
async fn a_docker_runner_start_with_a_long_image_pull_still_starts() -> anyhow::Result<()> {
    // The image is not on the host, and pulling it takes nine and a half
    // minutes: well past the three minutes a Docker start used to get, inside
    // the pull's own ten.
    let images = crate::stand_in_docker::Images {
        pull: Some(Duration::from_secs(570)),
        ..Default::default()
    };
    let docker = crate::stand_in_docker::start(images.clone())?;
    let manager = manager_in_front_of_docker(&docker).await?;
    let started = tokio::time::Instant::now();
    let (status, text) = on_test_clock(&images.pulling, async {
        let response = reqwest::Client::new()
            .post(format!("{manager}/v1/runners"))
            .bearer_auth(TOKEN)
            .json(&docker_runner_request())
            .send()
            .await?;
        let status = response.status();
        anyhow::Ok((status, response.text().await?))
    })
    .await?;
    let _ = std::fs::remove_file(&docker);
    let body = serde_json::from_str::<Value>(&text).unwrap_or(Value::Null);
    assert_eq!(status, StatusCode::CREATED, "{text}");
    assert_eq!(body["id"], crate::stand_in_docker::CONTAINER_ID, "{body}");
    // The clock moved for the pull, and only for it.
    assert!(
        (570..=571).contains(&started.elapsed().as_secs()),
        "{:?}",
        started.elapsed()
    );
    Ok(())
}
