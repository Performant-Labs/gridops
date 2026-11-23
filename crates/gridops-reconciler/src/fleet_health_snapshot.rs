//! The fleet health snapshots (#204, wave 2): once an hour, the fleet score
//! per window into `fleet_health_snapshots`, which the page's trend reads.
//!
//! [`run_pass`] runs every reconciler pass, log-and-continue (`reconcile()`).
//! #197 only creates the stub; #204 implements it.

use sqlx::SqlitePool;

/// One reconciler pass of the snapshots.
///
/// # Errors
/// A database read or write failed; `reconcile()` logs it and goes on.
#[allow(clippy::unused_async, reason = "#204 records the snapshot here")]
pub(crate) async fn run_pass(db: &SqlitePool, now: i64) -> anyhow::Result<()> {
    let _ = (db, now);
    Ok(())
}
