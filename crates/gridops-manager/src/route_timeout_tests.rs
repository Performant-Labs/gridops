//! The runner manager's time limits, end to end through the real router, with
//! a stand-in Tart agent that answers only after a set delay. Time is paused,
//! so the minutes these tests wait out pass at once.

use std::future::Future;

use super::*;

/// Runs `work` on paused time that moves only one second at a time, with a
/// real millisecond between steps for loopback I/O to land.
///
/// Left to itself, tokio jumps a paused clock straight to the next timer
/// whenever no task is ready, even while a request is still on its way
/// through a socket; the manager's time limit would then fire before the
/// request even reached the stand-in agent. A running blocking task stops
/// those jumps, so the clock moves only here.
async fn on_test_clock<F: Future>(work: F) -> F::Output {
    let (release, held) = std::sync::mpsc::channel::<()>();
    let hold = tokio::task::spawn_blocking(move || held.recv());
    let mut work = std::pin::pin!(work);
    let output = loop {
        tokio::select! {
            output = &mut work => break output,
            _ = tokio::task::spawn_blocking(|| std::thread::sleep(Duration::from_millis(1))) => {}
        }
        tokio::time::advance(Duration::from_secs(1)).await;
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
async fn stand_in_agent(delay: Duration, status: StatusCode, body: String) -> anyhow::Result<Url> {
    let answer = move || {
        let body = body.clone();
        async move {
            tokio::time::sleep(delay).await;
            (status, [(header::CONTENT_TYPE, "application/json")], body)
        }
    };
    let app = Router::new().route("/v1/runners", get(answer.clone()).post(answer));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let url = Url::parse(&format!("http://{}/", listener.local_addr()?))?;
    tokio::spawn(async move { axum::serve(listener, app).await });
    Ok(url)
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
                // Wall-clock time, which paused tokio time does not move.
                expires_at: Instant::now() + Duration::from_mins(5),
            },
        )]))),
        provisioning_paused: Arc::new(AtomicBool::new(false)),
        builtin_host: builtin_host::BuiltinHost::new(false, None),
        provision_lock: Arc::new(Mutex::new(())),
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
    let agent = stand_in_agent(agent_delay, agent_status, agent_body).await?;
    let manager = manager_in_front_of(agent).await?;
    let started = tokio::time::Instant::now();
    let response = reqwest::Client::new()
        .post(format!("{manager}/v1/runners"))
        .bearer_auth(TOKEN)
        .json(&tart_runner_request())
        .send()
        .await?;
    let status = response.status();
    let text = response.text().await?;
    let body = serde_json::from_str(&text).unwrap_or(Value::Null);
    Ok((status, body, started.elapsed()))
}

fn no_slot_body() -> String {
    json!({ "code": "vm_slot_unavailable", "error": NO_SLOT }).to_string()
}

#[tokio::test(start_paused = true)]
async fn the_default_slot_wait_reaches_the_caller_as_vm_slot_unavailable() -> anyhow::Result<()> {
    // What a real run did: the clone took about 4 s, then the agent waited
    // its default 180 s for a slot (now counted from the start request) and
    // spent a few seconds on its last `tart run`. The manager must still be
    // listening when the agent says why the start failed.
    let (status, body, _) = on_test_clock(start_tart_runner(
        Duration::from_secs(186),
        StatusCode::SERVICE_UNAVAILABLE,
        no_slot_body(),
    ))
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
    let (status, body, _) = on_test_clock(start_tart_runner(
        Duration::from_mins(14),
        StatusCode::SERVICE_UNAVAILABLE,
        no_slot_body(),
    ))
    .await?;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{body}");
    assert_eq!(body["code"], "vm_slot_unavailable");
    Ok(())
}

#[tokio::test(start_paused = true)]
async fn a_tart_start_past_the_ceiling_is_a_408_that_says_what_timed_out() -> anyhow::Result<()> {
    let (status, body, took) = on_test_clock(start_tart_runner(
        Duration::from_mins(20),
        StatusCode::SERVICE_UNAVAILABLE,
        no_slot_body(),
    ))
    .await?;
    assert_eq!(status, StatusCode::REQUEST_TIMEOUT, "{body}");
    assert_eq!(body["code"], "tart_start_timeout", "{body}");
    let message = body["error"].as_str().unwrap_or_default();
    assert!(
        message.contains("Tart agent") && message.contains("900 seconds"),
        "{message}"
    );
    // The clock moves in whole seconds, so allow one step either way.
    assert!((899..=901).contains(&took.as_secs()), "{took:?}");
    Ok(())
}

#[tokio::test(start_paused = true)]
async fn an_agent_error_without_a_message_is_still_explained() -> anyhow::Result<()> {
    let (status, body, took) = on_test_clock(start_tart_runner(
        Duration::ZERO,
        StatusCode::INTERNAL_SERVER_ERROR,
        String::new(),
    ))
    .await?;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "{body}");
    assert_eq!(body["code"], "tart_agent_error");
    let message = body["error"].as_str().unwrap_or_default();
    assert!(
        message.contains("Tart agent") && message.contains("500"),
        "{message}"
    );
    // At once, not at a time limit (the test clock steps a second at a time
    // while sockets settle).
    assert!(took < Duration::from_secs(30), "{took:?}");
    Ok(())
}

#[tokio::test(start_paused = true)]
async fn any_timed_out_request_says_what_timed_out() -> anyhow::Result<()> {
    // Listing runners asks the Tart agent too, under the standard limit.
    let agent = stand_in_agent(
        Duration::from_mins(5),
        StatusCode::OK,
        json!({ "runners": [] }).to_string(),
    )
    .await?;
    let manager = manager_in_front_of(agent).await?;
    let response = on_test_clock(
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
fn docker_runner_starts_keep_the_standard_limit() {
    assert_eq!(provision_time_limit("docker"), Duration::from_mins(3));
    assert_eq!(provision_time_limit("tart"), Duration::from_mins(15));
}
