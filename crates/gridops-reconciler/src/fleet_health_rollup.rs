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
//!
//! The reads of an hour run outside the write transaction, bounded by the
//! `completed_at` indexes (`workflow_jobs_completed_idx`,
//! `gitlab_jobs_completed_idx`); the hour's rows are then written in one
//! short transaction (at most [`WRITE_BATCH`] rows per transaction), which
//! also takes the hour off `fleet_health_recompute`, so re-running an hour
//! gives the same rows. The catch-up walks the last 7 days one finished hour
//! per pass, oldest first; where it got to is the `fleet_health.rollup_next`
//! setting, absent until the first pass.

use std::collections::BTreeMap;

use anyhow::{Context as _, Result};
use gridops_core::fleet_health::{
    HISTOGRAM_BUCKETS, HOUR_MILLIS, HealthWindow, Histogram, JobCategory, UNMANAGED,
    histogram_bucket,
};
use sqlx::{FromRow, SqlitePool};

/// The settings key holding the next hour the catch-up rolls up.
const ROLLUP_NEXT_SETTING: &str = "fleet_health.rollup_next";

/// How many `fleet_health_hourly` rows one write transaction writes at most
/// (#179's SQLite discipline).
const WRITE_BATCH: usize = 500;

/// The hour's GitHub jobs (`workflow_jobs_completed_idx`), final attempts
/// only: a job counts when its attempt (`workflow_job_sync.run_attempt`,
/// unknown counts as 1) is the newest one known for its run, exactly the
/// attempt `gridops_core::finish_earlier_attempts` finishes earlier jobs
/// against. The runner and its newest `workload_placements` row give the
/// machine, pool and CPU size; no runner row, or no placement, is unmanaged.
const GITHUB_JOBS: &str = r#"
SELECT wj.category,
       COALESCE(wj.conclusion='success',0) AS success,
       COALESCE(p.state IN ('failed','uncertain'),0) AS infra_failure,
       p.host_id,intent.pool_id,p.cpu_millis,
       wj.created_at,wj.started_at,wj.completed_at
  FROM workflow_jobs wj
  LEFT JOIN workflow_job_sync sync ON sync.job_id=wj.id
  LEFT JOIN runners r ON r.id=(SELECT cand.id FROM runners cand
          WHERE cand.github_runner_id=wj.runner_id
          ORDER BY cand.created_at DESC,cand.id DESC LIMIT 1)
  LEFT JOIN workload_placements p ON p.id=(
         SELECT latest.id FROM workload_placements latest
          WHERE latest.runner_id=r.id
          ORDER BY latest.generation DESC,latest.updated_at DESC,latest.id DESC LIMIT 1)
  LEFT JOIN workload_intents intent ON intent.id=p.intent_id
 WHERE wj.completed_at>=? AND wj.completed_at<?
   AND COALESCE(sync.run_attempt,1)=MAX(
         COALESCE((SELECT run.run_attempt FROM workflow_runs run WHERE run.id=wj.run_id),1),
         COALESCE((SELECT MAX(later.run_attempt) FROM workflow_job_sync later
                     JOIN workflow_jobs later_job ON later_job.id=later.job_id
                    WHERE later_job.run_id=wj.run_id),1))"#;

/// The hour's GitLab jobs (`gitlab_jobs_completed_idx`), joined to their
/// machines through `runners.gitlab_runner_id` the same way. A GitLab job is
/// its own only attempt; `status='success'` is its completion.
const GITLAB_JOBS: &str = r#"
SELECT gj.category,
       COALESCE(gj.status='success',0) AS success,
       COALESCE(p.state IN ('failed','uncertain'),0) AS infra_failure,
       p.host_id,intent.pool_id,p.cpu_millis,
       gj.created_at,gj.started_at,gj.completed_at
  FROM gitlab_jobs gj
  LEFT JOIN runners r ON r.id=(SELECT cand.id FROM runners cand
          WHERE cand.gitlab_runner_id=gj.gitlab_runner_id
          ORDER BY cand.created_at DESC,cand.id DESC LIMIT 1)
  LEFT JOIN workload_placements p ON p.id=(
         SELECT latest.id FROM workload_placements latest
          WHERE latest.runner_id=r.id
          ORDER BY latest.generation DESC,latest.updated_at DESC,latest.id DESC LIMIT 1)
  LEFT JOIN workload_intents intent ON intent.id=p.intent_id
 WHERE gj.completed_at>=? AND gj.completed_at<?"#;

/// One counted job, as one of the hour's reads sees it.
#[derive(Debug, FromRow)]
struct JobFact {
    category: Option<String>,
    /// GitHub conclusion `success`, GitLab status `success`.
    success: bool,
    /// The job's placement ended `failed` or `uncertain`.
    infra_failure: bool,
    /// The placement's machine and pool; no placement is unmanaged.
    host_id: Option<String>,
    pool_id: Option<String>,
    cpu_millis: Option<i64>,
    created_at: i64,
    started_at: Option<i64>,
    completed_at: i64,
}

/// One `fleet_health_hourly` row being summed: (host, pool, category) and
/// its counts, with both histograms.
#[derive(Debug)]
struct HourTotals {
    job_count: i64,
    completed_count: i64,
    infra_failure_count: i64,
    duration_ms_sum: i64,
    /// CPU-millis × duration-millis, exact while summing; divided by a
    /// million when written, so `cpu_seconds_sum` loses nothing to rounding.
    cpu_milli_seconds: i64,
    wait: Histogram,
    duration: Histogram,
}

impl Default for HourTotals {
    fn default() -> Self {
        Self {
            job_count: 0,
            completed_count: 0,
            infra_failure_count: 0,
            duration_ms_sum: 0,
            cpu_milli_seconds: 0,
            wait: [0; HISTOGRAM_BUCKETS],
            duration: [0; HISTOGRAM_BUCKETS],
        }
    }
}

