//! The reconciler's fleet health passes as a library (#204), for
//! `gridops-api`'s end-to-end test: it seeds a database, runs the passes the
//! reconciler runs every pass, in the same order (`fleet_health_passes`, the
//! one copy both use), and reads the page through the API.
//!
//! The binary compiles these modules as its own and tests them there. In
//! test builds this library is empty, so cargo never runs their tests twice
//! (`test = false` alone doesn't stop a filtered `cargo test` from running
//! them).

#[cfg(not(test))]
mod fleet_health_backfill;
#[cfg(not(test))]
mod fleet_health_passes;
#[cfg(not(test))]
mod fleet_health_rollup;
#[cfg(not(test))]
mod fleet_health_snapshot;
#[cfg(not(test))]
mod fleet_host_rollup;

/// One reconciler pass of the fleet health work at `now`: every pass, in the
/// binary's order. The binary logs each failure and goes on; this runs them
/// all the same and returns the first failure, named.
///
/// # Errors
/// A pass failed.
#[cfg(not(test))]
pub async fn fleet_health_pass(db: &sqlx::SqlitePool, now: i64) -> anyhow::Result<()> {
    let mut first = None;
    fleet_health_passes::run(db, now, |what, error| {
        if first.is_none() {
            first = Some(error.context(format!("fleet health {what} failed")));
        }
    })
    .await;
    first.map_or(Ok(()), Err)
}
