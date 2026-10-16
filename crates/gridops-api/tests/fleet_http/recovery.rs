//! A host recovers by itself when a command's deadline passes without its
//! answer. The real agent talks to the API binary through a proxy that loses
//! replies the way a failing API or network does; nothing repairs the
//! database by hand.

use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use axum::{body::Body, extract::State, http::Request, response::IntoResponse};
use gridops_core::fleet::{
    ids::PlacementId,
    placement,
    protocol::agent::{AgentOperation, LocalResourceIdentity},
};
use gridops_host_agent::supervisor::Stop;
use secrecy::ExposeSecret as _;

use super::{
    placement::{
        FakeDocker, active_machine, active_machine_on, issue_start, placement_state,
        prepared_placement, seed_pool,
    },
    *,
};

/// Replies the proxy loses.
#[derive(Default)]
struct Faults {
    /// Result posts fail before they reach the API.
    refuse_results: AtomicBool,
    /// The next poll reply carrying a command reaches the API, which records
    /// the command as delivered, but never the agent.
    swallow_command: AtomicBool,
}

struct Proxy {
    origin: String,
    faults: Arc<Faults>,
}

impl Proxy {
    async fn start(upstream: &str) -> Result<Self> {
        let faults = Arc::new(Faults::default());
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0)).await?;
        let origin = format!("http://{}", listener.local_addr()?);
        let state = (Arc::clone(&faults), upstream.to_owned(), Client::new());
        let app = axum::Router::new()
            .fallback(forward)
            .with_state(Arc::new(state));
        tokio::spawn(async move { axum::serve(listener, app).await });
        Ok(Self { origin, faults })
    }
}

async fn forward(
    State(state): State<Arc<(Arc<Faults>, String, Client)>>,
    request: Request<Body>,
) -> axum::response::Response {
    let (faults, upstream, client) = state.as_ref();
    let unavailable = || axum::http::StatusCode::SERVICE_UNAVAILABLE.into_response();
    let path = request
        .uri()
        .path_and_query()
        .map_or("/", |path| path.as_str())
        .to_owned();
    if path.ends_with("/commands/result") && faults.refuse_results.load(Ordering::SeqCst) {
        return unavailable();
    }
    let mut forwarded = client.request(request.method().clone(), format!("{upstream}{path}"));
    for (name, value) in request.headers() {
        if name != header::HOST {
            forwarded = forwarded.header(name, value);
        }
    }
    let Ok(body) = axum::body::to_bytes(request.into_body(), 4 * 1024 * 1024).await else {
        return unavailable();
    };
    let Ok(response) = forwarded.body(body).send().await else {
        return unavailable();
    };
    let status = response.status();
    let headers = response.headers().clone();
    let Ok(bytes) = response.bytes().await else {
        return unavailable();
    };
    if path.ends_with("/commands/poll")
        && serde_json::from_slice::<Value>(&bytes)
            .is_ok_and(|body| body["commands"].as_array().is_some_and(|c| !c.is_empty()))
        && faults.swallow_command.swap(false, Ordering::SeqCst)
    {
        return unavailable();
    }
    let mut reply = (status, bytes).into_response();
    for name in [header::CONTENT_TYPE, header::CACHE_CONTROL] {
        if let Some(value) = headers.get(&name) {
            reply.headers_mut().insert(name, value.clone());
        }
    }
    reply
}

