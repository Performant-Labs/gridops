//! A fleet runner that cannot be registered and started gives up its
//! placement once it has spent the cap in `registering`, instead of retrying
//! every pass forever while it holds the machine's capacity (#130).
//!
//! Through the machine of `gitlab_lifecycle_tests::Fleet` serving a GitHub
//! organization pool, `Setup`'s fake GitHub (which can refuse every
//! registration with a 409 for a name an online runner holds), and the real
//! database.

use super::*;
use gitlab_lifecycle_tests::Fleet;
use gridops_core::fleet::ids::PlacementId;
use start_tests::PASS_LIMIT;

/// The cap, `fleet::REGISTRATION_TIMEOUT_MILLIS` (handoff-A D1): ten
/// minutes. Spelled out because the constant is private to `fleet`.
const CAP_MILLIS: i64 = 10 * 60_000;

async fn placement_state(fleet: &Fleet, placement: PlacementId) -> Result<String> {
    Ok(
        sqlx::query_scalar("SELECT state FROM workload_placements WHERE id=?")
            .bind(placement.to_string())
            .fetch_one(&fleet.setup.app.database)
            .await?,
    )
}

/// The states of the placement's capacity allocations.
async fn allocations(fleet: &Fleet, placement: PlacementId) -> Result<Vec<String>> {
    Ok(
        sqlx::query_scalar("SELECT state FROM capacity_allocations WHERE placement_id=?")
            .bind(placement.to_string())
            .fetch_all(&fleet.setup.app.database)
            .await?,
    )
}

async fn runner_status(fleet: &Fleet, runner: &str) -> Result<String> {
    Ok(sqlx::query_scalar("SELECT status FROM runners WHERE id=?")
        .bind(runner)
        .fetch_one(&fleet.setup.app.database)
        .await?)
}

/// The detail of every `kind` fleet event recorded for the placement.
async fn events(fleet: &Fleet, placement: PlacementId, kind: &str) -> Result<Vec<Value>> {
    let details: Vec<String> = sqlx::query_scalar(
        "SELECT detail_json FROM fleet_events WHERE placement_id=? AND kind=? ORDER BY cursor",
    )
    .bind(placement.to_string())
    .bind(kind)
    .fetch_all(&fleet.setup.app.database)
    .await?;
    Ok(details
        .iter()
        .map(|detail| serde_json::from_str(detail))
        .collect::<Result<_, _>>()?)
}

/// The placement entered `registering` `millis` ago.
async fn registering_for(fleet: &Fleet, placement: PlacementId, millis: i64) -> Result<()> {
    sqlx::query("UPDATE workload_placements SET updated_at=? WHERE id=? AND state='registering'")
        .bind(now_millis() - millis)
        .bind(placement.to_string())
        .execute(&fleet.setup.app.database)
        .await?;
    Ok(())
}

/// A pass places the pool's runner and its environment is prepared, so it
/// is `registering`, holding its capacity.
async fn prepared_runner(fleet: &Fleet) -> Result<(String, PlacementId)> {
    fleet.pass().await?;
    let (runner, placement) = fleet.newest().await?;
    fleet.prepared(placement).await?;
    assert_eq!(placement_state(fleet, placement).await?, "registering");
    Ok((runner, placement))
}

/// Still registering, and nothing of it let go: its capacity is held and its
/// runner row is still starting, so it counts towards the pool.
async fn assert_still_held(fleet: &Fleet, runner: &str, placement: PlacementId) -> Result<()> {
    assert_eq!(placement_state(fleet, placement).await?, "registering");
    let held = allocations(fleet, placement).await?;
    assert!(
        !held.is_empty() && held.iter().all(|state| state != "released"),
        "its capacity is held: {held:?}"
    );
    assert_eq!(runner_status(fleet, runner).await?, "starting");
    assert_eq!(
        events(fleet, placement, "placement_failed").await?,
        Vec::<Value>::new()
    );
    Ok(())
}

/// Given up: the placement failed, its capacity is released, its runner row
/// left `starting` so the pool replaces it, and the cause is recorded once
/// as a registration timeout with the numbers behind it. Returns the
/// recorded detail.
async fn assert_given_up(fleet: &Fleet, runner: &str, placement: PlacementId) -> Result<Value> {
    assert_eq!(placement_state(fleet, placement).await?, "failed");
    let released = allocations(fleet, placement).await?;
    assert!(
        !released.is_empty() && released.iter().all(|state| state == "released"),
        "its capacity is released: {released:?}"
    );
    let status = runner_status(fleet, runner).await?;
    assert!(
        matches!(status.as_str(), "failed" | "deleted"),
        "the runner row left starting: {status}"
    );
    let failed = events(fleet, placement, "placement_failed").await?;
    assert_eq!(failed.len(), 1, "recorded once: {failed:?}");
    let detail = &failed[0];
    assert_eq!(detail["reason"], "registration_timed_out", "{detail}");
    assert_eq!(detail["allowed_ms"], CAP_MILLIS, "{detail}");
    assert!(
        detail["elapsed_ms"]
            .as_i64()
            .is_some_and(|elapsed| elapsed >= CAP_MILLIS),
        "{detail}"
    );
    // Not a start failure (handoff-A D6): no immediate retry on another
    // machine; the pool's own provisioning replaces it.
    assert_eq!(
        events(fleet, placement, "placement_retried").await?,
        Vec::<Value>::new()
    );
    Ok(detail.clone())
}