/// One row of `fleet_health_hourly`: (host, pool, category).
type RowKey = (String, String, JobCategory);

/// Writes the rows of the hour starting at `hour` (epoch ms, on a whole
/// hour), replacing what is there, and takes the hour off
/// `fleet_health_recompute`.
///
/// # Errors
/// A database read or write failed.
pub(crate) async fn rollup_hour(db: &SqlitePool, hour: i64, now: i64) -> Result<()> {
    let hour = hour - hour.rem_euclid(HOUR_MILLIS);
    let facts = hour_facts(db, hour).await?;
    let mut rows = BTreeMap::<RowKey, HourTotals>::new();
    for fact in &facts {
        add(&mut rows, fact);
    }
    write_hour(db, hour, now, &rows.into_iter().collect::<Vec<_>>()).await
}

/// One reconciler pass of the rollup: every hour listed in
/// `fleet_health_recompute` that has finished, oldest first, then at most
/// one catch-up hour of the last 7 days.
///
/// # Errors
/// A database read or write failed; `reconcile()` logs it and goes on.
pub(crate) async fn run_pass(db: &SqlitePool, now: i64) -> Result<()> {
    let listed: Vec<i64> =
        sqlx::query_scalar("SELECT hour FROM fleet_health_recompute WHERE hour<=? ORDER BY hour")
            .bind(now - HOUR_MILLIS)
            .fetch_all(db)
            .await?;
    for hour in listed {
        rollup_hour(db, hour, now).await?;
    }

    // Catch-up: one finished hour per pass, oldest first, over the last 7
    // days. The first pass starts at the window's oldest hour; one whose
    // marker the sliding window has passed starts again at its oldest.
    let window_start = now - HealthWindow::Week.millis();
    let window_start = window_start - window_start.rem_euclid(HOUR_MILLIS);
    let next = match next_hour(db).await? {
        Some(next) => next.max(window_start),
        None => window_start,
    };
    if next + HOUR_MILLIS <= now {
        rollup_hour(db, next, now).await?;
        save_next_hour(db, next + HOUR_MILLIS, now).await?;
    }
    Ok(())
}

/// The hour's jobs: GitHub and GitLab, bounded by their `completed_at`
/// indexes.
///
/// # Errors
/// A read failed.
async fn hour_facts(db: &SqlitePool, hour: i64) -> Result<Vec<JobFact>> {
    let end = hour + HOUR_MILLIS;
    let mut facts = sqlx::query_as::<_, JobFact>(GITHUB_JOBS)
        .bind(hour)
        .bind(end)
        .fetch_all(db)
        .await?;
    facts.extend(
        sqlx::query_as::<_, JobFact>(GITLAB_JOBS)
            .bind(hour)
            .bind(end)
            .fetch_all(db)
            .await?,
    );
    Ok(facts)
}

/// Folds one job into the hour's rows: its machine from the placement (none
/// is `unmanaged`, with no CPU-seconds), its category (NULL as `other`), its
/// clamped duration and its two histogram buckets. A negative wait (clock
/// skew: `created_at` is when `GridOps` first saw the job) is no wait, the
/// bucket `histogram_bucket` gives a negative value; the duration is clamped
/// because its sum is stored non-negative.
fn add(rows: &mut BTreeMap<RowKey, HourTotals>, fact: &JobFact) {
    let category = JobCategory::from_column(fact.category.as_deref());
    let (host, pool, cpu_millis) = match (fact.host_id.as_ref(), fact.pool_id.as_ref()) {
        (Some(host), Some(pool)) => (host.clone(), pool.clone(), fact.cpu_millis),
        _ => (UNMANAGED.to_owned(), UNMANAGED.to_owned(), None),
    };
    let started = fact.started_at.unwrap_or(fact.completed_at);
    let duration = (fact.completed_at - started).max(0);
    let totals = rows.entry((host, pool, category)).or_default();
    totals.job_count += 1;
    totals.completed_count += i64::from(fact.success);
    totals.infra_failure_count += i64::from(fact.infra_failure);
    totals.duration_ms_sum += duration;
    totals.cpu_milli_seconds += duration * cpu_millis.unwrap_or(0);
    totals.wait[histogram_bucket(started - fact.created_at)] += 1;
    totals.duration[histogram_bucket(duration)] += 1;
}

/// Writes the hour's rows, replacing what is there and taking the hour off
/// `fleet_health_recompute`: one short transaction per [`WRITE_BATCH`] rows,
/// the first emptying the hour, the last also emptying its recompute entry
/// (an hour with no jobs still gets both). An hour is one (host, pool,
/// category) group per job that ran, so it is far below the batch size and
/// one transaction writes it whole; the split only serves #179's rule that
/// no transaction writes more than 500 rows, should a fleet ever grow that
/// wide, and a failure between batches leaves the hour listed for a recompute
/// that rewrites it from scratch.
///
/// # Errors
/// A write failed.
async fn write_hour(
    db: &SqlitePool,
    hour: i64,
    now: i64,
    rows: &[(RowKey, HourTotals)],
) -> Result<()> {
    let batches = rows.chunks(WRITE_BATCH).collect::<Vec<_>>();
    let batches = if batches.is_empty() {
        vec![&[][..]]
    } else {
        batches
    };
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
            .bind(totals.cpu_milli_seconds as f64 / 1_000_000.0)
            .bind(serde_json::to_string(&totals.wait)?)
            .bind(serde_json::to_string(&totals.duration)?)
            .bind(now)
            .execute(&mut *tx)
            .await?;
        }
        if index == last {
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
