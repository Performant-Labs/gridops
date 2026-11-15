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

// ---------------------------------------------------------------------------
// Round 2 (review of PR #123).
// ---------------------------------------------------------------------------

/// A runner to seed: its pool, provider and VM or container, and what makes
/// a pass want it gone (an outdated configuration, a repository outside the
/// pool). Seeded `online`, idle, registered with GitHub, and listed by the
/// manager while its provider answers.
struct Seeded {
    id: &'static str,
    pool: &'static str,
    provider: &'static str,
    container: &'static str,
    github_id: i64,
    /// Newer runners come first wherever a pass picks one to remove.
    created_at: i64,
    configuration_version: i64,
    repository: Option<i64>,
}

impl Seeded {
    fn tart(id: &'static str, pool: &'static str, container: &'static str, github_id: i64) -> Self {
        Self {
            id,
            pool,
            provider: "tart",
            container,
            github_id,
            created_at: 50,
            configuration_version: 2,
            repository: None,
        }
    }

    fn docker(
        id: &'static str,
        pool: &'static str,
        container: &'static str,
        github_id: i64,
    ) -> Self {
        Self {
            provider: "docker",
            created_at: 10,
            ..Self::tart(id, pool, container, github_id)
        }
    }
}

async fn seed(setup: &Setup, runner: &Seeded) -> Result<()> {
    let (os, architecture) = if runner.provider == "tart" {
        ("macOS", "ARM64")
    } else {
        ("Linux", "X64")
    };
    sqlx::query(
        "INSERT INTO runners (id,pool_id,name,provider,os,architecture,status,busy,ephemeral,container_id,github_runner_id,configuration_version,target_repository_id,last_heartbeat_at,created_at,updated_at)
         VALUES (?,?,?,?,?,?,'online',0,1,?,?,?,?,1,?,?)",
    )
    .bind(runner.id)
    .bind(runner.pool)
    .bind(format!("{}-name", runner.id))
    .bind(runner.provider)
    .bind(os)
    .bind(architecture)
    .bind(runner.container)
    .bind(runner.github_id)
    .bind(runner.configuration_version)
    .bind(runner.repository)
    .bind(runner.created_at)
    .bind(runner.created_at)
    .execute(&setup.app.database)
    .await?;
    setup
        .manager
        .running
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .push(json!({
            "id": runner.container, "state": "running", "provider": runner.provider,
            "labels": {
                "io.gridops.managed": "true",
                "io.gridops.runner-id": runner.id,
                "io.gridops.pool-id": runner.pool,
            },
        }));
    Ok(())
}

fn github_removals(setup: &Setup) -> Vec<String> {
    setup
        .github_removals
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone()
}

/// The runner is as it was seeded: online, not deleted, still registered
/// with GitHub (nothing asked to remove it there or queued for later), and
/// its VM not asked to be removed.
async fn assert_left_alone(setup: &Setup, runner: &Seeded) -> Result<()> {
    let (status, deleted_at): (String, Option<i64>) =
        sqlx::query_as("SELECT status,deleted_at FROM runners WHERE id=?")
            .bind(runner.id)
            .fetch_one(&setup.app.database)
            .await?;
    assert_eq!(
        (status.as_str(), deleted_at),
        ("online", None),
        "{} was taken for missing or removed",
        runner.id
    );
    let suffix = format!("/actions/runners/{}", runner.github_id);
    let removals = github_removals(setup);
    assert!(
        !removals.iter().any(|path| path.ends_with(&suffix)),
        "{} was deregistered from GitHub: {removals:?}",
        runner.id
    );
    let cleanups: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM github_runner_cleanup WHERE id=?")
        .bind(runner.id)
        .fetch_one(&setup.app.database)
        .await?;
    assert_eq!(
        cleanups, 0,
        "{}: a GitHub deregistration was queued",
        runner.id
    );
    let removed = setup.manager.removed();
    assert!(
        !removed.iter().any(|id| id == runner.container),
        "{}: its VM was asked to be removed: {removed:?}",
        runner.id
    );
    Ok(())
}

/// The runner was removed: deleted in the database and its container asked
/// to be removed.
async fn assert_removed(setup: &Setup, runner: &Seeded, why: &str) -> Result<()> {
    let deleted_at: Option<i64> = sqlx::query_scalar("SELECT deleted_at FROM runners WHERE id=?")
        .bind(runner.id)
        .fetch_one(&setup.app.database)
        .await?;
    let removed = setup.manager.removed();
    assert!(
        deleted_at.is_some() && removed.iter().any(|id| id == runner.container),
        "{} was not {why} (deleted at {deleted_at:?}, manager removals {removed:?})",
        runner.id
    );
    Ok(())
}

/// Three Tart runners of `pool`, each of which a pass would remove if it
/// took the Mac's silence for their being gone or idle: one current, one
/// with an outdated configuration (rotation), and the newest, one more than
/// the pool wants (excess trimming).
fn three_tart_runners(pool: &'static str) -> [Seeded; 3] {
    [
        Seeded {
            created_at: 30,
            ..Seeded::tart(
                "runner-tart-current",
                pool,
                "tart-00000000000000000000000000000001",
                501,
            )
        },
        Seeded {
            created_at: 40,
            configuration_version: 1,
            ..Seeded::tart(
                "runner-tart-stale",
                pool,
                "tart-00000000000000000000000000000002",
                502,
            )
        },
        Seeded {
            created_at: 50,
            ..Seeded::tart(
                "runner-tart-extra",
                pool,
                "tart-00000000000000000000000000000003",
                503,
            )
        },
    ]
}

/// A pool that starts runners with Docker or Tart, as configuration 2, so a
/// runner of configuration 1 is outdated.
const MIXED_POOL: &str = r#"INSERT INTO runner_pools (id,installation_id,name,scope,image,provider,providers,docker_image,tart_image,desired_count,autoscaling_enabled,configuration_version,created_at,updated_at)
     VALUES ('pool-mix',1,'mix','organization','runner:latest','docker','["docker","tart"]','runner:latest','base',1,0,2,3,3);"#;

