//! The fleet health hourly rollup (#191): one `fleet_health_hourly` row per
//! (hour, machine, pool, category) of the final-attempt jobs that completed
//! in that hour, with job and success counts, CPU-seconds, the queue-wait and
//! duration histograms (`fleet_health::HISTOGRAM_LOWER_BOUNDS_MILLIS`), and
//! the start and infrastructure failures. Jobs no managed machine ran go
//! under `host_id = pool_id = 'unmanaged'` with 0 CPU-seconds.
//!
//! [`run_pass`] runs every reconciler pass, log-and-continue (`reconcile()`):
//! it rolls up every hour listed in `fleet_health_recompute`, then catches
//! up as many finished hours as fit in [`CATCH_UP_BUDGET`] (one at least). #197 fixes the signatures; #191 implements them. It is
//! the only reader of `fleet_health_recompute` (#239).
//!
//! The reads of an hour (`gridops_core::fleet_health::hour::job_hour`, the
//! one copy the database source also computes live hours with, #204) run
//! outside the write transaction, bounded by the `completed_at` indexes
//! (`workflow_jobs_completed_idx`, `gitlab_jobs_completed_idx`); the hour's rows are then written in one
//! short transaction (at most [`WRITE_BATCH`] rows per transaction), which
//! also takes the hour off `fleet_health_recompute`, so re-running an hour
//! gives the same rows. The same transaction lists the hours a re-run's
//! earlier attempts completed in, so they are rolled again without them
//! (#239). The catch-up walks the last 7 days oldest first, under a time
//! budget per pass, one write transaction per hour; where it got to is the `fleet_health.rollup_next` setting,
//! absent until the first pass. No hour, listed or caught up, is rolled
//! before its end plus [`ROLLUP_GRACE_MILLIS`].

use anyhow::{Context as _, Result};
use gridops_core::fleet_health::{
    HOUR_MILLIS, HealthWindow, ROLLUP_GRACE_MILLIS,
    hour::{self, JobRowKey, JobTotals},
};
use sqlx::SqlitePool;

/// The settings key holding the next hour the catch-up rolls up.
pub(crate) const ROLLUP_NEXT_SETTING: &str = "fleet_health.rollup_next";

/// How many `fleet_health_hourly` rows one write transaction writes at most
/// (#179's SQLite discipline).
const WRITE_BATCH: usize = 500;

/// The other hours in which the earlier attempts of the hour's re-run jobs
/// (the same run and job name, earlier as `hour::GITHUB_JOBS` orders them)
/// completed (`workflow_jobs_run_status_idx`), over the last 7 days: each was
/// rolled while its attempt was still the run's newest, so it counts there
/// until the hour is rolled again (#239). The hour itself is left out; its own
/// rows already read only final attempts.
const EARLIER_ATTEMPT_HOURS: &str = r#"
SELECT DISTINCT earlier.completed_at-(earlier.completed_at%3600000) AS hour
  FROM workflow_jobs wj
  LEFT JOIN workflow_job_sync sync ON sync.job_id=wj.id
  JOIN workflow_jobs earlier ON earlier.run_id=wj.run_id AND earlier.name=wj.name
                            AND earlier.status='completed' AND earlier.id<>wj.id
  LEFT JOIN workflow_job_sync earlier_sync ON earlier_sync.job_id=earlier.id
 WHERE wj.completed_at>=? AND wj.completed_at<?
   AND (earlier_sync.run_attempt<sync.run_attempt
        OR ((earlier_sync.run_attempt IS NULL OR sync.run_attempt IS NULL)
            AND earlier.id<wj.id
            AND (SELECT run.run_attempt FROM workflow_runs run
                  WHERE run.id=wj.run_id)>1))
   AND earlier.completed_at>=?
   AND (earlier.completed_at<? OR earlier.completed_at>=?)
 ORDER BY hour"#;

/// Writes the rows of the hour starting at `hour` (epoch ms, on a whole
/// hour), replacing what is there, takes the hour off
/// `fleet_health_recompute`, and lists there the hours its re-runs' earlier
/// attempts completed in.
///
/// # Errors
/// A database read or write failed.
pub(crate) async fn rollup_hour(db: &SqlitePool, hour: i64, now: i64) -> Result<()> {
    let hour = hour - hour.rem_euclid(HOUR_MILLIS);
    let rows = hour::job_hour(db, hour, hour + HOUR_MILLIS).await?;
    let earlier = earlier_attempt_hours(db, hour, now).await?;
    write_hour(
        db,
        hour,
        now,
        &rows.into_iter().collect::<Vec<_>>(),
        &earlier,
    )
    .await
}

