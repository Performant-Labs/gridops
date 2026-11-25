//! One hour of the fleet's jobs and machines, as the hourly rollups store it
//! (#191, #200), and the window's hours as the page reads them (#204).
//!
//! The rollups (`fleet_health_rollup.rs`, `fleet_host_rollup.rs` in the
//! reconciler) write a finished hour from [`job_hour`] and [`host_hours`];
//! the database source and the score snapshots read a window through
//! [`window_rows`]: the hours past their rollup grace from the hourly tables,
//! and every later hour of the window (the hour in progress, and the hour
//! that just ended until `hour_end + ROLLUP_GRACE_MILLIS`) computed live by
//! the same two functions, so a live hour has exactly a rolled hour's shape.
//! One copy of the SQL, whoever reads it.
//!
//! Every read is bounded by an indexed time column (#179): the
//! `completed_at` indexes (`workflow_jobs_completed_idx`,
//! `gitlab_jobs_completed_idx`), `host_samples_host_time_idx`,
//! `fleet_events_host_kind_time_idx`, `host_commands_host_kind_issued_idx`,
//! `workload_placements_state_idx`, and the hourly tables' primary keys.

use std::collections::BTreeMap;

use anyhow::{Context as _, Result};
use serde_json::Value;
use sqlx::{FromRow, SqlitePool};

use super::{
    HISTOGRAM_BUCKETS, HOUR_MILLIS, HealthWindow, Histogram, HourlyHostRow, HourlyJobRow,
    JobCategory, ROLLUP_GRACE_MILLIS, RollupHost, UNMANAGED, histogram_bucket,
};
use crate::fleet::{
    PlacementFailureReason, failure::PLACEMENT_FAILED_EVENT, start_health::is_start_failure,
};

const MINUTE: i64 = 60_000;

/// An hour's GitHub jobs (`workflow_jobs_completed_idx`), final attempts
/// only: a job counts unless a later attempt of its run ran a job of the
/// same name. "Later" is a higher `workflow_job_sync.run_attempt`; when
/// either attempt is unknown (GitHub didn't give one), the higher job id,
/// since GitHub numbers a re-run's jobs after the attempt it re-runs, but
/// only once the run row knows a re-run (`run_attempt` above 1): two
/// same-named jobs of a run never re-run are both counted. A
/// "re-run failed jobs" attempt runs only the failed jobs, and the run's
/// other jobs keep their earlier attempt, so they still count. The runner
/// and its newest `workload_placements` row give the machine, pool and CPU
/// size; no runner row, or no placement, is unmanaged.
pub const GITHUB_JOBS: &str = r#"
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
   AND NOT EXISTS (SELECT 1 FROM workflow_jobs later_job
          LEFT JOIN workflow_job_sync later ON later.job_id=later_job.id
         WHERE later_job.run_id=wj.run_id AND later_job.name=wj.name
           AND later_job.id<>wj.id
           AND (later.run_attempt>sync.run_attempt
                OR ((later.run_attempt IS NULL OR sync.run_attempt IS NULL)
                    AND later_job.id>wj.id
                    AND (SELECT run.run_attempt FROM workflow_runs run
                          WHERE run.id=wj.run_id)>1)))"#;

/// An hour's GitLab jobs (`gitlab_jobs_completed_idx`), joined to their
/// machines through `runners.gitlab_runner_id` the same way. A GitLab job is
/// its own only attempt; `status='success'` is its completion.
pub const GITLAB_JOBS: &str = r#"
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

/// One `fleet_health_hourly` row being summed: its counts, with both
/// histograms.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JobTotals {
    pub job_count: i64,
    pub completed_count: i64,
    pub infra_failure_count: i64,
    pub duration_ms_sum: i64,
    /// CPU-millis × duration-millis, exact while summing; divided by a
    /// million when written ([`Self::cpu_seconds`]), so `cpu_seconds_sum`
    /// loses nothing to rounding.
    pub cpu_milli_seconds: i64,
    pub wait: Histogram,
    pub duration: Histogram,
}