/// Review finding 2: a pool that uses Tart and Docker is reconciled while
/// the Mac sleeps (only a Tart-only pool is skipped), so nothing but the
/// out-of-sight guards keeps the pass from taking its Tart runners for
/// missing (and failing them), from rotating the outdated one, or from
/// trimming the one too many. They must stay as they are.
#[tokio::test]
async fn a_mixed_pools_tart_runners_stay_online_and_registered_while_the_mac_cannot_be_asked()
-> Result<()> {
    let setup = Setup::new(MIXED_POOL, tart_lane::SLOT_WATCH_INTERVAL).await?;
    let runners = three_tart_runners("pool-mix");
    for runner in &runners {
        seed(&setup, runner).await?;
    }
    setup.manager.tart_down.store(true, Ordering::SeqCst);

    tokio::time::timeout(PASS_LIMIT, run_pass(&setup.app)).await?;
    tokio::time::timeout(PASS_LIMIT, run_pass(&setup.app)).await?;

    for runner in &runners {
        assert_left_alone(&setup, runner).await?;
    }
    assert_eq!(
        setup.runners("pool-linux").await?,
        [("online".to_owned(), true)],
        "the pass went on for the Linux pool"
    );
    setup.finish().await
}

/// Review finding 2, the other way a Tart-only pool is reconciled during an
/// outage: it has a fleet platform (a macOS machine) to run on. Its Tart
/// runners on the Mac must be left as they are all the same.
#[tokio::test]
async fn a_mac_pool_with_a_fleet_platform_leaves_its_tart_runners_alone_while_the_mac_cannot_be_asked()
-> Result<()> {
    let setup = Setup::new(
        r#"UPDATE runner_pools SET configuration_version=2 WHERE id='pool-mac';
           INSERT INTO pool_execution_profiles (id,pool_id,name,enabled,runtime_kind,execution_os,architecture,mode,image,cpu_millis,memory_mib,disk_bytes,created_at,updated_at)
             VALUES ('40000000-0000-4000-8000-0000000000aa','pool-mac','macos',1,'tart_vm','macos','arm64','ephemeral','base',4000,4096,0,1,1);"#,
        tart_lane::SLOT_WATCH_INTERVAL,
    )
    .await?;
    assert!(
        !fleet::profiles(&setup.app, "pool-mac").await?.is_empty(),
        "the setup gives pool-mac a fleet platform, so the pass reconciles it"
    );
    let runners = three_tart_runners("pool-mac");
    for runner in &runners {
        seed(&setup, runner).await?;
    }
    setup.manager.tart_down.store(true, Ordering::SeqCst);

    tokio::time::timeout(PASS_LIMIT, run_pass(&setup.app)).await?;
    tokio::time::timeout(PASS_LIMIT, run_pass(&setup.app)).await?;

    for runner in &runners {
        assert_left_alone(&setup, runner).await?;
    }
    // A pass that was skipped whole would leave them alone too, and prove
    // nothing.
    assert_eq!(
        setup.runners("pool-linux").await?,
        [("online".to_owned(), true)],
        "the pass went on for the Linux pool"
    );
    setup.finish().await
}

/// Review finding 4, rotation: in a mixed pool during an outage, the newest
/// outdated runner is a Tart runner out of sight, which must be skipped. The
/// outdated Docker runner is still rotated, rather than rotation stopping at
/// the Tart runner it may not remove.
#[tokio::test]
async fn a_mixed_pool_still_rotates_its_docker_runners_while_the_mac_cannot_be_asked() -> Result<()>
{
    let setup = Setup::new(
        &MIXED_POOL.replace(",1,0,2,3,3)", ",2,0,2,3,3)"),
        tart_lane::SLOT_WATCH_INTERVAL,
    )
    .await?;
    let tart = Seeded {
        configuration_version: 1,
        ..Seeded::tart(
            "runner-tart-stale",
            "pool-mix",
            "tart-00000000000000000000000000000002",
            502,
        )
    };
    let docker = Seeded {
        configuration_version: 1,
        ..Seeded::docker("runner-docker-stale", "pool-mix", "ctr-docker-stale", 601)
    };
    seed(&setup, &tart).await?;
    seed(&setup, &docker).await?;
    setup.manager.tart_down.store(true, Ordering::SeqCst);

    tokio::time::timeout(PASS_LIMIT, run_pass(&setup.app)).await?;

    assert_left_alone(&setup, &tart).await?;
    assert_removed(&setup, &docker, "rotated").await?;
    setup.finish().await
}

/// Review finding 4, excess trimming: the mixed pool wants one runner and
/// has two idle ones, the newer on the out-of-sight Mac. The Docker runner
/// is the one trimmed; the Tart runner is skipped, not picked and refused.
#[tokio::test]
async fn a_mixed_pool_still_trims_its_docker_runners_while_the_mac_cannot_be_asked() -> Result<()> {
    let setup = Setup::new(MIXED_POOL, tart_lane::SLOT_WATCH_INTERVAL).await?;
    let tart = Seeded::tart(
        "runner-tart-1",
        "pool-mix",
        "tart-00000000000000000000000000000001",
        501,
    );
    let docker = Seeded::docker("runner-docker-1", "pool-mix", "ctr-docker-1", 601);
    seed(&setup, &tart).await?;
    seed(&setup, &docker).await?;
    setup.manager.tart_down.store(true, Ordering::SeqCst);

    tokio::time::timeout(PASS_LIMIT, run_pass(&setup.app)).await?;

    assert_left_alone(&setup, &tart).await?;
    assert_removed(&setup, &docker, "trimmed").await?;
    setup.finish().await
}