/// How much of a pass the catch-up may spend rolling hours (#204): enough
/// for a first start's 7 days to take a few passes instead of 168 (84
/// minutes at one per 30 s pass), while each hour is still its own short
/// write transaction (#179).
pub(crate) const CATCH_UP_BUDGET: std::time::Duration = std::time::Duration::from_secs(1);

/// One reconciler pass of the rollup: the catch-up's hours of the last 7
/// days that are past their grace, oldest first, for as long as
/// [`CATCH_UP_BUDGET`] lasts (at least one), then every hour listed in
/// `fleet_health_recompute` that is past its grace, oldest first (those the
/// catch-up just listed included).
///
/// # Errors
/// A database read or write failed; `reconcile()` logs it and goes on.
pub(crate) async fn run_pass(db: &SqlitePool, now: i64) -> Result<()> {
    run_pass_within(db, now, CATCH_UP_BUDGET).await
}

/// [`run_pass`] with the catch-up's time `budget`, counted from the start of
/// the pass: the catch-up rolls one hour at least, then more while the pass
/// is under its budget.
///
/// # Errors
/// A database read or write failed.
pub(crate) async fn run_pass_within(
    db: &SqlitePool,
    now: i64,
    budget: std::time::Duration,
) -> Result<()> {
    let started = std::time::Instant::now();
    // Catch-up: finished hours oldest first, over the last 7 days, each once
    // its grace has passed so a late arrival still counts, and each with its
    // marker saved after it, so a failure or the end of the budget resumes
    // there. The first pass starts at the window's oldest hour; one whose
    // marker the sliding window has passed starts again at its oldest.
    let window_start = now - HealthWindow::Week.millis();
    let window_start = window_start - window_start.rem_euclid(HOUR_MILLIS);
    let mut next = match next_hour(db).await? {
        Some(next) => next.max(window_start),
        None => window_start,
    };
    while next + HOUR_MILLIS + ROLLUP_GRACE_MILLIS <= now {
        rollup_hour(db, next, now).await?;
        next += HOUR_MILLIS;
        save_next_hour(db, next, now).await?;
        if started.elapsed() >= budget {
            break;
        }
    }

    // Then the listed hours, so an earlier attempt's hour the catch-up has
    // just listed (rolling a re-run's later hour) is rolled again in the
    // same pass.
    let listed: Vec<i64> =
        sqlx::query_scalar("SELECT hour FROM fleet_health_recompute WHERE hour<=? ORDER BY hour")
            .bind(now - HOUR_MILLIS - ROLLUP_GRACE_MILLIS)
            .fetch_all(db)
            .await?;
    for hour in listed {
        rollup_hour(db, hour, now).await?;
    }
    Ok(())
}

/// [`EARLIER_ATTEMPT_HOURS`] for the hour starting at `hour`, as of `now`.
///
/// # Errors
/// The read failed.
async fn earlier_attempt_hours(db: &SqlitePool, hour: i64, now: i64) -> Result<Vec<i64>> {
    let end = hour + HOUR_MILLIS;
    let oldest = now - HealthWindow::Week.millis();
    Ok(sqlx::query_scalar::<_, i64>(EARLIER_ATTEMPT_HOURS)
        .bind(hour)
        .bind(end)
        .bind(oldest - oldest.rem_euclid(HOUR_MILLIS))
        .bind(hour)
        .bind(end)
        .fetch_all(db)
        .await?)
}