/// The issue's case: GitHub refuses the runner's registration every time,
/// because an online runner already holds its name (a 409 that #155 rightly
/// never clears). Before the cap the placement keeps retrying with its
/// capacity held; once past the cap a pass gives it up and records why.
#[tokio::test]
async fn a_runner_github_never_registers_gives_up_its_placement_past_the_cap() -> Result<()> {
    let fleet = Fleet::github().await?;
    let github = Arc::clone(&fleet.setup.github_registrations);
    github.conflict.store(true, Ordering::SeqCst);
    let (runner, placement) = prepared_runner(&fleet).await?;

    fleet.pass().await?;
    fleet.pass().await?;
    assert!(
        github.asked.load(Ordering::SeqCst) >= 2,
        "each pass asked GitHub again"
    );
    assert_still_held(&fleet, &runner, placement).await?;

    registering_for(&fleet, placement, CAP_MILLIS + 60_000).await?;
    fleet.pass().await?;

    let detail = assert_given_up(&fleet, &runner, placement).await?;
    let message = detail["message"].as_str().unwrap_or_default();
    assert!(
        message.contains("409"),
        "the message carries the last error: {message}"
    );
    assert!(message.chars().count() <= 240, "{message}");
    fleet.finish().await
}

/// A registration that fails for a while and then succeeds, still within
/// the cap, starts the runner as usual. The cap counts from when the
/// placement entered `registering` (handoff-A D2), not from admission: this
/// one was admitted an hour ago, its prepare having taken most of it.
#[tokio::test]
async fn a_runner_registered_within_the_cap_starts_normally() -> Result<()> {
    let fleet = Fleet::github().await?;
    let github = Arc::clone(&fleet.setup.github_registrations);
    github.conflict.store(true, Ordering::SeqCst);
    let (runner, placement) = prepared_runner(&fleet).await?;
    fleet.pass().await?;
    assert_still_held(&fleet, &runner, placement).await?;

    sqlx::query("UPDATE workload_placements SET created_at=? WHERE id=?")
        .bind(now_millis() - 60 * 60_000)
        .bind(placement.to_string())
        .execute(&fleet.setup.app.database)
        .await?;
    registering_for(&fleet, placement, CAP_MILLIS - 60_000).await?;
    // Refused once more, nine minutes in: still within the cap.
    fleet.pass().await?;
    assert_still_held(&fleet, &runner, placement).await?;

    github.conflict.store(false, Ordering::SeqCst);
    fleet.pass().await?;

    assert_eq!(placement_state(&fleet, placement).await?, "starting");
    assert_eq!(fleet.commands(placement, "start_runner").await?, 1);
    assert_eq!(runner_status(&fleet, &runner).await?, "starting");
    assert_eq!(
        events(&fleet, placement, "placement_failed").await?,
        Vec::<Value>::new()
    );
    fleet.started(placement).await?;
    fleet.pass().await?;
    assert_eq!(placement_state(&fleet, placement).await?, "running");
    let held = allocations(&fleet, placement).await?;
    assert!(
        !held.is_empty() && held.iter().all(|state| state != "released"),
        "a running runner keeps its capacity: {held:?}"
    );
    fleet.finish().await
}

/// The cap does not depend on the attempt failing (handoff-A D3): a
/// placement whose machine lost its work authority makes no attempt at all,
/// and without the cap would hold its capacity until the machine came back,
/// and forever after a recovery bumped the machine's epoch.
#[tokio::test]
async fn a_placement_whose_machine_lost_authority_gives_up_past_the_cap() -> Result<()> {
    let fleet = Fleet::github().await?;
    let github = Arc::clone(&fleet.setup.github_registrations);
    let (runner, placement) = prepared_runner(&fleet).await?;
    // The machine's authority lapses, and no pass below renews it.
    sqlx::query("UPDATE fleet_hosts SET authority_expires_at=?")
        .bind(now_millis())
        .execute(&fleet.setup.app.database)
        .await?;

    tokio::time::timeout(PASS_LIMIT, run_pass(&fleet.setup.app)).await?;
    assert_eq!(
        github.asked.load(Ordering::SeqCst),
        0,
        "nothing is registered for a machine without authority"
    );
    assert_still_held(&fleet, &runner, placement).await?;

    registering_for(&fleet, placement, CAP_MILLIS + 60_000).await?;
    tokio::time::timeout(PASS_LIMIT, run_pass(&fleet.setup.app)).await?;

    assert_given_up(&fleet, &runner, placement).await?;
    fleet.finish().await
}