/// Review finding 4, rebalancing (`evict_surplus_runner`): a repository
/// pool at its desired size, with queued work in its repository 10 and two
/// idle runners parked on repository 11, which it no longer serves. The
/// newer is on the out-of-sight Mac. The Docker runner is the one evicted to
/// make room; the Tart runner is skipped, not picked and refused.
#[tokio::test]
async fn a_mixed_pool_still_rebalances_with_its_docker_runners_while_the_mac_cannot_be_asked()
-> Result<()> {
    let setup = Setup::new(
        r#"INSERT INTO repositories (id,installation_id,owner,name,full_name,private,default_branch,html_url,last_synced_at,created_at,updated_at)
             VALUES (10,1,'octo-org','app','octo-org/app',0,'main','https://github.com/octo-org/app',1,1,1),
                    (11,1,'octo-org','old','octo-org/old',0,'main','https://github.com/octo-org/old',1,1,1);
           INSERT INTO runner_pools (id,installation_id,repository_id,name,scope,labels,image,provider,providers,docker_image,tart_image,desired_count,min_count,max_count,autoscaling_enabled,created_at,updated_at)
             VALUES ('pool-mix',1,10,'mix','repository','["mix"]','runner:latest','docker','["docker","tart"]','runner:latest','base',2,2,2,1,3,3);
           INSERT INTO runner_pool_repositories (pool_id,repository_id,created_at) VALUES ('pool-mix',10,1);
           INSERT INTO workflow_runs (id,repository_id,workflow_name,run_number,event,status,head_sha,html_url,github_created_at,github_updated_at,created_at,updated_at)
             VALUES (20,10,'CI',1,'push','queued','abc123','https://github.com/octo-org/app/actions/runs/20',1,1,1,1);
           INSERT INTO workflow_jobs (id,run_id,name,status,labels,html_url,created_at,updated_at)
             VALUES (30,20,'build','queued','["self-hosted","mix"]','https://github.com/octo-org/app/actions/runs/20/job/30',1,1);"#,
        tart_lane::SLOT_WATCH_INTERVAL,
    )
    .await?;
    let tart = Seeded {
        repository: Some(11),
        ..Seeded::tart(
            "runner-tart-parked",
            "pool-mix",
            "tart-00000000000000000000000000000001",
            501,
        )
    };
    let docker = Seeded {
        repository: Some(11),
        ..Seeded::docker("runner-docker-parked", "pool-mix", "ctr-docker-parked", 601)
    };
    seed(&setup, &tart).await?;
    seed(&setup, &docker).await?;
    let waiting = gridops_core::autoscaling::repository_capacities(&setup.app.database, "pool-mix")
        .await?
        .into_iter()
        .map(|capacity| (capacity.repository_id, capacity.queued, capacity.active))
        .collect::<Vec<_>>();
    assert_eq!(
        waiting,
        [(10, 1, 0)],
        "the setup gives repository 10 queued work and no runner"
    );
    setup.manager.tart_down.store(true, Ordering::SeqCst);

    tokio::time::timeout(PASS_LIMIT, run_pass(&setup.app)).await?;

    assert_left_alone(&setup, &tart).await?;
    assert_removed(&setup, &docker, "evicted for the waiting repository").await?;
    setup.finish().await
}

/// Review finding 1, the reconciler's side: it asks the manager for the
/// partial list (`GET /v1/runners?partial=true`) every time it lists
/// runners: for the pass, and when it looks for the container of a start
/// that failed. A manager answers a caller that does not ask with the old
/// all-or-nothing list, so an older reconciler or API still running during
/// a per-service deploy or rollback never sees the Mac's runners as gone.
#[tokio::test]
async fn the_reconciler_asks_for_the_partial_runner_list_every_time_it_lists_runners() -> Result<()>
{
    let setup = Setup::new("", tart_lane::SLOT_WATCH_INTERVAL).await?;
    setup.manager.tart_down.store(true, Ordering::SeqCst);
    // The Linux pool's start fails, so the pass also looks for its container.
    setup.manager.docker_fails.store(true, Ordering::SeqCst);

    tokio::time::timeout(PASS_LIMIT, run_pass(&setup.app)).await?;

    let queries = setup.manager.list_queries();
    assert!(
        queries.len() >= 2,
        "the pass and the failed start's cleanup each list runners: {queries:?}"
    );
    assert!(
        queries
            .iter()
            .all(|query| crate::start_tests::asks_for_partial_list(Some(query))),
        "a runner list did not ask for the partial list: {queries:?}"
    );
    setup.finish().await
}

/// The event a failed start records, and what it says about the cleanup of
/// the runner's container.
async fn failed_start_cleanup(setup: &Setup, pool_id: &str) -> Result<Value> {
    let metadata: String = sqlx::query_scalar(
        "SELECT metadata FROM runner_events WHERE pool_id=? AND event='Runner provisioning failed' ORDER BY created_at DESC,rowid DESC LIMIT 1",
    )
    .bind(pool_id)
    .fetch_optional(&setup.app.database)
    .await?
    .context("no failed start was recorded")?;
    Ok(serde_json::from_str::<Value>(&metadata)?["managerContainerCleanup"].clone())
}

/// Review finding 6: a Docker start fails, leaving no container, while the
/// Mac sleeps. The manager listed every Docker container, so "not found" is
/// the truth for a Docker runner; Tart being out of sight has nothing to do
/// with it and must not turn the cleanup into "could not ask tart".
#[tokio::test]
async fn a_failed_docker_start_during_a_tart_outage_finds_no_container_rather_than_blaming_tart()
-> Result<()> {
    let setup = Setup::new("", tart_lane::SLOT_WATCH_INTERVAL).await?;
    setup.manager.tart_down.store(true, Ordering::SeqCst);
    setup.manager.docker_fails.store(true, Ordering::SeqCst);

    tokio::time::timeout(PASS_LIMIT, run_pass(&setup.app)).await?;

    let cleanup = failed_start_cleanup(&setup, "pool-linux").await?;
    assert_eq!(cleanup["status"], "not-found", "{cleanup}");
    setup.finish().await
}