impl Default for JobTotals {
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

impl JobTotals {
    /// `cpu_seconds_sum`: CPU-seconds reserved.
    #[must_use]
    pub fn cpu_seconds(&self) -> f64 {
        self.cpu_milli_seconds as f64 / 1_000_000.0
    }
}

/// One row of `fleet_health_hourly`: (host, pool, category).
pub type JobRowKey = (String, String, JobCategory);

/// The jobs completed in `[from, to)`, folded into `fleet_health_hourly`'s
/// rows: GitHub and GitLab, bounded by their `completed_at` indexes. The
/// rollup passes a whole hour; the source passes the hour in progress up to
/// now.
///
/// # Errors
/// A read failed.
pub async fn job_hour(
    db: &SqlitePool,
    from: i64,
    to: i64,
) -> Result<BTreeMap<JobRowKey, JobTotals>> {
    let mut facts = sqlx::query_as::<_, JobFact>(GITHUB_JOBS)
        .bind(from)
        .bind(to)
        .fetch_all(db)
        .await?;
    facts.extend(
        sqlx::query_as::<_, JobFact>(GITLAB_JOBS)
            .bind(from)
            .bind(to)
            .fetch_all(db)
            .await?,
    );
    let mut rows = BTreeMap::<JobRowKey, JobTotals>::new();
    for fact in &facts {
        add(&mut rows, fact);
    }
    Ok(rows)
}

/// Folds one job into the hour's rows: its machine from the placement (none
/// is `unmanaged`, with no CPU-seconds), its category (NULL as `other`), its
/// clamped duration and its two histogram buckets. A negative wait (clock
/// skew: `created_at` is when `GridOps` first saw the job) is no wait, the
/// bucket `histogram_bucket` gives a negative value; the duration is clamped
/// because its sum is stored non-negative.
fn add(rows: &mut BTreeMap<JobRowKey, JobTotals>, fact: &JobFact) {
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

/// The engine's rows of one hour's [`job_hour`].
#[must_use]
pub fn job_rows(hour: i64, totals: &BTreeMap<JobRowKey, JobTotals>) -> Vec<HourlyJobRow> {
    totals
        .iter()
        .map(|((host, pool, category), totals)| HourlyJobRow {
            hour,
            host: RollupHost::from_column(host),
            pool_id: pool.clone(),
            category: *category,
            job_count: totals.job_count.max(0).unsigned_abs(),
            completed_count: totals.completed_count.max(0).unsigned_abs(),
            infra_failure_count: totals.infra_failure_count.max(0).unsigned_abs(),
            duration_ms_sum: totals.duration_ms_sum,
            cpu_seconds_sum: totals.cpu_seconds(),
            wait_histogram: totals.wait,
            duration_histogram: totals.duration,
        })
        .collect()
}

/// One sample row: when it was observed, and the CPU, memory and disk free
/// it reported (`None` when the machine did not say).
type SampleRow = (i64, Option<i64>, Option<i64>, Option<i64>);

/// One machine's hour, as `fleet_host_hourly` stores it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostHour {
    pub host_id: String,
    pub online_minutes: i64,
    pub cpu_used_p50_millis: Option<i64>,
    pub cpu_used_p95_millis: Option<i64>,
    pub memory_used_p95_mib: Option<i64>,
    pub min_disk_free_bytes: Option<i64>,
    pub start_attempts: i64,
    /// Machine-fault start failures by `PlacementFailureReason` code.
    pub start_failures: BTreeMap<String, i64>,
    pub uncertain_placements: i64,
}

impl HostHour {
    /// The engine's row of this machine's hour starting at `hour`.
    ///
    /// # Errors
    /// A count or a measurement is negative or too large for the row.
    pub fn to_row(&self, hour: i64) -> Result<HourlyHostRow> {
        let narrow = |value: Option<i64>| -> Result<Option<u32>> {
            value
                .map(u32::try_from)
                .transpose()
                .context("a measurement out of range")
        };
        Ok(HourlyHostRow {
            hour,
            host_id: self.host_id.clone(),
            online_minutes: u32::try_from(self.online_minutes)?,
            cpu_used_p50_millis: narrow(self.cpu_used_p50_millis)?,
            cpu_used_p95_millis: narrow(self.cpu_used_p95_millis)?,
            memory_used_p95_mib: narrow(self.memory_used_p95_mib)?,
            min_disk_free_bytes: self.min_disk_free_bytes.map(u64::try_from).transpose()?,
            start_attempts: u32::try_from(self.start_attempts)?,
            start_failures: self
                .start_failures
                .iter()
                .map(|(reason, count)| Ok((reason.clone(), u32::try_from(*count)?)))
                .collect::<Result<_>>()?,
            uncertain_placements: u32::try_from(self.uncertain_placements)?,
        })
    }
}

/// The nearest-rank percentile `percent` (50, 95, …) of ascending `values`:
/// the value at rank ⌈percent × n / 100⌉. `None` without values.
#[must_use]
pub fn nearest_rank(sorted: &[i64], percent: i64) -> Option<i64> {
    let count = i64::try_from(sorted.len()).ok()?;
    if count == 0 {
        return None;
    }
    let rank = (percent.saturating_mul(count).saturating_add(99) / 100).clamp(1, count);
    sorted.get(usize::try_from(rank - 1).ok()?).copied()
}

/// Every machine's hour `[from, to)` (`from` on a whole hour, `to` at most
/// an hour later): one per `fleet_hosts` row in id order, also for a machine
/// with no samples (0 minutes, no percentiles).
///
/// # Errors
/// A read failed.
pub async fn host_hours(db: &SqlitePool, from: i64, to: i64) -> Result<Vec<HostHour>> {
    let hosts: Vec<String> = sqlx::query_scalar("SELECT id FROM fleet_hosts ORDER BY id")
        .fetch_all(db)
        .await?;
    let uncertain = uncertain_counts(db, from, to).await?;
    let mut hours = Vec::with_capacity(hosts.len());
    for host_id in &hosts {
        let uncertain_placements = uncertain.get(host_id).copied().unwrap_or(0);
        hours.push(read_host_hour(db, host_id, from, to, uncertain_placements).await?);
    }
    Ok(hours)
}

/// Reads one machine's hour `[from, to)`. Every read is bounded by an
/// indexed time column (#179) and happens outside any write transaction;
/// `uncertain_placements` was counted for the whole hour in one query
/// ([`uncertain_counts`]).
async fn read_host_hour(
    db: &SqlitePool,
    host_id: &str,
    from: i64,
    to: i64,
    uncertain_placements: i64,
) -> Result<HostHour> {
    // One bounded read per machine per hour on `host_samples_host_time_idx`
    // (host_id, observed_at): the online minutes, the CPU and memory series
    // for their percentiles, and the disk's least free.
    let samples: Vec<SampleRow> = sqlx::query_as(
        "SELECT observed_at,cpu_used_millis,memory_used_mib,disk_free_bytes FROM host_samples
          WHERE host_id=? AND observed_at>=? AND observed_at<?",
    )
    .bind(host_id)
    .bind(from)
    .bind(to)
    .fetch_all(db)
    .await?;
    let mut minutes = 0_u64;
    let mut cpu_used = Vec::new();
    let mut memory_used = Vec::new();
    let mut min_disk_free: Option<i64> = None;
    for (observed_at, cpu_used_millis, memory_used_mib, disk_free_bytes) in samples {
        // The query keeps `observed_at` inside the hour, so this is a minute
        // of it; the clamp only keeps a stray row from shifting past the mask.
        let minute = ((observed_at - from) / MINUTE).clamp(0, 59);
        minutes |= 1_u64 << u32::try_from(minute).unwrap_or(0);
        if let Some(cpu_used_millis) = cpu_used_millis {
            cpu_used.push(cpu_used_millis);
        }
        if let Some(memory_used_mib) = memory_used_mib {
            memory_used.push(memory_used_mib);
        }
        if let Some(disk_free_bytes) = disk_free_bytes {
            min_disk_free =
                Some(min_disk_free.map_or(disk_free_bytes, |least| least.min(disk_free_bytes)));
        }
    }
    cpu_used.sort_unstable();
    memory_used.sort_unstable();

    // The machine's placements that reached `starting` in the hour: a start
    // is issued exactly then, on `host_commands_host_kind_issued_idx`.
    let start_attempts: i64 = sqlx::query_scalar(
        "SELECT COUNT(DISTINCT placement_id) FROM host_commands
          WHERE host_id=? AND kind='start_runner' AND issued_at>=? AND issued_at<?",
    )
    .bind(host_id)
    .bind(from)
    .bind(to)
    .fetch_one(db)
    .await?;

    // The hour's `placement_failed` events, read as `start_gate` reads them,
    // counted only when the machine was at fault, by reason. One bounded read
    // on `fleet_events_host_kind_time_idx`.
    let details: Vec<String> = sqlx::query_scalar(
        "SELECT detail_json FROM fleet_events
          WHERE host_id=? AND kind=? AND created_at>=? AND created_at<?",
    )
    .bind(host_id)
    .bind(PLACEMENT_FAILED_EVENT)
    .bind(from)
    .bind(to)
    .fetch_all(db)
    .await?;
    let mut start_failures = BTreeMap::new();
    for detail_json in details {
        let detail: Value = serde_json::from_str(&detail_json).unwrap_or_default();
        let reason = PlacementFailureReason::from_code(
            detail.get("reason").and_then(Value::as_str).unwrap_or(""),
        );
        if is_start_failure(reason, detail.get("agent_code").and_then(Value::as_str)) {
            *start_failures
                .entry(reason.as_str().to_owned())
                .or_insert(0) += 1;
        }
    }

    Ok(HostHour {
        host_id: host_id.to_owned(),
        online_minutes: i64::from(minutes.count_ones()),
        cpu_used_p50_millis: nearest_rank(&cpu_used, 50),
        cpu_used_p95_millis: nearest_rank(&cpu_used, 95),
        memory_used_p95_mib: nearest_rank(&memory_used, 95),
        min_disk_free_bytes: min_disk_free,
        start_attempts,
        start_failures,
        uncertain_placements,
    })
}

/// The placements that ended `uncertain` in `[from, to)`, per machine. One
/// bounded read on `workload_placements_state_idx` (`state`, `updated_at`)
/// for the whole hour, instead of one unbounded read per machine.
async fn uncertain_counts(db: &SqlitePool, from: i64, to: i64) -> Result<BTreeMap<String, i64>> {
    let counts: Vec<(String, i64)> = sqlx::query_as(
        "SELECT host_id,COUNT(*) FROM workload_placements
          WHERE state='uncertain' AND updated_at>=? AND updated_at<? GROUP BY host_id",
    )
    .bind(from)
    .bind(to)
    .fetch_all(db)
    .await?;
    Ok(counts.into_iter().collect())
}

/// The whole hour `ms` falls in starts at.
#[must_use]
pub const fn floor_hour(ms: i64) -> i64 {
    ms - ms.rem_euclid(HOUR_MILLIS)
}

/// The first whole hour at or after `ms`.
#[must_use]
pub const fn ceil_hour(ms: i64) -> i64 {
    let remainder = ms.rem_euclid(HOUR_MILLIS);
    if remainder == 0 {
        ms
    } else {
        ms + HOUR_MILLIS - remainder
    }
}

/// Whether the rollups have an hour's rows by now: it is past its grace
/// (`hour + 1 h + ROLLUP_GRACE_MILLIS <= now`). The one test the source
/// uses; it never reads the rollups' markers.
#[must_use]
pub const fn is_past_grace(hour: i64, now: i64) -> bool {
    hour + HOUR_MILLIS + ROLLUP_GRACE_MILLIS <= now
}

/// The hours of `window` at `now` the source computes live: every hour of
/// `[ceil_hour(now − W), now)` not yet past its grace, oldest first (the
/// hour that just ended until its grace passes, and the hour in progress).
#[must_use]
pub fn live_hours(window: HealthWindow, now: i64) -> Vec<i64> {
    let first = ceil_hour(now - window.millis());
    let mut hour = floor_hour(now - HOUR_MILLIS - ROLLUP_GRACE_MILLIS).max(first);
    let mut hours = Vec::new();
    while hour < now {
        if !is_past_grace(hour, now) {
            hours.push(hour);
        }
        hour += HOUR_MILLIS;
    }
    hours
}

/// A `fleet_health_hourly` row as stored.
#[derive(Debug, FromRow)]
struct StoredJobRow {
    hour: i64,
    host_id: String,
    pool_id: String,
    category: String,
    job_count: i64,
    completed_count: i64,
    infra_failure_count: i64,
    duration_ms_sum: i64,
    cpu_seconds_sum: f64,
    queue_wait_histogram: String,
    duration_histogram: String,
}

/// A `fleet_host_hourly` row as stored.
#[derive(Debug, FromRow)]
struct StoredHostRow {
    hour: i64,
    host_id: String,
    online_minutes: i64,
    cpu_used_p50_millis: Option<i64>,
    cpu_used_p95_millis: Option<i64>,
    memory_used_p95_mib: Option<i64>,
    min_disk_free_bytes: Option<i64>,
    start_attempts: i64,
    start_failures_json: String,
    uncertain_placements: i64,
}

fn histogram(stored: &str) -> Result<Histogram> {
    serde_json::from_str(stored)
        .with_context(|| format!("a histogram that is not 9 counts: {stored}"))
}

fn count(value: i64) -> Result<u64> {
    u64::try_from(value).context("a negative count in fleet_health_hourly")
}

/// Every hour of `window` at `now`, as the scoring engine reads it: the
/// rows of the hours past their grace from `fleet_health_hourly` and
/// `fleet_host_hourly` (one read each on their primary keys), and the rows
/// of [`live_hours`] computed by [`job_hour`] and [`host_hours`] up to `now`.
/// A past-grace hour the rollups haven't written yet has no rows (the pass
/// lag): covered time with nothing in it.
///
/// # Errors
/// A read failed, or a stored row does not fit the engine's types.
pub async fn window_rows(
    db: &SqlitePool,
    window: HealthWindow,
    now: i64,
) -> Result<(Vec<HourlyJobRow>, Vec<HourlyHostRow>)> {
    let first = ceil_hour(now - window.millis());
    let last_rolled = now - HOUR_MILLIS - ROLLUP_GRACE_MILLIS;
    let mut jobs = Vec::new();
    for row in sqlx::query_as::<_, StoredJobRow>(
        "SELECT hour,host_id,pool_id,category,job_count,completed_count,infra_failure_count,
                duration_ms_sum,cpu_seconds_sum,queue_wait_histogram,duration_histogram
           FROM fleet_health_hourly WHERE hour>=? AND hour<=? ORDER BY hour,host_id,pool_id,category",
    )
    .bind(first)
    .bind(last_rolled)
    .fetch_all(db)
    .await?
    {
        jobs.push(HourlyJobRow {
            hour: row.hour,
            host: RollupHost::from_column(&row.host_id),
            pool_id: row.pool_id,
            category: JobCategory::from_column(Some(&row.category)),
            job_count: count(row.job_count)?,
            completed_count: count(row.completed_count)?,
            infra_failure_count: count(row.infra_failure_count)?,
            duration_ms_sum: row.duration_ms_sum,
            cpu_seconds_sum: row.cpu_seconds_sum,
            wait_histogram: histogram(&row.queue_wait_histogram)?,
            duration_histogram: histogram(&row.duration_histogram)?,
        });
    }
    let mut hosts = Vec::new();
    for row in sqlx::query_as::<_, StoredHostRow>(
        "SELECT hour,host_id,online_minutes,cpu_used_p50_millis,cpu_used_p95_millis,
                memory_used_p95_mib,min_disk_free_bytes,start_attempts,start_failures_json,
                uncertain_placements
           FROM fleet_host_hourly WHERE hour>=? AND hour<=? ORDER BY hour,host_id",
    )
    .bind(first)
    .bind(last_rolled)
    .fetch_all(db)
    .await?
    {
        let start_failures: BTreeMap<String, i64> = serde_json::from_str(&row.start_failures_json)
            .with_context(|| format!("start_failures_json: {}", row.start_failures_json))?;
        hosts.push(
            HostHour {
                host_id: row.host_id,
                online_minutes: row.online_minutes,
                cpu_used_p50_millis: row.cpu_used_p50_millis,
                cpu_used_p95_millis: row.cpu_used_p95_millis,
                memory_used_p95_mib: row.memory_used_p95_mib,
                min_disk_free_bytes: row.min_disk_free_bytes,
                start_attempts: row.start_attempts,
                start_failures,
                uncertain_placements: row.uncertain_placements,
            }
            .to_row(row.hour)?,
        );
    }
    for hour in live_hours(window, now) {
        let to = (hour + HOUR_MILLIS).min(now);
        jobs.extend(job_rows(hour, &job_hour(db, hour, to).await?));
        for host in host_hours(db, hour, to).await? {
            hosts.push(host.to_row(hour)?);
        }
    }
    Ok((jobs, hosts))
}

#[cfg(test)]
mod tests {
    use super::*;

