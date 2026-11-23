//! The job category backfill (#190): classifies `workflow_jobs` and
//! `gitlab_jobs` rows whose `category` is still NULL, in bounded batches,
//! and, when the rules change, classifies the last 7 days again and lists
//! their hours in `fleet_health_recompute` for the rollups. The rules
//! version it last applied is kept as the `fleet_health.rules_version`
//! setting (no schema of its own).
//!
//! [`run_pass`] runs every reconciler pass, log-and-continue (`reconcile()`).
//! #197 fixes the signature; #190 implements it.

use sqlx::SqlitePool;

/// One reconciler pass of the backfill.
///
/// # Errors
/// A database read or write failed; `reconcile()` logs it and goes on.
#[allow(clippy::unused_async, reason = "#190 classifies the NULL rows here")]
pub(crate) async fn run_pass(db: &SqlitePool, now: i64) -> anyhow::Result<()> {
    let _ = (db, now);
    Ok(())
}