/// Review finding 6, the other side: a Tart start cut off by the Mac going
/// to sleep cannot be told apart from "no VM", because the Mac could not be
/// asked. That cleanup still says so rather than "not found".
#[tokio::test]
async fn a_tart_start_cut_off_by_the_mac_sleeping_still_says_its_vm_could_not_be_asked()
-> Result<()> {
    let setup = Setup::new("", tart_lane::SLOT_WATCH_INTERVAL).await?;
    tokio::time::timeout(PASS_LIMIT, run_pass(&setup.app)).await?;
    setup.tart_start_reached_manager().await?;
    setup.manager.tart_down.store(true, Ordering::SeqCst);
    setup.manager.tart_gate.add_permits(1);
    setup.tart_start_ended().await?;

    let cleanup = failed_start_cleanup(&setup, "pool-mac").await?;
    assert_eq!(cleanup["status"], "failed", "{cleanup}");
    let error = cleanup["error"].as_str().unwrap_or_default().to_lowercase();
    assert!(
        error.contains("tart") || error.contains("macos"),
        "the cleanup does not say which provider could not be asked: {cleanup}"
    );
    setup.manager.tart_gate.add_permits(1);
    setup.finish().await
}

/// A pool's state and the reason it waits, if any.
async fn pool_state(setup: &Setup, pool_id: &str) -> Result<(String, Option<String>)> {
    Ok(sqlx::query_as(
        "SELECT pool.state,wait.reason FROM runner_pools pool LEFT JOIN runner_pool_waits wait ON wait.pool_id=pool.id WHERE pool.id=?",
    )
    .bind(pool_id)
    .fetch_one(&setup.app.database)
    .await?)
}

/// A paused macOS pool, as pausing leaves it.
const PAUSED_MAC_POOL: &str = r#"INSERT INTO runner_pools (id,installation_id,name,scope,image,provider,providers,docker_image,tart_image,desired_count,autoscaling_enabled,paused,state,created_at,updated_at)
     VALUES ('pool-mac-paused',1,'mac-paused','organization','base','tart','["tart"]','','base',1,0,1,'paused',3,3);"#;

fn says_unavailable(text: &str) -> bool {
    let lower = text.to_lowercase();
    lower.contains("unavailable") || lower.contains("unreachable")
}

/// Review finding 5, visibility: while a macOS pool is skipped because the
/// Mac cannot be asked, the pool must say so (a state naming the provider
/// unavailable, or a reason that does, in place of the state it last had)
/// rather than go on showing `active` as if all were well. Once the Mac
/// answers again it says so no more. A paused macOS pool, skipped all the
/// same, stays paused: pausing is the operator's own block and says more
/// (round 3, finding 5).
#[tokio::test]
async fn a_skipped_mac_pool_shows_its_provider_unavailable_until_the_mac_answers() -> Result<()> {
    let setup = Setup::new(
        &format!("UPDATE runner_pools SET desired_count=0 WHERE id='pool-mac'; {PAUSED_MAC_POOL}"),
        tart_lane::SLOT_WATCH_INTERVAL,
    )
    .await?;
    tokio::time::timeout(PASS_LIMIT, run_pass(&setup.app)).await?;
    let (before, _) = pool_state(&setup, "pool-mac").await?;
    assert_eq!(before, "active", "the setup: the pool runs normally first");
    let (paused_before, _) = pool_state(&setup, "pool-mac-paused").await?;
    assert_eq!(
        paused_before, "paused",
        "the setup: the paused pool is paused"
    );

    setup.manager.tart_down.store(true, Ordering::SeqCst);
    tokio::time::timeout(PASS_LIMIT, run_pass(&setup.app)).await?;
    let (state, reason) = pool_state(&setup, "pool-mac").await?;
    assert!(
        says_unavailable(&state)
            || (state != before && reason.as_deref().is_some_and(says_unavailable)),
        "the skipped pool shows state {state:?}, reason {reason:?}"
    );
    let (paused, _) = pool_state(&setup, "pool-mac-paused").await?;
    assert_eq!(
        paused, "paused",
        "a paused pool was shown as unable to run while the Mac could not be asked"
    );

    setup.manager.tart_down.store(false, Ordering::SeqCst);
    tokio::time::timeout(PASS_LIMIT, run_pass(&setup.app)).await?;
    let (state, reason) = pool_state(&setup, "pool-mac").await?;
    assert!(
        !says_unavailable(&state) && !reason.as_deref().is_some_and(says_unavailable),
        "the pool still shows its provider unavailable after the Mac answered: state {state:?}, reason {reason:?}"
    );
    setup.finish().await
}

/// The events that tell of the Tart (macOS) provider going away or coming
/// back: `event`, then its message and metadata, oldest first.
async fn provider_events(setup: &Setup) -> Result<Vec<(String, String)>> {
    let rows: Vec<(String, String, String)> = sqlx::query_as(
        "SELECT event,message,metadata FROM runner_events ORDER BY created_at,rowid",
    )
    .fetch_all(&setup.app.database)
    .await?;
    Ok(rows
        .into_iter()
        .filter(|(event, message, _)| {
            let text = format!("{event} {message}").to_lowercase();
            (text.contains("tart") || text.contains("macos"))
                && (text.contains("provider") || text.contains("agent"))
        })
        .map(|(event, message, metadata)| (event, format!("{message} {metadata}")))
        .collect())
}