    const HOUR: i64 = HOUR_MILLIS;
    /// A whole hour.
    const NOW: i64 = 1_800_000_000_000;

    #[test]
    fn the_live_hours_are_the_ones_not_past_their_grace() {
        // On the hour: the hour that just ended is in its grace; nothing is
        // in progress yet.
        assert_eq!(live_hours(HealthWindow::Day, NOW), [NOW - HOUR]);
        // 10 minutes in: the hour that ended 10 minutes ago, and this one.
        assert_eq!(
            live_hours(HealthWindow::Day, NOW + 10 * MINUTE),
            [NOW - HOUR, NOW]
        );
        // 15 minutes in: the ended hour is past its grace, the rollups have it.
        assert_eq!(live_hours(HealthWindow::Day, NOW + 15 * MINUTE), [NOW]);
        // A 1 h window 10 minutes in covers only the hour in progress.
        assert_eq!(live_hours(HealthWindow::Hour, NOW + 10 * MINUTE), [NOW]);
        // On the hour the 1 h window's one hour is the one in its grace.
        assert_eq!(live_hours(HealthWindow::Hour, NOW), [NOW - HOUR]);
    }

    #[test]
    fn hours_floor_and_ceil_and_pass_their_grace_at_15_minutes() {
        assert_eq!(floor_hour(NOW + MINUTE), NOW);
        assert_eq!(ceil_hour(NOW + MINUTE), NOW + HOUR);
        assert_eq!(ceil_hour(NOW), NOW);
        assert!(!is_past_grace(NOW - HOUR, NOW + 15 * MINUTE - 1));
        assert!(is_past_grace(NOW - HOUR, NOW + 15 * MINUTE));
    }
}
