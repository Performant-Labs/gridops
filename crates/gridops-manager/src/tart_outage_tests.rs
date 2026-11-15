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

/// The runner list as the reconciler and the API ask for it since #122:
/// opted in to a partial list (review finding 1 of PR #123).
fn get_runners(manager: &str) -> reqwest::RequestBuilder {
    reqwest::Client::new()
        .get(format!("{manager}/v1/runners?partial=true"))
        .bearer_auth(TOKEN)
}

/// The runner list as a reconciler or API from before #122 asks for it.
fn get_runners_without_opting_in(manager: &str) -> reqwest::RequestBuilder {
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
    assert_eq!(
        body["providerErrors"]["tart"]["kind"], "unreachable",
        "a refused connection is a network failure: {body}"
    );
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
    assert_eq!(
        body["providerErrors"]["tart"]["kind"], "unreachable",
        "a timed-out read is a network failure: {body}"
    );
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

// ---------------------------------------------------------------------------
// Round 2 (review of PR #123).
// ---------------------------------------------------------------------------

/// Review finding 1: the partial list is opt-in. A reconciler or API from
/// before #122, still running against this manager during a per-service
/// deploy or rollback, ignores `unavailableProviders` and takes every runner
/// missing from the list for gone: it would fail the Mac's runners,
/// deregister them and remove their VMs. So a caller that does not ask for
/// the partial list (`?partial=true`) gets the old answer: the whole list
/// fails, `503 tart_agent_unavailable`, and its pass is skipped as before.
#[tokio::test]
async fn a_caller_that_does_not_opt_in_gets_the_old_failed_list_while_tart_is_unavailable()
-> anyhow::Result<()> {
    let docker = listed_docker()?;
    let manager = manager(Some(&docker), &refused_url()?).await?;
    let response = get_runners_without_opting_in(&manager).send().await?;
    let status = response.status();
    let body = response.json::<Value>().await.unwrap_or(Value::Null);
    let _ = std::fs::remove_file(&docker);
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{body}");
    assert_eq!(body["code"], "tart_agent_unavailable", "{body}");
    Ok(())
}

/// Review finding 1: while every provider answers, opting in or not makes
/// no difference: an older caller keeps the whole list it always had.
#[tokio::test]
async fn a_caller_that_does_not_opt_in_still_gets_every_runner_while_tart_answers()
-> anyhow::Result<()> {
    let docker = listed_docker()?;
    let tart = stand_in_tart(false).await?;
    let manager = manager(Some(&docker), &tart.url).await?;
    let response = get_runners_without_opting_in(&manager).send().await?;
    let status = response.status();
    let body = response.json::<Value>().await.unwrap_or(Value::Null);
    let _ = std::fs::remove_file(&docker);
    assert_eq!(status, StatusCode::OK, "{body}");
    let mut runners = listed(&body);
    runners.sort();
    assert_eq!(
        runners,
        [
            (CONTAINER_ID.to_owned(), "docker".to_owned()),
            (TART_RUNNER.to_owned(), "tart".to_owned()),
        ],
        "{body}"
    );
    Ok(())
}

/// Review finding 3: compose checks the manager's health with
/// `curl --max-time 2`, and `api` waits for the manager to be healthy before
/// it starts. Health must therefore answer well inside two seconds while the
/// Mac drops packets, reporting Tart as unavailable (or what it last knew,
/// marked stale), or a sleeping Mac blocks every deploy.
const HEALTH_LIMIT: Duration = Duration::from_millis(1_500);

#[tokio::test(start_paused = true)]
async fn health_answers_within_the_compose_healthcheck_while_the_tart_agent_hangs()
-> anyhow::Result<()> {
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
    assert!(
        took < HEALTH_LIMIT,
        "health waited {took:?} for the agent; compose gives it 2 s: {body}"
    );
    let tart_health = &body["providers"]["tart"];
    assert!(
        tart_health["available"] == false || tart_health["stale"] == true,
        "a hung agent is reported available and current: {body}"
    );
    Ok(())
}

/// A stand-in Tart agent whose runner list answers `status` with `body`.
async fn answering_tart(status: StatusCode, body: &'static str) -> anyhow::Result<Url> {
    let app = Router::new().route(
        "/v1/runners",
        get(move || async move {
            (
                status,
                [(axum::http::header::CONTENT_TYPE, "application/json")],
                body,
            )
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let url = Url::parse(&format!("http://{}/", listener.local_addr()?))?;
    tokio::spawn(async move { axum::serve(listener, app).await });
    Ok(url)
}

/// A stand-in Tart agent that starts answering and drops the connection
/// halfway through the body, as when the Mac goes to sleep mid-answer.
async fn cut_off_tart() -> anyhow::Result<Url> {
    use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let url = Url::parse(&format!("http://{}/", listener.local_addr()?))?;
    tokio::spawn(async move {
        while let Ok((mut stream, _)) = listener.accept().await {
            tokio::spawn(async move {
                let mut request = [0_u8; 4_096];
                let _ = stream.read(&mut request).await;
                let _ = stream
                    .write_all(
                        b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 4096\r\n\r\n{\"runners\":[{\"id\":\"tart-",
                    )
                    .await;
                let _ = stream.shutdown().await;
            });
        }
    });
    Ok(url)
}

/// The partial list's account of why Tart could not be listed.
async fn tart_failure(tart: &Url) -> anyhow::Result<Value> {
    let docker = listed_docker()?;
    let manager = manager(Some(&docker), tart).await?;
    let response = get_runners(&manager).send().await?;
    let status = response.status();
    let body = response.json::<Value>().await.unwrap_or(Value::Null);
    let _ = std::fs::remove_file(&docker);
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(listed(&body), docker_runner_only(), "{body}");
    assert_eq!(body["unavailableProviders"], json!(["tart"]), "{body}");
    Ok(body["providerErrors"]["tart"].clone())
}

/// Review finding 5: a body cut off mid-answer is the Mac going away, a
/// network failure like a refused connection or a timeout.
#[tokio::test]
async fn a_tart_list_cut_off_mid_answer_is_reported_unreachable() -> anyhow::Result<()> {
    let failure = tart_failure(&cut_off_tart().await?).await?;
    assert_eq!(failure["kind"], "unreachable", "{failure}");
    Ok(())
}

/// Review finding 5: an agent that answers, but refuses the manager's token
/// or sends a list that does not parse, is not asleep: it is broken and
/// needs someone. The Docker runners are still listed, but the list tells
/// this apart from a network failure and says what went wrong.
#[tokio::test]
async fn a_tart_agent_that_refuses_the_token_or_answers_garbage_is_reported_as_an_error()
-> anyhow::Result<()> {
    let refused = tart_failure(
        &answering_tart(
            StatusCode::UNAUTHORIZED,
            r#"{"code":"unauthorized","error":"invalid token"}"#,
        )
        .await?,
    )
    .await?;
    assert_eq!(refused["kind"], "error", "{refused}");
    assert!(
        refused["message"]
            .as_str()
            .is_some_and(|message| message.contains("401")),
        "the reason does not say the agent refused the manager: {refused}"
    );

    let garbage =
        tart_failure(&answering_tart(StatusCode::OK, r#"{"runners":"none"}"#).await?).await?;
    assert_eq!(garbage["kind"], "error", "{garbage}");
    assert!(
        garbage["message"]
            .as_str()
            .is_some_and(|message| !message.is_empty()),
        "{garbage}"
    );
    Ok(())
}

// ---------------------------------------------------------------------------
// Round 3 (second review of PR #123).
// ---------------------------------------------------------------------------

/// Round 3, finding 3, the manager's side: its message for an agent it
/// cannot reach is passed on, by the API, to users who may only read pools,
/// and by the reconciler into pool reasons and events. It must not name the
/// agent's internal URL, host or port (reqwest's error does: `error sending
/// request for url (http://<mac>:8787/...)`); that detail goes to the log.
/// Through the partial list's `providerErrors` and a removal's `503`.
#[tokio::test]
async fn the_tart_agents_address_is_not_in_what_the_manager_answers_while_it_is_unavailable()
-> anyhow::Result<()> {
    let tart = refused_url()?;
    let address = tart.authority().to_owned();
    let docker = listed_docker()?;
    let manager = manager(Some(&docker), &tart).await?;

    let listed = get_runners(&manager).send().await?;
    let list = listed.json::<Value>().await.unwrap_or(Value::Null);
    let removal = reqwest::Client::new()
        .delete(format!("{manager}/v1/runners/{TART_RUNNER}"))
        .bearer_auth(TOKEN)
        .send()
        .await?;
    let removal_status = removal.status();
    let removed = removal.json::<Value>().await.unwrap_or(Value::Null);
    let _ = std::fs::remove_file(&docker);

    assert_eq!(
        list["providerErrors"]["tart"]["kind"], "unreachable",
        "{list}"
    );
    let message = list["providerErrors"]["tart"]["message"]
        .as_str()
        .unwrap_or_default();
    assert!(
        !message.is_empty() && !message.contains(&address) && !message.contains("http://"),
        "the list names the agent's address ({address}): {message:?}"
    );
    assert_eq!(removal_status, StatusCode::SERVICE_UNAVAILABLE, "{removed}");
    assert_eq!(removed["code"], "tart_agent_unavailable", "{removed}");
    let error = removed["error"].as_str().unwrap_or_default();
    assert!(
        !error.is_empty() && !error.contains(&address) && !error.contains("http://"),
        "the removal's error names the agent's address ({address}): {error:?}"
    );
    Ok(())
}

/// An address nothing answers: TEST-NET-1 (RFC 5737), reserved for
/// documentation and never assigned, so a connection attempt there is
/// dropped without an answer, as a sleeping Mac drops it.
const DROPPING_AGENT: &str = "http://192.0.2.1:8787/";

/// Round 3, finding 6: a Mac that sleeps drops packets, so connecting to it
/// never ends by itself. A VM start, a removal and a log read carry no
/// per-call limit of their own (only reads do), so only the client's connect
/// limit stops them hanging until the 15-minute start limit or the 45-minute
/// client limit. Through the client the manager really builds for the agent
/// (`tart_agent_client`, extracted from `tart_agent_from_environment` so a
/// test can build it), not a copy of its settings. On the paused clock,
/// which moves to the next timer at once: each call must fail as
/// `tart_agent_unavailable` after about the 5-second connect limit. A call
/// that failed at once (no route to the address at all) would prove nothing,
/// hence the lower bound.
#[tokio::test(start_paused = true)]
async fn a_start_removal_or_log_read_to_an_agent_that_drops_packets_fails_within_the_connect_limit()
-> anyhow::Result<()> {
    let agent = TartAgent {
        base_url: Url::parse(DROPPING_AGENT)?,
        token: SecretString::from("tart-agent-token"),
        http: tart_agent_client()?,
    };
    let start = json!({ "runnerId": "runner-mac-1", "name": "mac-abcd1234" });
    let logs = format!("v1/runners/{TART_RUNNER}/logs?tail=100&follow=false");
    let removal = format!("v1/runners/{TART_RUNNER}");
    for call in ["start", "removal", "logs"] {
        let started = tokio::time::Instant::now();
        let result = match call {
            "start" => tart_agent_value(
                &agent,
                reqwest::Method::POST,
                "v1/runners",
                Some(start.clone()),
            )
            .await
            .map(|_| ()),
            "removal" => tart_agent_value(&agent, reqwest::Method::DELETE, &removal, None)
                .await
                .map(|_| ()),
            _ => tart_agent_response(&agent, reqwest::Method::GET, &logs, None, None)
                .await
                .map(|_| ()),
        };
        let took = started.elapsed();
        match result {
            Err(ManagerError::Upstream { code, .. }) => {
                assert_eq!(code, "tart_agent_unavailable", "the {call}");
            }
            other => panic!("the {call} to an agent that drops packets gave {other:?}"),
        }
        assert!(
            took >= Duration::from_secs(1) && took <= FAIL_FAST,
            "the {call} to an agent that drops packets failed after {took:?}, not within the connect limit"
        );
    }
    Ok(())
}