/// Review finding 5, events: the Mac going away is written down once, when
/// it happens, not on every pass it stays away, and so is its coming back,
/// as a different event.
#[tokio::test]
async fn the_mac_going_away_and_coming_back_are_each_recorded_once() -> Result<()> {
    let setup = Setup::new(
        "UPDATE runner_pools SET desired_count=0 WHERE id='pool-mac';",
        tart_lane::SLOT_WATCH_INTERVAL,
    )
    .await?;
    tokio::time::timeout(PASS_LIMIT, run_pass(&setup.app)).await?;
    assert_eq!(
        provider_events(&setup).await?,
        [],
        "nothing while it answers"
    );

    setup.manager.tart_down.store(true, Ordering::SeqCst);
    tokio::time::timeout(PASS_LIMIT, run_pass(&setup.app)).await?;
    let away = provider_events(&setup).await?;
    assert_eq!(away.len(), 1, "the Mac went away: {away:#?}");
    assert!(
        says_unavailable(&away[0].0) || says_unavailable(&away[0].1),
        "{away:#?}"
    );
    tokio::time::timeout(PASS_LIMIT, run_pass(&setup.app)).await?;
    assert_eq!(
        provider_events(&setup).await?,
        away,
        "recorded again while it stays away"
    );

    setup.manager.tart_down.store(false, Ordering::SeqCst);
    tokio::time::timeout(PASS_LIMIT, run_pass(&setup.app)).await?;
    let back = provider_events(&setup).await?;
    assert_eq!(back.len(), 2, "the Mac came back: {back:#?}");
    assert_ne!(
        back[1].0, away[0].0,
        "coming back reads as going away: {back:#?}"
    );
    tokio::time::timeout(PASS_LIMIT, run_pass(&setup.app)).await?;
    assert_eq!(provider_events(&setup).await?.len(), 2, "recorded again");
    setup.finish().await
}

/// What a pass records about the Tart provider going away when the manager
/// says it failed as `error` (its `providerErrors.tart` entry).
async fn outage_record(error: Value) -> Result<(String, Vec<(String, String)>)> {
    let setup = Setup::new(
        "UPDATE runner_pools SET desired_count=0 WHERE id='pool-mac';",
        tart_lane::SLOT_WATCH_INTERVAL,
    )
    .await?;
    *setup
        .manager
        .tart_down_error
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = error;
    setup.manager.tart_down.store(true, Ordering::SeqCst);
    tokio::time::timeout(PASS_LIMIT, run_pass(&setup.app)).await?;
    let linux = setup.runners("pool-linux").await?;
    let events = provider_events(&setup).await?;
    let (state, reason) = pool_state(&setup, "pool-mac").await?;
    setup.finish().await?;
    assert_eq!(
        linux,
        [("online".to_owned(), true)],
        "the Linux pool got its runner whatever the Tart agent's failure"
    );
    Ok((format!("{state} {}", reason.unwrap_or_default()), events))
}

/// Review finding 5, telling failures apart: a Mac that is asleep or away
/// (the agent cannot be reached) is an outage that ends by itself; an agent
/// that answers but refuses the manager's token, or answers with a list that
/// does not parse, is broken and needs someone. The Linux pools go on in
/// both cases, but the record of each must say which it is.
#[tokio::test]
async fn an_unreachable_tart_agent_and_a_broken_one_are_recorded_differently() -> Result<()> {
    let (_, unreachable) = outage_record(crate::start_tests::tart_unreachable()).await?;
    let (auth_state, auth) = outage_record(json!({
        "kind": "error",
        "message": "Tart agent answered 401 Unauthorized: invalid token",
    }))
    .await?;

    assert_eq!(unreachable.len(), 1, "{unreachable:#?}");
    assert_eq!(auth.len(), 1, "{auth:#?}");
    let network = format!("{} {}", unreachable[0].0, unreachable[0].1).to_lowercase();
    let refused = format!("{} {}", auth[0].0, auth[0].1).to_lowercase();
    assert!(
        ["unreachable", "network", "connect", "timed out", "timeout"]
            .iter()
            .any(|word| network.contains(word)),
        "the record of a Mac that cannot be reached does not say so: {network}"
    );
    // Not the agent's own words: a fixed message per kind reaches the
    // events the API serves, and the detail the log (round 3, finding 3).
    assert!(
        refused.contains("error"),
        "the record of an agent that refused the manager does not say it answered with an error: {refused}"
    );
    assert!(
        !["unreachable", "network", "timed out"]
            .iter()
            .any(|word| refused.contains(word)),
        "an agent that answered 401 is recorded as unreachable: {refused}"
    );
    // The pool tells the operator too, not only the event list.
    assert!(
        says_unavailable(&auth_state) || auth_state.contains("401"),
        "the skipped pool hides the agent's failure: {auth_state}"
    );
    Ok(())
}

