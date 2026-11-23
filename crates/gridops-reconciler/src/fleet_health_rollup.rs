//! The fleet health hourly rollup (#191): one `fleet_health_hourly` row per
//! (hour, machine, pool, category) of the final-attempt jobs that completed
//! in that hour, with job and success counts, CPU-seconds, the queue-wait and
//! duration histograms (`fleet_health::HISTOGRAM_LOWER_BOUNDS_MILLIS`), and
//! the start and infrastructure failures. Jobs no managed machine ran go
//! under `host_id = pool_id = 'unmanaged'` with 0 CPU-seconds.
//!
//! [`run_pass`] runs every reconciler pass, log-and-continue (`reconcile()`):
//! it rolls up every hour listed in `fleet_health_recompute` plus at most
//! one catch-up hour. #197 fixes the signatures; #191 implements them.

use sqlx::SqlitePool;

/// Writes the rows of the hour starting at `hour` (epoch ms, on a whole
/// hour), replacing what is there, and takes the hour off
/// `fleet_health_recompute`.
///
/// # Errors
/// A database read or write failed.
#[allow(
    dead_code,
    clippy::unused_async,
    reason = "#191 writes the hour here; run_pass calls it"
)]
pub(crate) async fn rollup_hour(db: &SqlitePool, hour: i64, now: i64) -> anyhow::Result<()> {
    let _ = (db, hour, now);
    Ok(())
}

/// One reconciler pass of the rollup.
///
/// # Errors
/// A database read or write failed; `reconcile()` logs it and goes on.
#[allow(clippy::unused_async, reason = "#191 rolls up the listed hours here")]
pub(crate) async fn run_pass(db: &SqlitePool, now: i64) -> anyhow::Result<()> {
    let _ = (db, now);
    Ok(())
}
