//! Reconciler passes while the Tart (macOS) agent is out of reach: the Mac
//! it runs on sleeps. The manager then lists the Docker runners and names
//! `tart` under `unavailableProviders` (see the manager's
//! `tart_outage_tests`). The pass must go on for every pool that does not
//! need the Mac, leave the Mac's pools and runners exactly as they are, and
//! not count the outage against them. Through the stand-in runner manager
//! and GitHub of `start_tests`, and the real database.

use std::sync::Mutex;

use tracing::instrument::WithSubscriber as _;

use super::*;
use crate::start_tests::{PASS_LIMIT, Setup};

/// The Mac's runner the tests seed, as the manager and the database know it.
const MAC_RUNNER: &str = "runner-mac-1";
const MAC_CONTAINER: &str = "tart-0123456789abcdef0123456789abcdef";
const MAC_GITHUB_ID: i64 = 4_242;

/// A second macOS pool, so a message logged once per pool shows up twice.
const SECOND_MAC_POOL: &str = r#"INSERT INTO runner_pools (id,installation_id,name,scope,image,provider,providers,docker_image,tart_image,desired_count,autoscaling_enabled,created_at,updated_at)
     VALUES ('pool-mac-2',1,'mac-2','organization','base','tart','["tart"]','','base',1,0,3,3);"#;

/// What a pass logged at `WARN` and above, one line per event.
#[derive(Clone, Default)]
struct Captured(Arc<Mutex<Vec<u8>>>);

impl std::io::Write for Captured {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// Runs one pass and gives what it logged at `WARN` and above.
async fn logged_pass(app: &Reconciler) -> Result<Vec<String>> {
    let captured = Captured::default();
    let writer = captured.clone();
    let subscriber = tracing_subscriber::fmt()
        .with_ansi(false)
        .with_max_level(tracing::Level::WARN)
        .with_writer(move || writer.clone())
        .finish();
    tokio::time::timeout(PASS_LIMIT, run_pass(app).with_subscriber(subscriber)).await?;
    let text = String::from_utf8_lossy(
        &captured
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner),
    )
    .into_owned();
    Ok(text.lines().map(ToOwned::to_owned).collect())
}

/// The warnings that say the macOS (Tart) provider could not be reached.
fn tart_unavailable_warnings(lines: &[String]) -> usize {
    lines
        .iter()
        .filter(|line| {
            let lower = line.to_lowercase();
            line.contains(" WARN ")
                && lower.contains("tart")
                && (lower.contains("unavailable") || lower.contains("unreachable"))
        })
        .count()
}

fn errors(lines: &[String]) -> Vec<&String> {
    lines
        .iter()
        .filter(|line| line.contains(" ERROR "))
        .collect()
}

async fn events(setup: &Setup, event: &str) -> Result<i64> {
    Ok(
        sqlx::query_scalar("SELECT COUNT(*) FROM runner_events WHERE event=?")
            .bind(event)
            .fetch_one(&setup.app.database)
            .await?,
    )
}

/// Seeds an online runner in `pool-mac`, registered with GitHub, whose VM
/// the manager lists while the Mac answers.
async fn seed_mac_runner(setup: &Setup) -> Result<()> {
    sqlx::query(
        "INSERT INTO runners (id,pool_id,name,provider,os,architecture,status,ephemeral,container_id,github_runner_id,last_heartbeat_at,created_at,updated_at)
         VALUES (?,'pool-mac','mac-abcd1234','tart','macOS','ARM64','online',1,?,?,1,1,1)",
    )
    .bind(MAC_RUNNER)
    .bind(MAC_CONTAINER)
    .bind(MAC_GITHUB_ID)
    .execute(&setup.app.database)
    .await?;
    setup
        .manager
        .running
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .push(json!({
            "id": MAC_CONTAINER, "state": "running", "provider": "tart",
            "labels": {
                "io.gridops.managed": "true",
                "io.gridops.runner-id": MAC_RUNNER,
                "io.gridops.pool-id": "pool-mac",
            },
        }));
    Ok(())
}