/// Review finding (GREEN round): a macOS pool placed host-first, with a
/// macOS fleet machine ready to take its runners. With the Mac's Tart agent
/// out of reach the host cannot start the runner, exactly as when it has no
/// room, so the pool falls back to the fleet machine as it does then. It
/// must not stop at "nothing to do" because the host's only provider was
/// left out for being unavailable.
#[tokio::test]
async fn a_host_first_mac_pool_falls_back_to_its_fleet_machine_while_the_mac_cannot_be_asked()
-> Result<()> {
    const PROFILE: &str = "40000000-0000-4000-8000-0000000000bb";
    const HOST: &str = "10000000-0000-4000-8000-0000000000bb";
    let setup = Setup::new(
        &format!(
            r#"INSERT INTO pool_execution_profiles (id,pool_id,name,enabled,runtime_kind,execution_os,architecture,mode,image,cpu_millis,memory_mib,disk_bytes,placement,created_at,updated_at)
                 VALUES ('{PROFILE}','pool-mac','macos',1,'tart_vm','macos','arm64','ephemeral','base',4000,4096,0,'host_first',1,1);
               INSERT INTO fleet_hosts (id,name,host_os,architecture,enrollment_state,lifecycle_state,integrity_state,pressure_state,last_heartbeat_at,authority_expires_at,created_at,updated_at)
                 VALUES ('{HOST}','Mac mini','macos','arm64','approved','active','verified','normal',{now},{expires},1,1);
               INSERT INTO host_resource_domains (id,host_id,parent_domain_id,name,cpu_millis,memory_mib,disk_bytes)
                 VALUES ('11000000-0000-4000-8000-0000000000bb','{HOST}',NULL,'Physical',16000,32768,1099511627776);
               INSERT INTO host_backends (id,host_id,domain_id,runtime_kind,execution_os,architecture,readiness,enabled,created_at,updated_at)
                 VALUES ('13000000-0000-4000-8000-0000000000bb','{HOST}','11000000-0000-4000-8000-0000000000bb','tart_vm','macos','arm64','ready',1,1,1);"#,
            now = now_millis(),
            expires = now_millis() + 3_600_000,
        ),
        tart_lane::SLOT_WATCH_INTERVAL,
    )
    .await?;
    // The pool's organization target, granted to the machine.
    let target = gridops_core::fleet::targets::pool_targets(&setup.app.database, "pool-mac")
        .await?
        .into_iter()
        .next()
        .context("pool-mac has an organization fleet target")?;
    sqlx::query("INSERT INTO profile_ci_targets (profile_id,target_id,enabled) VALUES (?,?,1)")
        .bind(PROFILE)
        .bind(&target)
        .execute(&setup.app.database)
        .await?;
    sqlx::query("INSERT INTO host_target_grants (host_id,target_id,allow_schedule) VALUES (?,?,1)")
        .bind(HOST)
        .bind(&target)
        .execute(&setup.app.database)
        .await?;
    assert!(
        gridops_core::fleet::targets::machine_ready(&setup.app.database, "pool-mac", PROFILE)
            .await?,
        "the setup gives pool-mac a ready macOS fleet machine"
    );
    setup.manager.tart_down.store(true, Ordering::SeqCst);

    tokio::time::timeout(PASS_LIMIT, run_pass(&setup.app)).await?;

    assert_eq!(
        setup.runners("pool-linux").await?,
        [("online".to_owned(), true)],
        "the pass went on for the Linux pool"
    );
    assert_eq!(
        setup.manager.admissions("tart"),
        [],
        "a Tart runner was asked of the unreachable Mac"
    );
    let intents: Vec<String> =
        sqlx::query_scalar("SELECT profile_id FROM workload_intents WHERE pool_id='pool-mac'")
            .fetch_all(&setup.app.database)
            .await?;
    assert_eq!(
        intents,
        [PROFILE],
        "pool-mac did not fall back to its fleet machine while the Mac's Tart agent was unreachable"
    );
    setup.finish().await
}

// ---------------------------------------------------------------------------
// Round 3 (second review of PR #123).
// ---------------------------------------------------------------------------

/// The events of one pool that tell of the Tart (macOS) provider going away
/// or coming back, as `provider_events` picks them: `event`, then its
/// message and metadata, oldest first. The API serves a pool's events, in
/// its events tab and the overview, only by the pool they belong to.
async fn pool_provider_events(setup: &Setup, pool_id: &str) -> Result<Vec<(String, String)>> {
    let rows: Vec<(String, String, String)> = sqlx::query_as(
        "SELECT event,message,metadata FROM runner_events WHERE pool_id=? ORDER BY created_at,rowid",
    )
    .bind(pool_id)
    .fetch_all(&setup.app.database)
    .await?;
    Ok(rows
        .into_iter()
        .filter(|(event, message, _)| {
            let text = format!("{event} {message}").to_lowercase();
            (text.contains("tart") || text.contains("macos"))
                && (text.contains("provider") || text.contains("agent"))
        })
        .map(|(event, message, metadata)| (event, format!("{message} {metadata}")))
        .collect())
}

/// Round 3, finding 1a: the Mac going away and coming back must show in the
/// events of every pool that needs it, since the API serves an event only by
/// its pool. Each such pool (macOS-only, or Docker and macOS) gets each
/// transition once, written with its `pool_id`; a Docker-only pool gets
/// none. Chosen over serving pool-less provider events: the API's one rule,
/// that a user sees the events of the pools they can see, stays as it is, and
/// a user with no macOS pool is not told about the Mac.
#[tokio::test]
async fn every_pool_that_needs_the_mac_records_it_going_away_and_coming_back() -> Result<()> {
    let setup = Setup::new(
        &format!(
            "UPDATE runner_pools SET desired_count=0 WHERE id='pool-mac'; {SECOND_MAC_POOL} {MIXED_POOL} UPDATE runner_pools SET desired_count=0 WHERE id IN ('pool-mac-2','pool-mix');"
        ),
        tart_lane::SLOT_WATCH_INTERVAL,
    )
    .await?;
    let affected = ["pool-mac", "pool-mac-2", "pool-mix"];
    tokio::time::timeout(PASS_LIMIT, run_pass(&setup.app)).await?;

    setup.manager.tart_down.store(true, Ordering::SeqCst);
    tokio::time::timeout(PASS_LIMIT, run_pass(&setup.app)).await?;
    tokio::time::timeout(PASS_LIMIT, run_pass(&setup.app)).await?;
    let mut away = Vec::new();
    for pool in affected {
        let events = pool_provider_events(&setup, pool).await?;
        assert_eq!(
            events.len(),
            1,
            "{pool} does not show the Mac going away, once: {events:#?}"
        );
        assert!(
            says_unavailable(&events[0].0) || says_unavailable(&events[0].1),
            "{pool}: {events:#?}"
        );
        away.push(events);
    }
    assert_eq!(
        pool_provider_events(&setup, "pool-linux").await?,
        [],
        "a Docker-only pool was told about the Mac"
    );

    setup.manager.tart_down.store(false, Ordering::SeqCst);
    tokio::time::timeout(PASS_LIMIT, run_pass(&setup.app)).await?;
    tokio::time::timeout(PASS_LIMIT, run_pass(&setup.app)).await?;
    for (pool, before) in affected.into_iter().zip(away) {
        let events = pool_provider_events(&setup, pool).await?;
        assert_eq!(
            events.len(),
            2,
            "{pool} does not show the Mac coming back, once: {events:#?}"
        );
        assert_eq!(events[0], before[0], "{pool}");
        assert!(
            events[1].0 != events[0].0 && !says_unavailable(&events[1].0),
            "{pool}: coming back reads as going away: {events:#?}"
        );
    }
    assert_eq!(pool_provider_events(&setup, "pool-linux").await?, []);
    setup.finish().await
}

