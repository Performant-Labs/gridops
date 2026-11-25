//! The reconciler's fleet health passes as a library (#204), for
//! `gridops-api`'s end-to-end test: it seeds a database, runs the passes the
//! reconciler runs every pass (`fleet_health_passes` in `main.rs`), in the
//! same order, and reads the page through the API. The binary compiles these
//! modules as its own and tests them there; this library only re-exports
//! their passes.

mod fleet_health_backfill;
mod fleet_health_rollup;
mod fleet_health_snapshot;
mod fleet_host_rollup;

use sqlx::SqlitePool;

/// One reconciler pass of the fleet health work at `now`, in the binary's
/// order: classify new jobs (#190), roll up the hours (#191, #200), then
/// snapshot the scores (#204). Unlike the binary, which logs each failure
/// and goes on, the first failure is returned.
///
/// # Errors
/// A pass failed.
pub async fn fleet_health_pass(db: &SqlitePool, now: i64) -> anyhow::Result<()> {
    fleet_health_backfill::run_pass(db, now).await?;
    fleet_health_rollup::run_pass(db, now).await?;
    fleet_host_rollup::run_pass(db, now).await?;
    fleet_health_snapshot::run_pass(db, now).await?;
    Ok(())
}