/// Requirement 2: with the Mac out of reach, a pass still starts the Linux
/// pool's runner, asks nothing of the Mac for the macOS pools (it would only
/// be refused), does not record their pass as failed, and says once per
/// pass, not once per pool, that the macOS provider is unavailable.
#[tokio::test]
async fn a_pass_with_the_mac_out_of_reach_starts_linux_runners_and_skips_only_the_mac_pools()
-> Result<()> {
    let setup = Setup::new(SECOND_MAC_POOL, tart_lane::SLOT_WATCH_INTERVAL).await?;
    setup.manager.tart_down.store(true, Ordering::SeqCst);

    let first = logged_pass(&setup.app).await?;
    assert_eq!(setup.manager.starts("docker"), ["pool-linux"]);
    assert_eq!(
        setup.runners("pool-linux").await?,
        [("online".to_owned(), true)],
        "the Linux pool got its runner"
    );
    assert_eq!(
        setup.manager.admissions("tart"),
        [],
        "a Tart runner was asked for while the manager says tart is unavailable"
    );
    assert_eq!(setup.runners("pool-mac").await?, []);
    assert_eq!(setup.runners("pool-mac-2").await?, []);
    assert_eq!(
        events(&setup, "Pool reconciliation failed").await?,
        0,
        "skipping a pool whose provider is out of reach is not a failure"
    );
    assert!(errors(&first).is_empty(), "{first:#?}");
    assert_eq!(tart_unavailable_warnings(&first), 1, "{first:#?}");

    let second = logged_pass(&setup.app).await?;
    assert_eq!(
        tart_unavailable_warnings(&second),
        1,
        "said again, once, on the next pass: {second:#?}"
    );
    assert_eq!(setup.manager.admissions("tart"), []);
    setup.finish().await
}

/// Requirement 3: the manager leaves the Mac's runners out of the list
/// because it could not ask for them, not because they are gone. A pass must
/// not take them for missing: not mark them failed, not deregister them from
/// GitHub, not ask the manager to remove their VMs.
#[tokio::test]
async fn the_macs_runners_are_not_failed_deregistered_or_removed_while_it_cannot_be_asked()
-> Result<()> {
    let setup = Setup::new("", tart_lane::SLOT_WATCH_INTERVAL).await?;
    seed_mac_runner(&setup).await?;
    setup.manager.tart_down.store(true, Ordering::SeqCst);

    tokio::time::timeout(PASS_LIMIT, run_pass(&setup.app)).await?;
    tokio::time::timeout(PASS_LIMIT, run_pass(&setup.app)).await?;

    let (status, deleted_at): (String, Option<i64>) =
        sqlx::query_as("SELECT status,deleted_at FROM runners WHERE id=?")
            .bind(MAC_RUNNER)
            .fetch_one(&setup.app.database)
            .await?;
    assert_eq!(
        (status.as_str(), deleted_at),
        ("online", None),
        "the Mac's runner was taken for missing"
    );
    let github_removals = setup
        .github_removals
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone();
    assert_eq!(
        github_removals,
        Vec::<String>::new(),
        "deregistered from GitHub"
    );
    assert_eq!(setup.manager.removed(), Vec::<String>::new());
    let cleanups: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM github_runner_cleanup WHERE id=?")
        .bind(MAC_RUNNER)
        .fetch_one(&setup.app.database)
        .await?;
    assert_eq!(cleanups, 0, "a GitHub deregistration was queued");
    // The Linux pool was looked after all the same.
    assert_eq!(
        setup.runners("pool-linux").await?,
        [("online".to_owned(), true)]
    );
    setup.finish().await
}

/// Requirements 5 and 6: the Mac goes to sleep while a Tart start, already
/// admitted, waits on it, so the start fails as `tart_agent_unavailable`.
/// That is the Mac being away, not the pool's start being broken: it must
/// not count towards the pool's provisioning circuit, or a few nights of
/// sleep leave the pool `blocked` for fifteen minutes after the Mac wakes.
/// Here the pool already has two failures, so counting this one would open
/// the circuit.
#[tokio::test]
async fn a_start_cut_off_by_the_mac_sleeping_does_not_count_towards_the_circuit() -> Result<()> {
    let setup = Setup::new(
        "UPDATE runner_pools SET provision_failure_count=2 WHERE id='pool-mac';",
        tart_lane::SLOT_WATCH_INTERVAL,
    )
    .await?;
    tokio::time::timeout(PASS_LIMIT, run_pass(&setup.app)).await?;
    setup.tart_start_reached_manager().await?;
    setup.manager.tart_down.store(true, Ordering::SeqCst);
    setup.manager.tart_gate.add_permits(1);
    setup.tart_start_ended().await?;

    // The Mac wakes; the next pass records how the start ended.
    setup.manager.tart_down.store(false, Ordering::SeqCst);
    tokio::time::timeout(PASS_LIMIT, run_pass(&setup.app)).await?;

    let (state, failures, circuit_open, retry_at): (String, i64, bool, Option<i64>) =
        sqlx::query_as(
            "SELECT state,provision_failure_count,provision_circuit_open,provision_retry_at FROM runner_pools WHERE id='pool-mac'",
        )
        .fetch_one(&setup.app.database)
        .await?;
    assert_eq!(
        (failures, circuit_open),
        (2, false),
        "the Mac sleeping was counted as a failed start (state {state})"
    );
    assert_ne!(state, "blocked");
    assert!(
        retry_at.is_none_or(|at| at <= now_millis() + 30_000),
        "the pool waits past a short pause after the Mac woke: retry at {retry_at:?}"
    );
    setup.manager.tart_gate.add_permits(1);
    setup.finish().await
}