/// The state and reason `pool-mac` is left with by a pass while the manager
/// says the Tart agent failed as `error` (its `providerErrors.tart` entry).
async fn skipped_pool(error: Value) -> Result<(String, Option<String>)> {
    let setup = Setup::new(
        "UPDATE runner_pools SET desired_count=0 WHERE id='pool-mac';",
        tart_lane::SLOT_WATCH_INTERVAL,
    )
    .await?;
    *setup
        .manager
        .tart_down_error
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = error;
    setup.manager.tart_down.store(true, Ordering::SeqCst);
    tokio::time::timeout(PASS_LIMIT, run_pass(&setup.app)).await?;
    let state = pool_state(&setup, "pool-mac").await?;
    setup.finish().await?;
    Ok(state)
}

/// Whether a reason says the agent could not be reached at all.
fn says_unreachable(text: &str) -> bool {
    let lower = text.to_lowercase();
    lower.contains("unreachable") || lower.contains("cannot be reached")
}

/// Round 3, finding 1b: a pool skipped as `provider-unavailable` keeps a
/// reason, where the API finds a pool's reason (`runner_pool_waits`), that
/// says which provider and whether its agent cannot be reached or answers
/// with an error. Today the skip clears it.
#[tokio::test]
async fn a_skipped_mac_pool_says_which_provider_is_unavailable_and_how() -> Result<()> {
    let (state, unreachable) = skipped_pool(crate::start_tests::tart_unreachable()).await?;
    assert_eq!(state, "provider-unavailable");
    let unreachable =
        unreachable.context("the skipped pool has no reason while the Mac cannot be reached")?;
    let lower = unreachable.to_lowercase();
    assert!(
        lower.contains("macos") || lower.contains("tart"),
        "the reason does not name the provider: {unreachable}"
    );
    assert!(
        says_unreachable(&unreachable),
        "the reason does not say the agent cannot be reached: {unreachable}"
    );

    let (state, broken) = skipped_pool(json!({
        "kind": "error",
        "message": "Tart agent answered 401 Unauthorized: invalid token",
    }))
    .await?;
    assert_eq!(state, "provider-unavailable");
    let broken = broken.context("the skipped pool has no reason while the agent answers 401")?;
    let lower = broken.to_lowercase();
    assert!(
        lower.contains("macos") || lower.contains("tart"),
        "the reason does not name the provider: {broken}"
    );
    assert!(
        lower.contains("error") && !says_unreachable(&broken),
        "the reason does not say the agent answered with an error: {broken}"
    );
    Ok(())
}

/// Round 3, finding 2 (a regression against master): a Tart start whose
/// connection drops answers `tart_agent_unavailable`, but here the agent
/// goes on answering the runner list, so the Mac is not away: the agent
/// crashed on the start. That failure counts towards the pool's circuit, and
/// the third in a row opens it, as on master. Left deferred, an agent that
/// crashes on every start is retried every 30 s for ever. The Mac really
/// being away stays deferred and uncounted
/// (`a_start_cut_off_by_the_mac_sleeping_does_not_count_towards_the_circuit`).
#[tokio::test]
async fn a_tart_start_that_drops_while_the_agent_still_answers_counts_towards_the_circuit()
-> Result<()> {
    let setup = Setup::new(
        "UPDATE runner_pools SET provision_failure_count=2 WHERE id='pool-mac';",
        tart_lane::SLOT_WATCH_INTERVAL,
    )
    .await?;
    setup.manager.tart_start_drops.store(true, Ordering::SeqCst);
    tokio::time::timeout(PASS_LIMIT, run_pass(&setup.app)).await?;
    setup.tart_start_reached_manager().await?;
    setup.manager.tart_gate.add_permits(1);
    setup.tart_start_ended().await?;
    tokio::time::timeout(PASS_LIMIT, run_pass(&setup.app)).await?;

    let (state, failures, circuit_open): (String, i64, bool) = sqlx::query_as(
        "SELECT state,provision_failure_count,provision_circuit_open FROM runner_pools WHERE id='pool-mac'",
    )
    .fetch_one(&setup.app.database)
    .await?;
    assert_eq!(
        (failures, circuit_open),
        (3, true),
        "a start the agent dropped while it still answered was not counted (state {state})"
    );
    assert_eq!(state, "blocked");
    setup.manager.tart_gate.add_permits(1);
    setup.finish().await
}

/// The address of the Tart agent as the stand-in manager's errors give it
/// (`tart_unreachable`, `tart_agent_unavailable`), and its port.
const AGENT_ADDRESS: [&str; 3] = ["192.0.2.10", ":8787", "http://"];

/// Every text the API serves to a `pools:read` user that a pass writes: each
/// event's message and metadata, each pool's state and wait reason, each
/// runner's failure reason.
async fn served_text(setup: &Setup) -> Result<Vec<String>> {
    let mut texts: Vec<String> = sqlx::query_scalar(
        "SELECT event||' '||message||' '||metadata FROM runner_events
         UNION ALL SELECT reason FROM runner_pool_waits
         UNION ALL SELECT state FROM runner_pools
         UNION ALL SELECT COALESCE(failure_reason,'') FROM runners",
    )
    .fetch_all(&setup.app.database)
    .await?;
    texts.retain(|text| !text.is_empty());
    Ok(texts)
}

fn leaked(texts: &[String]) -> Vec<&String> {
    texts
        .iter()
        .filter(|text| AGENT_ADDRESS.iter().any(|part| text.contains(part)))
        .collect()
}

