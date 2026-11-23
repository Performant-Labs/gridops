//! The fleet health per-machine hourly rollup (#200): one
//! `fleet_host_hourly` row per (hour, machine) from its resource samples
//! (online minutes, CPU and memory p50/p95, disk headroom), its start
//! attempts and failures, and its uncertain placements. A machine with no
//! samples in the hour still gets a row (0 minutes, NULL percentiles).
//!
//! [`run_pass`] runs every reconciler pass, log-and-continue (`reconcile()`),
//! for the listed hours plus at most one catch-up hour, as #191's does. #197
//! fixes the signatures; #200 implements them.

use sqlx::SqlitePool;

/// Writes the rows of the hour starting at `hour`, replacing what is there.
///
/// # Errors
/// A database read or write failed.
#[allow(
    dead_code,
    clippy::unused_async,
    reason = "#200 writes the hour here; run_pass calls it"
)]
pub(crate) async fn rollup_hour(db: &SqlitePool, hour: i64, now: i64) -> anyhow::Result<()> {
    let _ = (db, hour, now);
    Ok(())
}

/// One reconciler pass of the per-machine rollup.
///
/// # Errors
/// A database read or write failed; `reconcile()` logs it and goes on.
#[allow(clippy::unused_async, reason = "#200 rolls up the listed hours here")]
pub(crate) async fn run_pass(db: &SqlitePool, now: i64) -> anyhow::Result<()> {
    let _ = (db, now);
    Ok(())
}
