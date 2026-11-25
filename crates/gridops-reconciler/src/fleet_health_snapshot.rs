//! The fleet health snapshots (#204): once an hour, the fleet score and every
//! machine's, per window (1 h, 24 h and 7 d), into `fleet_health_snapshots`,
//! which the page's trend reads. Kept for 90 days (`cleanup_retention`).
//!
//! [`run_pass`] runs every reconciler pass after the rollups,
//! log-and-continue (`reconcile()`). A window is recorded once per clock
//! hour, and only once the rollups have the hour that just ended (past its
//! 15-minute grace, caught up, and nothing listed for a recompute), so a
//! snapshot never scores a window the catch-up is still filling. The scores
//! are the page's: the same windowed computation
//! (`gridops_core::fleet_health::window`) with the speed indexes read fresh.
//! The reads happen outside the write transaction; the hour's rows (three at
//! most) are written in one short transaction. Past hours are never
//! backfilled.

use std::collections::BTreeMap;

use anyhow::{Context as _, Result};
use gridops_core::fleet_health::{
    HOUR_MILLIS, HealthWindow,
    hour::{floor_hour, is_past_grace},
    score, speed, window,
};
use sqlx::SqlitePool;

use crate::{fleet_health_rollup::ROLLUP_NEXT_SETTING, fleet_host_rollup::LAST_ROLLED_KEY};

/// One reconciler pass of the snapshots: for each window without a snapshot
/// in the current clock hour, once the hour that just ended is rolled up,
/// its scores at `now`.
///
/// # Errors
/// A database read or write failed; `reconcile()` logs it and goes on.
pub(crate) async fn run_pass(db: &SqlitePool, now: i64) -> Result<()> {
    let this_hour = floor_hour(now);
    if !rolled_up(db, this_hour - HOUR_MILLIS, now).await? {
        return Ok(());
    }
    let mut rows = Vec::new();
    for window in HealthWindow::ALL {
        let recorded: Option<i64> = sqlx::query_scalar(
            r#"SELECT recorded_at FROM fleet_health_snapshots
                WHERE recorded_at>=? AND "window"=? LIMIT 1"#,
        )
        .bind(this_hour)
        .bind(window.as_str())
        .fetch_optional(db)
        .await?;
        if recorded.is_some() {
            continue;
        }
        let speed = speed::speed_indexes(db, window, now).await?;
        let inputs = window::inputs(db, window, now, speed).await?;
        let scored = score::score_fleet(&inputs);
        let hosts: BTreeMap<String, Option<u8>> = scored
            .hosts
            .into_iter()
            .map(|host| (host.host_id, host.score))
            .collect();
        rows.push((window, scored.score, serde_json::to_string(&hosts)?));
    }
    if rows.is_empty() {
        return Ok(());
    }
    let mut tx = db.begin().await?;
    for (window, fleet_score, hosts) in rows {
        sqlx::query(
            r#"INSERT INTO fleet_health_snapshots (recorded_at,"window",fleet_score,host_scores_json)
               VALUES (?,?,?,?) ON CONFLICT(recorded_at,"window") DO NOTHING"#,
        )
        .bind(now)
        .bind(window.as_str())
        .bind(fleet_score.map(i64::from))
        .bind(hosts)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok(())
}

/// Whether both rollups have rolled `hour` up as of `now`: it is past its
/// grace, the job rollup's catch-up has gone past it, the machine rollup's
/// has reached it, and no hour up to it waits in `fleet_health_recompute`.
async fn rolled_up(db: &SqlitePool, hour: i64, now: i64) -> Result<bool> {
    if !is_past_grace(hour, now) {
        return Ok(false);
    }
    let setting = |key: &'static str| async move {
        sqlx::query_scalar::<_, String>("SELECT value FROM settings WHERE key=?")
            .bind(key)
            .fetch_optional(db)
            .await?
            .map(|value| {
                value
                    .trim_matches('"')
                    .parse::<i64>()
                    .with_context(|| format!("setting {key} is not an hour: {value}"))
            })
            .transpose()
    };
    let jobs_next = setting(ROLLUP_NEXT_SETTING).await?;
    let hosts_last = setting(LAST_ROLLED_KEY).await?;
    let listed: Option<i64> =
        sqlx::query_scalar("SELECT hour FROM fleet_health_recompute WHERE hour<=? LIMIT 1")
            .bind(hour)
            .fetch_optional(db)
            .await?;
    Ok(jobs_next.is_some_and(|next| next > hour)
        && hosts_last.is_some_and(|last| last >= hour)
        && listed.is_none())
}