/// Round 3, finding 3: the manager's message for an agent it cannot reach
/// names the agent's internal URL (`error sending request for url
/// (http://<mac>:8787/...)`). A `pools:read` user must not read it: what the
/// pass writes where the API serves it (events and their metadata, a pool's
/// wait reason, a runner's failure reason) holds a fixed message per kind of
/// failure, with no URL, host or port. The detail goes to the log only.
/// Covers the skipped pool (unreachable, then answering with an error that
/// also names the URL) and a start cut off by the Mac sleeping, recorded as
/// the pool's wait once the Mac answers.
#[tokio::test]
async fn the_tart_agents_address_reaches_no_event_or_reason_the_api_serves() -> Result<()> {
    let setup = Setup::new("", tart_lane::SLOT_WATCH_INTERVAL).await?;
    tokio::time::timeout(PASS_LIMIT, run_pass(&setup.app)).await?;
    setup.tart_start_reached_manager().await?;
    setup.manager.tart_down.store(true, Ordering::SeqCst);
    setup.manager.tart_gate.add_permits(1);
    setup.tart_start_ended().await?;

    let logged = logged_pass(&setup.app).await?;
    assert!(
        logged.iter().any(|line| line.contains("192.0.2.10")),
        "the detail no longer reaches the log: {logged:#?}"
    );
    *setup
        .manager
        .tart_down_error
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = json!({
        "kind": "error",
        "message": "Tart agent answered 502 Bad Gateway for url (http://192.0.2.10:8787/v1/runners)",
    });
    tokio::time::timeout(PASS_LIMIT, run_pass(&setup.app)).await?;
    let texts = served_text(&setup).await?;
    assert_eq!(
        leaked(&texts),
        Vec::<&String>::new(),
        "the Tart agent's address is served while the Mac cannot be asked"
    );

    // The Mac wakes; the pass records how the cut-off start ended.
    setup.manager.tart_down.store(false, Ordering::SeqCst);
    tokio::time::timeout(PASS_LIMIT, run_pass(&setup.app)).await?;
    let texts = served_text(&setup).await?;
    assert_eq!(
        leaked(&texts),
        Vec::<&String>::new(),
        "the Tart agent's address is served once the Mac answered"
    );
    setup.manager.tart_gate.add_permits(1);
    setup.finish().await
}

/// Round 3, finding 4: the Mac's agent stops being unreachable and starts
/// answering with an error (the Mac woke with a rotated token). That is a
/// different failure, which needs someone: a new unavailable event is
/// recorded for it, once, and the skipped pool's reason follows.
#[tokio::test]
async fn a_change_in_how_the_tart_agent_fails_is_recorded_once() -> Result<()> {
    let setup = Setup::new(
        "UPDATE runner_pools SET desired_count=0 WHERE id='pool-mac';",
        tart_lane::SLOT_WATCH_INTERVAL,
    )
    .await?;
    setup.manager.tart_down.store(true, Ordering::SeqCst);
    tokio::time::timeout(PASS_LIMIT, run_pass(&setup.app)).await?;
    let unreachable = provider_events(&setup).await?;
    assert_eq!(unreachable.len(), 1, "{unreachable:#?}");

    *setup
        .manager
        .tart_down_error
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = json!({
        "kind": "error",
        "message": "Tart agent answered 401 Unauthorized: invalid token",
    });
    tokio::time::timeout(PASS_LIMIT, run_pass(&setup.app)).await?;
    tokio::time::timeout(PASS_LIMIT, run_pass(&setup.app)).await?;
    let events = provider_events(&setup).await?;
    assert_eq!(
        events.len(),
        2,
        "the agent answering with an error, after being unreachable, is not recorded once: {events:#?}"
    );
    assert_eq!(events[0], unreachable[0]);
    let broken = format!("{} {}", events[1].0, events[1].1).to_lowercase();
    assert!(
        says_unavailable(&events[1].0) || says_unavailable(&events[1].1),
        "{events:#?}"
    );
    assert!(
        broken.contains("error") && !says_unreachable(&broken),
        "the new record does not say the agent answered with an error: {broken}"
    );
    let (_, reason) = pool_state(&setup, "pool-mac").await?;
    let reason = reason.unwrap_or_default();
    assert!(
        reason.to_lowercase().contains("error") && !says_unreachable(&reason),
        "the skipped pool's reason did not follow the change: {reason:?}"
    );
    setup.finish().await
}

/// Round 3, GREEN review B1 (a regression against round 2): a skipped pool
/// keeps the outage as its reason in `runner_pool_waits`, and any row there
/// counts as a wait recorded (`wait_recorded`), which makes a pool held back
/// or still in its backoff show `waiting`. So once the Mac answers, the
/// outage must be over for the pool too: a pool in backoff before the Mac went
/// away is back in `backoff`, with no reason left saying the Mac cannot be
/// reached, not `waiting` with that stale reason.
#[tokio::test]
async fn a_pool_in_backoff_when_the_mac_went_away_is_back_in_backoff_without_the_outage_reason_once_it_answers()
-> Result<()> {
    let setup = Setup::new(
        "UPDATE runner_pools SET desired_count=1,provision_failure_count=1,
           provision_retry_at=9999999999999,state='backoff' WHERE id='pool-mac';",
        tart_lane::SLOT_WATCH_INTERVAL,
    )
    .await?;
    tokio::time::timeout(PASS_LIMIT, run_pass(&setup.app)).await?;
    assert_eq!(
        pool_state(&setup, "pool-mac").await?,
        ("backoff".to_owned(), None),
        "the setup: the pool waits out its backoff"
    );

    setup.manager.tart_down.store(true, Ordering::SeqCst);
    tokio::time::timeout(PASS_LIMIT, run_pass(&setup.app)).await?;
    let (during, _) = pool_state(&setup, "pool-mac").await?;
    assert_eq!(
        during, "provider-unavailable",
        "the setup: the pool is skipped"
    );

    setup.manager.tart_down.store(false, Ordering::SeqCst);
    tokio::time::timeout(PASS_LIMIT, run_pass(&setup.app)).await?;
    let (state, reason) = pool_state(&setup, "pool-mac").await?;
    assert!(
        !reason.as_deref().is_some_and(says_unavailable),
        "the Mac answers again, but the pool's reason still says it is unavailable: state {state:?}, reason {reason:?}"
    );
    assert_eq!(
        (state.as_str(), reason.as_deref()),
        ("backoff", None),
        "the pool is not back in its backoff once the Mac answers"
    );
    setup.finish().await
}
