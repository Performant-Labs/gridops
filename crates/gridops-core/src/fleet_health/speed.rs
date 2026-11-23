//! The relative speed index (#193): how long a machine takes to run the jobs
//! the rest of the fleet also runs, at one runner size, against the fleet's
//! median. 1.0 is typical; 1.5 is half again as slow.
//!
//! The rules (`docs/design/fleet-health.md`): a job key is repository +
//! workflow name + job name with a trailing shard counter stripped; only
//! successful final-attempt runs completed inside the window count; the
//! index is the run-count-weighted geometric mean of (own median ÷ fleet
//! median) over keys with at least 5 runs on the machine at that size and 5
//! on the rest of the fleet; fewer than 3 such keys is `None`.
//!
//! #197 fixes the signatures; #193 implements them.

use sqlx::SqlitePool;

use super::{HealthWindow, SpeedResult};

/// The key a job's runs are compared under.
#[must_use]
pub fn job_key(repository: &str, workflow: &str, job_name: &str) -> String {
    format!("{repository} / {workflow} / {job_name}")
}

/// Every (machine, runner size) that ran jobs in the window ending at `now`.
#[allow(
    clippy::unused_async,
    reason = "#193 reads the window's jobs here; the callers already await it"
)]
pub async fn speed_indexes(
    db: &SqlitePool,
    window: HealthWindow,
    now: i64,
) -> anyhow::Result<Vec<SpeedResult>> {
    let _ = (db, window, now);
    Ok(Vec::new())
}