/// Writes the hour's rows, replacing what is there and taking the hour off
/// `fleet_health_recompute`: one short transaction per [`WRITE_BATCH`] rows,
/// the first emptying the hour, the last also listing the `earlier` hours
/// (its re-runs' earlier attempts, never the hour itself) and emptying the
/// hour's own recompute entry (an hour with no jobs still gets both). An hour
/// is one (host, pool, category) group per job that ran, so it is far below
/// the batch size and one transaction writes it whole; the split only serves
/// #179's rule that no transaction writes more than 500 rows, should a fleet
/// ever grow that wide (the listings get a batch of their own when the last
/// one has no room), and a failure between batches leaves the hour listed
/// for a recompute that rewrites it from scratch.
///
/// # Errors
/// A write failed.
async fn write_hour(
    db: &SqlitePool,
    hour: i64,
    now: i64,
    rows: &[(JobRowKey, JobTotals)],
    earlier: &[i64],
) -> Result<()> {
    let mut batches = rows.chunks(WRITE_BATCH).collect::<Vec<_>>();
    if batches
        .last()
        .is_none_or(|batch| batch.len() + earlier.len() > WRITE_BATCH)
    {
        batches.push(&[][..]);
    }
    let last = batches.len() - 1;
    for (index, batch) in batches.into_iter().enumerate() {
        let mut tx = db.begin().await?;
        if index == 0 {
            sqlx::query("DELETE FROM fleet_health_hourly WHERE hour=?")
                .bind(hour)
                .execute(&mut *tx)
                .await?;
        }
        for ((host, pool, category), totals) in batch {
            sqlx::query(
                "INSERT INTO fleet_health_hourly
                   (hour,host_id,pool_id,category,job_count,completed_count,infra_failure_count,
                    duration_ms_sum,cpu_seconds_sum,queue_wait_histogram,duration_histogram,computed_at)
                 VALUES (?,?,?,?,?,?,?,?,?,?,?,?)
                 ON CONFLICT(hour,host_id,pool_id,category) DO UPDATE SET
                   job_count=excluded.job_count,completed_count=excluded.completed_count,
                   infra_failure_count=excluded.infra_failure_count,
                   duration_ms_sum=excluded.duration_ms_sum,cpu_seconds_sum=excluded.cpu_seconds_sum,
                   queue_wait_histogram=excluded.queue_wait_histogram,
                   duration_histogram=excluded.duration_histogram,computed_at=excluded.computed_at",
            )
            .bind(hour)
            .bind(host)
            .bind(pool)
            .bind(category.as_str())
            .bind(totals.job_count)
            .bind(totals.completed_count)
            .bind(totals.infra_failure_count)
            .bind(totals.duration_ms_sum)
            .bind(totals.cpu_seconds())
            .bind(serde_json::to_string(&totals.wait)?)
            .bind(serde_json::to_string(&totals.duration)?)
            .bind(now)
            .execute(&mut *tx)
            .await?;
        }
        if index == last {
            for listed in earlier {
                sqlx::query(
                    "INSERT INTO fleet_health_recompute (hour,requested_at) VALUES (?,?)
                     ON CONFLICT(hour) DO NOTHING",
                )
                .bind(listed)
                .bind(now)
                .execute(&mut *tx)
                .await?;
            }
            sqlx::query("DELETE FROM fleet_health_recompute WHERE hour=?")
                .bind(hour)
                .execute(&mut *tx)
                .await?;
        }
        tx.commit().await?;
    }
    Ok(())
}

/// The hour the catch-up rolls up next, if it ever has.
///
/// # Errors
/// The read failed, or the stored value is not an hour.
async fn next_hour(db: &SqlitePool) -> Result<Option<i64>> {
    let stored = sqlx::query_scalar::<_, String>("SELECT value FROM settings WHERE key=?")
        .bind(ROLLUP_NEXT_SETTING)
        .fetch_optional(db)
        .await?;
    stored
        .map(|value| {
            value
                .parse::<i64>()
                .with_context(|| format!("setting {ROLLUP_NEXT_SETTING} is not an hour: {value}"))
        })
        .transpose()
}

