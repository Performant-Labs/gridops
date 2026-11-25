//! The fleet health passes in their order (#197, #204): classify new jobs
//! (#190), roll up the hours (#191, #200), then snapshot the scores (#204).
//! The one copy of that order: the binary's `reconcile()` runs it every pass
//! and logs each failure, and the library's `fleet_health_pass` (the
//! end-to-end test's) runs it and returns the first, so the two can't drift
//! apart.

use sqlx::SqlitePool;

/// Runs every fleet health pass at `now`, in order. A failing pass is handed
/// to `failed` with its name, and the passes after it still run: each is
/// log-and-continue, and none depends on another having succeeded.
pub(crate) async fn run(
    db: &SqlitePool,
    now: i64,
    mut failed: impl FnMut(&'static str, anyhow::Error),
) {
    if let Err(error) = crate::fleet_health_backfill::run_pass(db, now).await {
        failed("job category backfill", error);
    }
    if let Err(error) = crate::fleet_health_rollup::run_pass(db, now).await {
        failed("hourly rollup", error);
    }
    if let Err(error) = crate::fleet_host_rollup::run_pass(db, now).await {
        failed("machine rollup", error);
    }
    if let Err(error) = crate::fleet_health_snapshot::run_pass(db, now).await {
        failed("snapshot", error);
    }
}

#[cfg(test)]
mod tests {
    use anyhow::Result;
    use gridops_core::{
        connect_database_path,
        fleet_health::testdata::{self, SampleSeed},
    };

    use super::run;

    const NOW: i64 = 1_800_000_000_000;

    /// A failing pass is reported by name and the passes after it still
    /// run: with the job rollup's recompute list gone, the machine rollup
    /// still writes its hours.
    #[tokio::test]
    async fn a_failing_pass_is_named_and_the_later_ones_still_run() -> Result<()> {
        let directory =
            std::env::temp_dir().join(format!("gridops-fh-passes-{}", uuid::Uuid::new_v4()));
        let db = connect_database_path(&directory.join("gridops.sqlite")).await?;
        let fleet = testdata::seed_fleet(&db).await?;
        testdata::insert_samples(&db, fleet.host("io")?, &[SampleSeed::at(NOW - 3_600_000)])
            .await?;
        sqlx::query("DROP TABLE fleet_health_recompute")
            .execute(&db)
            .await?;
        let mut failures = Vec::new();
        run(&db, NOW, |what, _| failures.push(what)).await;
        assert_eq!(failures, ["hourly rollup"]);
        let host_rows: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM fleet_host_hourly")
            .fetch_one(&db)
            .await?;
        assert!(host_rows > 0, "the machine rollup ran after the failure");
        db.close().await;
        std::fs::remove_dir_all(directory)?;
        Ok(())
    }
}