#[cfg(test)]
mod tests {
    //! #204 item 2: after the hour that just ended is rolled up, one
    //! snapshot per window (1 h, 24 h, 7 d) each hour, recording the score
    //! the page computes at that moment.

    use std::collections::BTreeMap;

    use anyhow::Result;
    use gridops_core::{
        connect_database_path,
        fleet_health::{
            HealthWindow,
            hour::floor_hour,
            score, speed,
            testdata::{self, JobSeed, SampleSeed, SeededFleet},
            window,
        },
    };
    use sqlx::SqlitePool;

    use super::run_pass;
    use crate::{fleet_health_rollup, fleet_host_rollup};

    const MINUTE: i64 = 60_000;
    const HOUR: i64 = 3_600_000;
    /// A whole hour.
    const NOW: i64 = 1_800_000_000_000;

    struct Harness {
        directory: std::path::PathBuf,
        db: SqlitePool,
        fleet: SeededFleet,
    }

    impl Harness {
        /// The seeded fleet with 30 jobs and a day of samples on io in the
        /// hours before `NOW`.
        async fn new() -> Result<Self> {
            let directory =
                std::env::temp_dir().join(format!("gridops-snapshot-{}", uuid::Uuid::new_v4()));
            let db = connect_database_path(&directory.join("gridops.sqlite")).await?;
            let fleet = testdata::seed_fleet(&db).await?;
            let io = fleet.host("io")?.clone();
            let jobs: Vec<JobSeed> = (0..30)
                .map(|index| {
                    let completed = NOW - 3 * HOUR + index * 4 * MINUTE;
                    JobSeed::completed(
                        index + 1,
                        "unit tests",
                        Some(io.id.as_str()),
                        completed - 5 * MINUTE,
                        completed - 3 * MINUTE,
                        completed,
                    )
                })
                .collect();
            testdata::insert_jobs(&db, &fleet, &jobs).await?;
            let samples: Vec<SampleSeed> = (0..24 * 60)
                .map(|minute| SampleSeed::at(NOW - 24 * HOUR + minute * MINUTE))
                .collect();
            testdata::insert_samples(&db, &io, &samples).await?;
            Ok(Self {
                directory,
                db,
                fleet,
            })
        }

        /// Rolls up every hour of the last day before `hour_end` and moves
        /// both rollups' catch-up markers past it, as the catch-up leaves
        /// them once it has caught up.
        async fn roll_up_to(&self, hour_end: i64) -> Result<()> {
            let mut hour = hour_end - 24 * HOUR;
            while hour < hour_end {
                fleet_health_rollup::rollup_hour(&self.db, hour, hour_end).await?;
                fleet_host_rollup::rollup_hour(&self.db, hour, hour_end).await?;
                hour += HOUR;
            }
            for (key, value) in [
                (fleet_health_rollup::ROLLUP_NEXT_SETTING, hour_end),
                (fleet_host_rollup::LAST_ROLLED_KEY, hour_end - HOUR),
            ] {
                sqlx::query(
                    "INSERT INTO settings (key,value,updated_at) VALUES (?,?,?)
                     ON CONFLICT(key) DO UPDATE SET value=excluded.value",
                )
                .bind(key)
                .bind(value.to_string())
                .bind(hour_end)
                .execute(&self.db)
                .await?;
            }
            Ok(())
        }

        /// The snapshots: (`recorded_at`, window, fleet score, host scores).
        async fn snapshots(&self) -> Result<Vec<(i64, String, Option<i64>, String)>> {
            Ok(sqlx::query_as(
                r#"SELECT recorded_at,"window",fleet_score,host_scores_json
                     FROM fleet_health_snapshots ORDER BY recorded_at,"window""#,
            )
            .fetch_all(&self.db)
            .await?)
        }

        async fn finish(self) -> Result<()> {
            self.db.close().await;
            std::fs::remove_dir_all(self.directory)?;
            Ok(())
        }
    }

    /// The fleet score and every machine's of `window` at `now`, as the
    /// page computes them.
    async fn page_scores(
        db: &SqlitePool,
        window: HealthWindow,
        now: i64,
    ) -> Result<(Option<u8>, BTreeMap<String, Option<u8>>)> {
        let speed = speed::speed_indexes(db, window, now).await?;
        let inputs = window::inputs(db, window, now, speed).await?;
        let scored = score::score_fleet(&inputs);
        Ok((
            scored.score,
            scored
                .hosts
                .into_iter()
                .map(|host| (host.host_id, host.score))
                .collect(),
        ))
    }