/// Saves the catch-up's next hour.
///
/// # Errors
/// The write failed.
async fn save_next_hour(db: &SqlitePool, hour: i64, now: i64) -> Result<()> {
    sqlx::query(
        "INSERT INTO settings (key,value,updated_at) VALUES (?,?,?)
         ON CONFLICT(key) DO UPDATE SET value=excluded.value,updated_at=excluded.updated_at",
    )
    .bind(ROLLUP_NEXT_SETTING)
    .bind(hour.to_string())
    .bind(now)
    .execute(db)
    .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    //! #239's rollup fixes: a re-run whose attempts complete in different
    //! hours counts once (item 6), and no hour, listed or caught up, is
    //! rolled before `hour_end + 15 min`, so a late arrival inside that grace
    //! still counts (item 7 and the plan review's finding 1).

    use super::*;
    use gridops_core::{
        connect_database_path,
        fleet_health::testdata::{self, JobSeed, SeededFleet},
    };

    const MINUTE: i64 = 60_000;
    const HOUR: i64 = 3_600_000;
    /// The grace an ended hour waits before it is rolled (#239).
    const GRACE: i64 = 15 * MINUTE;
    /// A whole hour.
    const NOW: i64 = 1_800_000_000_000;

    struct Harness {
        directory: std::path::PathBuf,
        db: SqlitePool,
        fleet: SeededFleet,
    }

    impl Harness {
        async fn new() -> Result<Self> {
            let directory = std::env::temp_dir()
                .join(format!("gridops-job-rollup-own-{}", uuid::Uuid::new_v4()));
            let db = connect_database_path(&directory.join("gridops.sqlite")).await?;
            let fleet = testdata::seed_fleet(&db).await?;
            Ok(Self {
                directory,
                db,
                fleet,
            })
        }

        /// A successful job on io completing at `completed`, part of run
        /// `run_id` at `attempt`.
        async fn job(&self, id: i64, run_id: i64, attempt: i64, completed: i64) -> Result<()> {
            self.named_job(id, run_id, attempt, "unit tests", completed)
                .await
        }

        /// [`Self::job`] with the job's `name`.
        async fn named_job(
            &self,
            id: i64,
            run_id: i64,
            attempt: i64,
            name: &str,
            completed: i64,
        ) -> Result<()> {
            let io = self.fleet.host("io")?.id.clone();
            let mut job = JobSeed::completed(
                id,
                name,
                Some(&io),
                completed - 3 * MINUTE,
                completed - 2 * MINUTE,
                completed,
            );
            job.run_id = run_id;
            job.run_attempt = attempt;
            testdata::insert_jobs(&self.db, &self.fleet, &[job]).await
        }

        /// The jobs `fleet_health_hourly` counts in `hour`.
        async fn counted(&self, hour: i64) -> Result<i64> {
            Ok(sqlx::query_scalar(
                "SELECT COALESCE(SUM(job_count),0) FROM fleet_health_hourly WHERE hour=?",
            )
            .bind(hour)
            .fetch_one(&self.db)
            .await?)
        }

        async fn listed(&self) -> Result<Vec<i64>> {
            Ok(
                sqlx::query_scalar("SELECT hour FROM fleet_health_recompute ORDER BY hour")
                    .fetch_all(&self.db)
                    .await?,
            )
        }

        async fn list(&self, hour: i64, at: i64) -> Result<()> {
            sqlx::query("INSERT INTO fleet_health_recompute (hour,requested_at) VALUES (?,?)")
                .bind(hour)
                .bind(at)
                .execute(&self.db)
                .await?;
            Ok(())
        }

        async fn finish(self) -> Result<()> {
            self.db.close().await;
            std::fs::remove_dir_all(self.directory)?;
            Ok(())
        }
    }

    /// A "re-run failed jobs" attempt runs only the failed job: the run's
    /// other attempt-1 jobs are not run again and still count, in their own
    /// hour, whenever it is rolled. Only the re-run job's earlier attempt is
    /// replaced, and only its hour is listed for a recompute.
    #[tokio::test]
    async fn a_partial_re_run_keeps_the_jobs_it_did_not_run_again() -> Result<()> {
        let harness = Harness::new().await?;
        let (a, b, c) = (NOW - 4 * HOUR, NOW - 3 * HOUR, NOW - 2 * HOUR);
        harness
            .named_job(1, 60, 1, "build", a + 10 * MINUTE)
            .await?;
        harness.named_job(2, 60, 1, "lint", b + 10 * MINUTE).await?;
        harness.named_job(3, 60, 2, "lint", c + 10 * MINUTE).await?;
        rollup_hour(&harness.db, c, NOW).await?;
        assert_eq!(
            harness.listed().await?,
            [b],
            "only the re-run job's earlier hour is listed"
        );
        rollup_hour(&harness.db, a, NOW).await?;
        rollup_hour(&harness.db, b, NOW).await?;
        assert_eq!(harness.counted(a).await?, 1, "build was not run again");
        assert_eq!(
            harness.counted(b).await?,
            0,
            "lint's attempt 1 was replaced"
        );
        assert_eq!(harness.counted(c).await?, 1, "lint's attempt 2");
        harness.finish().await
    }

    /// GitHub doesn't always give a job's attempt: a re-run job whose
    /// attempt is unknown is later than the same-named job it re-runs by its
    /// higher job id, so the run still counts once and the earlier hour is
    /// listed. An unknown attempt never makes an earlier job id replace a
    /// later one.
    #[tokio::test]
    async fn a_re_run_job_with_no_known_attempt_still_replaces_the_earlier_one() -> Result<()> {
        let harness = Harness::new().await?;
        let (a, b) = (NOW - 3 * HOUR, NOW - 2 * HOUR);
        harness.job(1, 70, 1, a + 30 * MINUTE).await?;
        harness.job(2, 70, 2, b + 10 * MINUTE).await?;
        sqlx::query("UPDATE workflow_job_sync SET run_attempt=NULL WHERE job_id=2")
            .execute(&harness.db)
            .await?;
        rollup_hour(&harness.db, b, NOW).await?;
        assert_eq!(harness.counted(b).await?, 1, "the re-run job counts");
        assert_eq!(harness.listed().await?, [a], "the earlier hour is listed");
        rollup_hour(&harness.db, a, NOW).await?;
        assert_eq!(harness.counted(a).await?, 0, "the earlier job was replaced");
        harness.finish().await
    }

    /// Two jobs of one attempt may share a name. With an attempt unknown and
    /// a run never re-run, neither replaces the other: both count, and no
    /// hour is listed.
    #[tokio::test]
    async fn same_named_jobs_of_a_run_never_re_run_both_count() -> Result<()> {
        let harness = Harness::new().await?;
        let (a, b) = (NOW - 3 * HOUR, NOW - 2 * HOUR);
        harness.job(1, 80, 1, a + 30 * MINUTE).await?;
        harness.job(2, 80, 1, b + 10 * MINUTE).await?;
        sqlx::query("UPDATE workflow_job_sync SET run_attempt=NULL WHERE job_id IN (1,2)")
            .execute(&harness.db)
            .await?;
        rollup_hour(&harness.db, b, NOW).await?;
        assert_eq!(
            harness.listed().await?,
            Vec::<i64>::new(),
            "no hour is listed"
        );
        rollup_hour(&harness.db, a, NOW).await?;
        assert_eq!(harness.counted(a).await?, 1, "the lower-id job counts");
        assert_eq!(harness.counted(b).await?, 1, "the higher-id job counts");
        harness.finish().await
    }

    /// Item 6: attempt 1 completes in hour A and is rolled there; the re-run
    /// (attempt 2) completes in hour B. Rolling B lists A for a recompute (in
    /// the same write transaction, and never B itself), and the next pass
    /// re-rolls A, so the run counts once across the two hours.
    #[tokio::test]
    async fn a_re_run_whose_attempts_fall_in_different_hours_counts_once() -> Result<()> {
        let harness = Harness::new().await?;
        let (a, b) = (NOW - 3 * HOUR, NOW - 2 * HOUR);
        harness.job(1, 50, 1, a + 30 * MINUTE).await?;
        rollup_hour(&harness.db, a, NOW).await?;
        assert_eq!(harness.counted(a).await?, 1, "attempt 1, before the re-run");

        harness.job(2, 50, 2, b + 10 * MINUTE).await?;
        rollup_hour(&harness.db, b, NOW).await?;
        assert_eq!(harness.counted(b).await?, 1, "attempt 2 in its own hour");
        assert_eq!(
            harness.listed().await?,
            [a],
            "rolling B lists attempt 1's hour, and not B"
        );

        run_pass(&harness.db, NOW).await?;
        assert_eq!(
            harness.counted(a).await? + harness.counted(b).await?,
            1,
            "the run counts once over A and B"
        );
        assert!(
            harness.listed().await?.is_empty(),
            "A was taken off the list"
        );
        harness.finish().await
    }

    /// Item 7: the catch-up rolls an ended hour only once `hour_end + 15 min
    /// <= now`. A job finishing at 10:59:40 whose webhook was missed is
    /// recorded by the poll after 11:00; it is still in hour 10's row.
    #[tokio::test]
    async fn the_catch_up_waits_15_minutes_so_a_late_arrival_still_counts() -> Result<()> {
        let harness = Harness::new().await?;
        let hour = NOW - HOUR;
        save_next_hour(&harness.db, hour, NOW - HOUR).await?;
        harness.job(1, 1, 1, hour + 30 * MINUTE).await?;

        // 11:14:59: the hour ended but is still in its grace, so not rolled.
        run_pass(&harness.db, NOW + GRACE - 1_000).await?;
        assert_eq!(harness.counted(hour).await?, 0, "rolled inside the grace");

        // The late one: completed 10:59:40, stored after 11:00.
        harness.job(2, 2, 1, NOW - 20_000).await?;
        run_pass(&harness.db, NOW + GRACE).await?;
        assert_eq!(
            harness.counted(hour).await?,
            2,
            "rolled at 11:15 with the late arrival"
        );
        harness.finish().await
    }

    /// The plan review's finding 1: a listed hour waits for the same grace,
    /// so "an hour has rows ⟺ it is past its grace" holds for #204's source,
    /// which computes every hour not yet past it live.
    #[tokio::test]
    async fn a_listed_hour_also_waits_for_the_grace() -> Result<()> {
        let harness = Harness::new().await?;
        let hour = NOW - HOUR;
        harness.job(1, 1, 1, hour + 30 * MINUTE).await?;
        harness.list(hour, NOW + MINUTE).await?;

        run_pass(&harness.db, NOW + 5 * MINUTE).await?;
        assert_eq!(harness.counted(hour).await?, 0, "rolled inside the grace");
        assert_eq!(harness.listed().await?, [hour], "still listed");

        run_pass(&harness.db, NOW + GRACE).await?;
        assert_eq!(harness.counted(hour).await?, 1);
        assert!(harness.listed().await?.is_empty());
        harness.finish().await
    }

    /// #204: a pass catches up as many past-grace hours as fit in its time
    /// budget, oldest first, each in its own short write transaction, so a
    /// first start's 7 days take a few passes instead of 168 (84 minutes at
    /// one per 30 s pass). Every past-grace hour is rolled once in one pass
    /// here, and the marker ends at the hour still in its grace.
    #[tokio::test]
    async fn one_pass_catches_up_every_hour_its_budget_allows() -> Result<()> {
        let harness = Harness::new().await?;
        let (old, recent) = (NOW - 3 * 24 * HOUR, NOW - 2 * HOUR);
        harness.job(1, 1, 1, old + 10 * MINUTE).await?;
        harness.job(2, 2, 1, recent + 10 * MINUTE).await?;
        run_pass(&harness.db, NOW).await?;
        assert_eq!(harness.counted(old).await?, 1, "three days ago, rolled");
        assert_eq!(harness.counted(recent).await?, 1, "two hours ago, rolled");
        assert_eq!(
            next_hour(&harness.db).await?,
            Some(NOW - HOUR),
            "caught up to the hour in its grace"
        );
        harness.finish().await
    }

    /// The budget bounds a pass, but a pass always makes progress: with no
    /// time to spare it still rolls one hour, as before.
    #[tokio::test]
    async fn a_pass_with_no_budget_left_still_rolls_one_hour() -> Result<()> {
        let harness = Harness::new().await?;
        run_pass_within(&harness.db, NOW, std::time::Duration::ZERO).await?;
        let start = NOW - 7 * 24 * HOUR;
        assert_eq!(next_hour(&harness.db).await?, Some(start + HOUR));
        harness.finish().await
    }

    /// The whole synthetic week (about 50,000 jobs) catches up in a few
    /// passes with no statement over 100 ms and no write transaction over
    /// 50 ms (#179).
    #[tokio::test]
    async fn the_synthetic_week_catches_up_in_a_few_passes() -> Result<()> {
        let directory = std::env::temp_dir().join(format!(
            "gridops-job-rollup-catch-up-{}",
            uuid::Uuid::new_v4()
        ));
        let db = connect_database_path(&directory.join("gridops.sqlite")).await?;
        let week = testdata::build_week(&db, NOW).await?;
        let passes = gridops_core::fleet_health::speed_check::check(&db, async {
            let mut passes = 0;
            while next_hour(&db).await? != Some(NOW - HOUR) {
                run_pass(&db, NOW).await?;
                passes += 1;
                anyhow::ensure!(passes < 168, "no faster than one hour a pass");
            }
            Ok(passes)
        })
        .await?;
        println!("the synthetic week caught up in {passes} passes");
        assert!(passes <= 24, "{passes} passes");
        let counted: i64 =
            sqlx::query_scalar("SELECT COALESCE(SUM(job_count),0) FROM fleet_health_hourly")
                .fetch_one(&db)
                .await?;
        // The week ends at NOW; its last hour is still in its grace.
        assert!(counted > 0 && counted <= i64::try_from(week.jobs)?);
        db.close().await;
        std::fs::remove_dir_all(directory)?;
        Ok(())
    }
}