/// The agent backs off for up to half a minute between reconnects.
async fn within<T, F, Fut>(what: &str, mut probe: F) -> Result<T>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<Option<T>>>,
{
    for _ in 0..450 {
        if let Some(value) = probe().await? {
            return Ok(value);
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    bail!("timed out waiting for {what}")
}

async fn has_authority(f: &Fixture, host: &str) -> Result<Option<()>> {
    let expires: Option<i64> =
        sqlx::query_scalar("SELECT authority_expires_at FROM fleet_hosts WHERE id=?")
            .bind(host)
            .fetch_one(&f.database)
            .await?;
    Ok(expires
        .is_some_and(|expires| expires > gridops_core::now_millis())
        .then_some(()))
}

async fn allocation_states(f: &Fixture, placement: PlacementId) -> Result<Vec<String>> {
    Ok(sqlx::query_scalar(
        "SELECT DISTINCT state FROM capacity_allocations WHERE placement_id=? ORDER BY state",
    )
    .bind(placement.to_string())
    .fetch_all(&f.database)
    .await?)
}

/// The 2026-10-14 lockout: the agent ran a start, but the API failed its
/// result post, and the deadline passed before the agent got through again.
/// The placement went uncertain and the host lost work authority for good.
#[tokio::test]
async fn host_recovers_when_a_started_runner_result_is_lost_past_its_deadline() -> Result<()> {
    let f = Fixture::new().await?;
    seed_pool(&f).await?;
    let proxy = Proxy::start(&f.origin).await?;
    let runtime = Arc::new(FakeDocker::default());
    let machine = active_machine(&f, &proxy.origin, Arc::clone(&runtime)).await?;
    let host = machine.host.to_string();
    let workload = Uuid::new_v4();
    let placement_id = prepared_placement(&f, &machine, workload).await?;

    proxy.faults.refuse_results.store(true, Ordering::SeqCst);
    issue_start(&f, placement_id, workload, 2_000).await?;
    within("the start to run", || async {
        Ok(runtime
            .operations
            .lock()
            .map_err(|_| anyhow::anyhow!("poisoned"))?
            .contains(&"start_runner")
            .then_some(()))
    })
    .await?;
    tokio::time::sleep(Duration::from_millis(2_500)).await;
    // Any poll for the host now finds the start past its deadline without
    // an answer: the placement is uncertain, which withholds work authority.
    let identity = machine.journal.identity().await?.context("identity")?;
    let poll = inventory::agent_post(
        &f,
        &host,
        identity.credential.expose_secret(),
        "commands/poll",
    )
    .json(&json!({"authority_session_id":Uuid::new_v4(),"wait_ms":0,"limit":1}))
    .send()
    .await?;
    assert_eq!(poll.status(), StatusCode::OK);
    assert_eq!(placement_state(&f, placement_id).await?, "uncertain");

    // Once its posts get through, the agent delivers the journaled result
    // before anything else, the start counts as done, and the host works.
    proxy.faults.refuse_results.store(false, Ordering::SeqCst);
    within("the placement to run", || async {
        Ok((placement_state(&f, placement_id).await? == "running").then_some(()))
    })
    .await?;
    assert_eq!(allocation_states(&f, placement_id).await?, ["committed"]);
    within("work authority", || has_authority(&f, &host)).await?;

    let _ = machine.stop.send(());
    assert_eq!(machine.running.await??, Stop::Shutdown);
    Ok(())
}

/// The 2026-10-14 21:05 lockout: the API recorded a command as delivered,
/// but its poll reply never reached the agent. It could never answer, so the
/// command went uncertain at its deadline.
#[tokio::test]
async fn host_recovers_when_a_delivered_command_never_reaches_it() -> Result<()> {
    let f = Fixture::new().await?;
    seed_pool(&f).await?;
    let proxy = Proxy::start(&f.origin).await?;
    let runtime = Arc::new(FakeDocker::default());
    let machine = active_machine(&f, &proxy.origin, Arc::clone(&runtime)).await?;
    let host = machine.host.to_string();
    let workload = Uuid::new_v4();
    let placement_id = prepared_placement(&f, &machine, workload).await?;

    proxy.faults.swallow_command.store(true, Ordering::SeqCst);
    issue_start(&f, placement_id, workload, 30_000).await?;
    // The lost reply ends the agent's session. Reconnecting, it reports
    // that it holds no other results, so the start never ran: its capacity
    // is released and the runner fails, long before the deadline.
    within("the unreceived start to settle", || async {
        Ok((placement_state(&f, placement_id).await? == "failed").then_some(()))
    })
    .await?;
    assert!(!proxy.faults.swallow_command.load(Ordering::SeqCst));
    let command: String = sqlx::query_scalar(
        "SELECT state FROM host_commands WHERE placement_id=? AND kind='start_runner'",
    )
    .bind(placement_id.to_string())
    .fetch_one(&f.database)
    .await?;
    assert_eq!(command, "cancelled");
    assert_eq!(allocation_states(&f, placement_id).await?, ["released"]);
    assert!(
        !runtime
            .operations
            .lock()
            .map_err(|_| anyhow::anyhow!("poisoned"))?
            .contains(&"start_runner")
    );
    within("work authority", || has_authority(&f, &host)).await?;

    let _ = machine.stop.send(());
    assert_eq!(machine.running.await??, Stop::Shutdown);
    Ok(())
}

/// The 2026-10-16 deploy: the API restarted while a machine's runner was
/// mid-job, with an inspect delivered but its answer not yet in. The agent's
/// long poll drops, it reconnects to the new process with a new session and
/// hands over the journaled answer. Nothing reads the restart as the runner
/// exiting: the placement keeps running, its capacity stays committed, and
/// the runtime is never told to stop or clean it up.
#[tokio::test]
async fn an_api_restart_leaves_a_running_runner_alone() -> Result<()> {
    let mut f = Fixture::new().await?;
    seed_pool(&f).await?;
    let proxy = Proxy::start(&f.origin).await?;
    let runtime = Arc::new(FakeDocker::default());
    runtime.alive.store(true, Ordering::SeqCst);
    let machine = active_machine(&f, &proxy.origin, Arc::clone(&runtime)).await?;
    let host = machine.host.to_string();
    let workload = Uuid::new_v4();
    let placement_id = prepared_placement(&f, &machine, workload).await?;
    issue_start(&f, placement_id, workload, 30_000).await?;
    within("the runner to start", || async {
        Ok((placement_state(&f, placement_id).await? == "running").then_some(()))
    })
    .await?;
    let database = f.database.clone();
    let session = || async {
        Ok::<_, anyhow::Error>(
            sqlx::query_scalar::<_, Option<String>>(
                "SELECT agent_session_id FROM fleet_hosts WHERE id=?",
            )
            .bind(&host)
            .fetch_one(&database)
            .await?,
        )
    };
    let before = session().await?.context("agent session")?;

    proxy.faults.refuse_results.store(true, Ordering::SeqCst);
    placement::issue_command(
        &f.database,
        placement_id,
        AgentOperation::Inspect {
            expected_resource: LocalResourceIdentity::Docker {
                object_id: gridops_core::fleet::ids::UpstreamText::new("fake-container")?,
            },
        },
        "inspect:before-restart",
        25_000,
    )
    .await?
    .context("inspect command")?;
    let inspected = || async {
        Ok::<_, anyhow::Error>(
            sqlx::query_scalar::<_, String>(
                "SELECT state FROM host_commands WHERE placement_id=? AND kind='inspect'",
            )
            .bind(placement_id.to_string())
            .fetch_one(&database)
            .await?,
        )
    };
    within("the inspect to reach the agent", || async {
        Ok((inspected().await? == "delivered").then_some(()))
    })
    .await?;

    f.restart(Duration::from_secs(2)).await?;
    proxy.faults.refuse_results.store(false, Ordering::SeqCst);
    within("the agent to reconnect", || async {
        let current = session().await?;
        Ok((current.is_some_and(|current| current != before)
            && has_authority(&f, &host).await?.is_some())
        .then_some(()))
    })
    .await?;
    within("the journaled inspect answer", || async {
        Ok((inspected().await? == "succeeded").then_some(()))
    })
    .await?;
    assert_eq!(placement_state(&f, placement_id).await?, "running");
    assert!(!placement::reported_gone(&f.database, placement_id, "inspect").await?);
    assert_eq!(allocation_states(&f, placement_id).await?, ["committed"]);
    assert_eq!(
        *runtime
            .operations
            .lock()
            .map_err(|_| anyhow::anyhow!("poisoned"))?,
        ["prepare_environment", "start_runner", "inspect"]
    );

    let _ = machine.stop.send(());
    assert_eq!(machine.running.await??, Stop::Shutdown);
    Ok(())
}

/// Work isn't left waiting on a machine without authority: a job any Linux
/// machine could run goes to the platform whose machine can take it, not
/// simply the pool's first Linux platform.
#[tokio::test]
async fn queued_work_goes_to_a_platform_with_an_authorized_machine() -> Result<()> {
    use gridops_core::fleet::targets::{choose_platform, pool_profiles};
    let f = Fixture::new().await?;
    seed_pool(&f).await?;
    // The pool's first platform is Linux x64, whose machine is away; the
    // machine here offers Linux arm64.
    let machine = active_machine_on(
        &f,
        &f.origin,
        Arc::new(FakeDocker::default()),
        json!([
            {"runtimeKind": "docker", "executionOs": "linux", "architecture": "x64", "image": "runner:fixture"},
            {"runtimeKind": "docker", "executionOs": "linux", "architecture": "arm64", "image": "runner:fixture"},
        ]),
    )
    .await?;
    sqlx::raw_sql(
        "UPDATE runner_pools SET autoscaling_enabled=1,paused=0 WHERE id='fleet-pool';
         INSERT INTO workflow_runs (id,repository_id,workflow_name,run_number,event,status,head_sha,html_url,github_created_at,github_updated_at,created_at,updated_at)
           VALUES (80,10,'CI',8,'push','queued','abc','https://github.com/octo-org/gridops/actions/runs/80',1,1,1,1);
         INSERT INTO workflow_jobs (id,run_id,name,status,labels,html_url,created_at,updated_at)
           VALUES (800,80,'build','queued','[\"self-hosted\",\"linux\"]','https://github.com/octo-org/gridops/actions/runs/80/job/800',1,1);",
    )
    .execute(&f.database)
    .await?;
    let profiles = pool_profiles(&f.database, "fleet-pool").await?;
    let configured = ["self-hosted".to_owned()];
    let choose = || choose_platform(&f.database, "fleet-pool", &configured, 1, &profiles);
    let (chosen, behind) = choose().await?.context("platform")?;
    assert_eq!(
        (chosen.id.as_str(), behind),
        (machine.profile_id.as_str(), true)
    );

    // With no machine able to take it anywhere, the job counts for the first
    // platform that can run it, as before.
    let _ = machine.stop.send(());
    assert_eq!(machine.running.await??, Stop::Shutdown);
    sqlx::query("UPDATE fleet_hosts SET authority_expires_at=NULL")
        .execute(&f.database)
        .await?;
    let (chosen, behind) = choose().await?.context("platform")?;
    assert_eq!(
        chosen.input.architecture,
        gridops_core::fleet::Architecture::X64
    );
    assert!(behind);
    Ok(())
}

/// A platform chosen for one repository's jobs doesn't take another's. With
/// long-waiting jobs pinned to x64 and a new job any Linux machine runs, x64
/// goes first, but its runners go only to the x64 repository: the other job
/// is arm64's, and an x64 runner for it would wait on the x64 machine while
/// the arm64 one sat idle (homelab, 2026-10-16).
#[tokio::test]
async fn a_platform_takes_only_the_jobs_counted_for_it() -> Result<()> {
    use gridops_core::fleet::targets::{choose_platform, platform_jobs, pool_profiles};
    let f = Fixture::new().await?;
    seed_pool(&f).await?;
    sqlx::raw_sql(
        "INSERT INTO repositories (id,installation_id,owner,name,full_name,private,default_branch,html_url,last_synced_at,created_at,updated_at)
           VALUES (11,1,'octo-org','sift','octo-org/sift',0,'main','https://github.com/octo-org/sift',1,1,1);
         INSERT INTO runner_pool_repositories (pool_id,repository_id,created_at) VALUES ('fleet-pool',11,1);",
    )
    .execute(&f.database)
    .await?;
    let machine = active_machine_on(
        &f,
        &f.origin,
        Arc::new(FakeDocker::default()),
        json!([
            {"runtimeKind": "docker", "executionOs": "linux", "architecture": "arm64", "image": "runner:fixture"},
            {"runtimeKind": "docker", "executionOs": "linux", "architecture": "x64", "image": "runner:fixture"},
        ]),
    )
    .await?;
    sqlx::raw_sql(
        "UPDATE runner_pools SET autoscaling_enabled=1,paused=0 WHERE id='fleet-pool';
         INSERT INTO workflow_runs (id,repository_id,workflow_name,run_number,event,status,head_sha,html_url,github_created_at,github_updated_at,created_at,updated_at)
           VALUES (80,10,'CI',8,'push','queued','abc','https://github.com/octo-org/gridops/actions/runs/80',1,1,1,1),
                  (81,11,'CI',9,'push','queued','def','https://github.com/octo-org/sift/actions/runs/81',1,1,1,1);
         INSERT INTO workflow_jobs (id,run_id,name,status,labels,html_url,created_at,updated_at)
           VALUES (800,80,'build','queued','[\"self-hosted\",\"linux\"]','https://github.com/octo-org/gridops/actions/runs/80/job/800',
                   CAST(strftime('%s','now') AS INTEGER)*1000,1),
                  (810,81,'check','queued','[\"self-hosted\",\"Linux\",\"X64\"]','https://github.com/octo-org/sift/actions/runs/81/job/810',1,1),
                  (811,81,'coverage','queued','[\"self-hosted\",\"Linux\",\"X64\"]','https://github.com/octo-org/sift/actions/runs/81/job/811',1,1);",
    )
    .execute(&f.database)
    .await?;
    let profiles = pool_profiles(&f.database, "fleet-pool").await?;
    let configured = ["self-hosted".to_owned()];
    let (chosen, behind) = choose_platform(&f.database, "fleet-pool", &configured, 1, &profiles)
        .await?
        .context("platform")?;
    assert_eq!(
        (chosen.input.architecture, behind),
        (gridops_core::fleet::Architecture::X64, true)
    );
    assert_ne!(chosen.id, machine.profile_id);
    let repositories = |profile: String| {
        let (database, profiles, configured) = (&f.database, &profiles, &configured);
        async move {
            Ok::<_, anyhow::Error>(
                platform_jobs(database, "fleet-pool", configured, profiles, &profile)
                    .await?
                    .into_iter()
                    .map(|job| job.repository_id)
                    .collect::<Vec<_>>(),
            )
        }
    };
    assert_eq!(repositories(chosen.id.clone()).await?, [11, 11]);
    assert_eq!(repositories(machine.profile_id.clone()).await?, [10]);
    Ok(())
}

/// When the agent can't settle a placement itself, an administrator can: the
/// machine shows as blocked, and repairing releases the unconfirmed start on
/// their word, with an audit entry, so the machine takes work again.
#[tokio::test]
async fn an_administrator_repairs_a_placement_the_agent_cannot_confirm() -> Result<()> {
    let f = Fixture::new().await?;
    seed_pool(&f).await?;
    let proxy = Proxy::start(&f.origin).await?;
    let runtime = Arc::new(FakeDocker::default());
    let machine = active_machine(&f, &proxy.origin, Arc::clone(&runtime)).await?;
    let host = machine.host.to_string();
    let workload = Uuid::new_v4();
    let placement_id = prepared_placement(&f, &machine, workload).await?;

    // The start's result never gets through, and its deadline passes.
    proxy.faults.refuse_results.store(true, Ordering::SeqCst);
    issue_start(&f, placement_id, workload, 2_000).await?;
    tokio::time::sleep(Duration::from_millis(2_500)).await;
    let identity = machine.journal.identity().await?.context("identity")?;
    inventory::agent_post(
        &f,
        &host,
        identity.credential.expose_secret(),
        "commands/poll",
    )
    .json(&json!({"authority_session_id":Uuid::new_v4(),"wait_ms":0,"limit":1}))
    .send()
    .await?
    .error_for_status()?;
    assert_eq!(placement_state(&f, placement_id).await?, "uncertain");

    let get = |path: String| f.client.get(f.url(&path)).header(header::COOKIE, &f.cookie);
    let listed: Value = get(format!("/hosts/{host}/unconfirmed"))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    let item = &listed["items"][0];
    assert_eq!(item["placementId"], placement_id.to_string());
    assert_eq!(item["repair"], "release");
    assert_eq!(item["command"]["kind"], "start_runner");
    assert_eq!(item["command"]["interrupted"], false);
    let queue: Value = get("/pools/fleet-pool/queue".to_owned())
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    assert_eq!(queue["blockedHosts"][0]["hostId"], host);
    assert_eq!(queue["blockedHosts"][0]["unconfirmed"], 1);

    let repair = || {
        f.admin_post(&format!("/hosts/{host}/unconfirmed/repair"))
            .json(&json!({"placementIds": [placement_id]}))
    };
    sqlx::query("UPDATE users SET role='member' WHERE id='fleet-user'")
        .execute(&f.database)
        .await?;
    error(repair().send().await?, StatusCode::FORBIDDEN, "forbidden").await?;
    sqlx::query("UPDATE users SET role='admin' WHERE id='fleet-user'")
        .execute(&f.database)
        .await?;
    let repaired: Value = repair().send().await?.error_for_status()?.json().await?;
    assert_eq!(repaired["repaired"], json!([placement_id]));
    assert_eq!(placement_state(&f, placement_id).await?, "failed");
    assert_eq!(allocation_states(&f, placement_id).await?, ["released"]);
    let audit: String = sqlx::query_scalar(
        "SELECT metadata FROM audit_events WHERE action='fleet.placement.repaired' AND target_id=?",
    )
    .bind(placement_id.to_string())
    .fetch_one(&f.database)
    .await?;
    assert_eq!(
        serde_json::from_str::<Value>(&audit)?,
        json!({"repair": "release", "cancelled": ["start_runner"]})
    );
    // Repeating it changes nothing.
    let again: Value = repair().send().await?.error_for_status()?.json().await?;
    assert_eq!(again["repaired"], json!([]));

    // The agent's late answer can't undo the repair, and the machine works.
    proxy.faults.refuse_results.store(false, Ordering::SeqCst);
    within("work authority", || has_authority(&f, &host)).await?;
    assert_eq!(placement_state(&f, placement_id).await?, "failed");

    let _ = machine.stop.send(());
    assert_eq!(machine.running.await??, Stop::Shutdown);
    Ok(())
}