    /// Once the hour that just ended is rolled up (past its 15-minute
    /// grace), each window gets one snapshot holding the page's scores of
    /// that moment, and a later pass in the same hour writes none.
    #[tokio::test]
    async fn one_snapshot_per_window_once_the_ended_hour_is_rolled_up() -> Result<()> {
        let harness = Harness::new().await?;
        harness.roll_up_to(NOW).await?;
        let at = NOW + 16 * MINUTE;
        run_pass(&harness.db, at).await?;
        let snapshots = harness.snapshots().await?;
        assert_eq!(
            snapshots
                .iter()
                .map(|(recorded, window, _, _)| (*recorded, window.as_str()))
                .collect::<Vec<_>>(),
            [(at, "1h"), (at, "24h"), (at, "7d")]
        );
        for (_, window, fleet_score, hosts) in &snapshots {
            let window: HealthWindow = window.parse()?;
            let (fleet, machines) = page_scores(&harness.db, window, at).await?;
            assert_eq!(
                fleet_score.map(u8::try_from).transpose()?,
                fleet,
                "{window}"
            );
            let stored: BTreeMap<String, Option<u8>> = serde_json::from_str(hosts)?;
            assert_eq!(stored, machines, "{window}: every machine's score");
            assert_eq!(stored.len(), harness.fleet.hosts.len());
        }
        assert!(
            snapshots.iter().any(|(_, _, score, _)| score.is_some()),
            "the seeded day scores"
        );

        run_pass(&harness.db, NOW + 50 * MINUTE).await?;
        assert_eq!(
            harness.snapshots().await?.len(),
            3,
            "one per window an hour"
        );
        harness.finish().await
    }

    /// Nothing is recorded until the rollups have the hour that just ended:
    /// inside its grace, during the catch-up, or while a past hour is listed
    /// for a recompute. A snapshot then never scores a window with a hole the
    /// rollups have yet to fill.
    #[tokio::test]
    async fn no_snapshot_until_the_rollups_have_the_ended_hour() -> Result<()> {
        let harness = Harness::new().await?;
        // Inside the grace: the hour that just ended is not rolled yet.
        harness.roll_up_to(NOW - HOUR).await?;
        run_pass(&harness.db, NOW + 10 * MINUTE).await?;
        assert!(harness.snapshots().await?.is_empty(), "inside the grace");
        // Past it, but the catch-up has not got there.
        run_pass(&harness.db, NOW + 20 * MINUTE).await?;
        assert!(harness.snapshots().await?.is_empty(), "behind the catch-up");
        // Caught up, but an hour is listed for a recompute.
        harness.roll_up_to(NOW).await?;
        sqlx::query("INSERT INTO fleet_health_recompute (hour,requested_at) VALUES (?,?)")
            .bind(NOW - 3 * HOUR)
            .bind(NOW)
            .execute(&harness.db)
            .await?;
        run_pass(&harness.db, NOW + 20 * MINUTE).await?;
        assert!(harness.snapshots().await?.is_empty(), "a listed hour");
        sqlx::query("DELETE FROM fleet_health_recompute")
            .execute(&harness.db)
            .await?;
        run_pass(&harness.db, NOW + 20 * MINUTE).await?;
        assert_eq!(harness.snapshots().await?.len(), 3);
        harness.finish().await
    }

    /// The next hour gets its own snapshots: the trend gains a point per
    /// window per hour, never more.
    #[tokio::test]
    async fn the_next_hour_gets_its_own_snapshots() -> Result<()> {
        let harness = Harness::new().await?;
        harness.roll_up_to(NOW).await?;
        run_pass(&harness.db, NOW + 16 * MINUTE).await?;
        harness.roll_up_to(NOW + HOUR).await?;
        run_pass(&harness.db, NOW + HOUR + 15 * MINUTE).await?;
        run_pass(&harness.db, NOW + HOUR + 45 * MINUTE).await?;
        let hours: Vec<i64> = harness
            .snapshots()
            .await?
            .iter()
            .map(|(recorded, _, _, _)| floor_hour(*recorded))
            .collect();
        assert_eq!(hours, [NOW, NOW, NOW, NOW + HOUR, NOW + HOUR, NOW + HOUR]);
        harness.finish().await
    }
}
