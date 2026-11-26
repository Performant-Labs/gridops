//! The live half of the fleet health response (#201): what is true of each
//! machine and of the queue right now, and the live areas of concern
//! (offline, start gate, disk, unmatched jobs, oldest queued, uncertain
//! placements).
//!
//! Everything a principal would gate is read through #197's fleet-wide reads
//! with none: `gridops_core::fleet::registry::host_live_reads` (per-machine
//! counts, root budget and allocation, latest root measurement, placement
//! failures, start gate; live placements only, #240),
//! `crate::resources::fleet_queue_breakdown` (the Overview's queue split over
//! every installation, with the queued jobs by category) and
//! `crate::resources::fleet_root_disks` (the Overview's disk alarm read: the
//! observed root disk of each machine with a recent heartbeat, #240). The
//! only queries here are the ones those can't answer, each a bounded index
//! read (#179): a machine's newest sample (per host, on `host_samples`'
//! `(host_id, observed_at, id)` index), the categories of the jobs its
//! runners are running now (per host, on `workload_placements`'
//! `(host_id, state)` index; a GitLab runner's job on the partial index of
//! active GitLab jobs), its start failures and starts in the last hour (per
//! host, on `fleet_events_host_kind_time_idx` and
//! `host_commands_host_kind_issued_idx`), the GitLab jobs waiting for a
//! runner (`gridops_core::autoscaling::waiting_gitlab_jobs`, on the partial index of
//! active GitLab jobs, #253), the oldest waiting job's name and repository
//! (by id), and the pool names (a table of a few rows).

use std::collections::{BTreeMap, HashMap};

use gridops_core::disk;
use gridops_core::fleet::failure::{PLACEMENT_FAILED_EVENT, PlacementFailureReason};
use gridops_core::fleet::registry::host_live_reads;
use gridops_core::fleet::start_health::is_start_failure;
use gridops_core::fleet_health::{
    Concern, ConcernSubject, FleetLiveStats, HostLiveStats, JobCategory, OldestQueued, PoolQueue,
    Severity,
};
use serde_json::Value;
use sqlx::{Row as _, SqlitePool};

/// A machine is online while it has a sample this recent.
const ONLINE_WINDOW_MILLIS: i64 = 5 * 60_000;
/// The oldest waiting job warns past this wait and is critical past twice it.
const QUEUED_WARN_MILLIS: i64 = 5 * 60_000;
const QUEUED_CRITICAL_MILLIS: i64 = 10 * 60_000;
/// How far back a machine's start failures and starts count.
const START_FAILURE_WINDOW_MILLIS: i64 = 3_600_000;
/// The `start-failures` concern needs at least this many start failures in
/// the hour, so one unlucky start on a quiet machine raises nothing.
const START_FAILURES_MIN: u32 = 3;
/// ... and at least this share of the hour's starts failing (warning), or
/// this one (critical), in percent.
const START_FAILURE_WARN_PERCENT: u32 = 25;
const START_FAILURE_CRITICAL_PERCENT: u32 = 50;

/// The fleet's and every machine's live stats at `now`, and the live
/// concerns.
///
/// # Errors
/// A database read failed.
#[allow(dead_code, reason = "#204's database source calls it")]
pub(crate) async fn collect_live(
    db: &SqlitePool,
    now: i64,
) -> anyhow::Result<(FleetLiveStats, Vec<HostLiveStats>, Vec<Concern>)> {
    let reads = host_live_reads(db, now).await?;
    let queue = crate::resources::fleet_queue_breakdown(db, now).await?;
    let gitlab_waiting = gridops_core::autoscaling::waiting_gitlab_jobs(db).await?;
    let disks: HashMap<String, crate::resources::FleetRootDisk> =
        crate::resources::fleet_root_disks(db, now)
            .await?
            .into_iter()
            .map(|disk| (disk.host_id.clone(), disk))
            .collect();
    let mut hosts = Vec::with_capacity(reads.len());
    let mut concerns = Vec::new();
    let mut fleet_running: BTreeMap<JobCategory, u32> = BTreeMap::new();

    for read in &reads {
        let id = read.id.to_string();
        let name = read.name.as_str();
        let machine = format!("/fleet/machines/{id}");

        // Online is a sample in the last five minutes; `last_seen_at` is the
        // newest sample however old it is. Both queries stay on
        // `host_samples`' `(host_id, observed_at, id)` index: the window one
        // reads a bounded range, and the second, only for a machine with no
        // sample in the window (so the offline concern can say when it was
        // last seen), seeks that index's last entry for the host.
        let recent: Option<i64> = sqlx::query_scalar(
            "SELECT MAX(observed_at) FROM host_samples WHERE host_id=? AND observed_at>=?",
        )
        .bind(&id)
        .bind(now - ONLINE_WINDOW_MILLIS)
        .fetch_optional(db)
        .await?
        .flatten();
        let last_seen_at = match recent {
            Some(at) => Some(at),
            None => sqlx::query_scalar("SELECT MAX(observed_at) FROM host_samples WHERE host_id=?")
                .bind(&id)
                .fetch_optional(db)
                .await?
                .flatten(),
        };

        // What its runners are running now, by category: each running CI
        // placement whose runner has a current job or is busy, once. Only
        // GitHub's `associate_runner_with_job` sets `current_job_id`; a
        // GitLab runner is only ever marked busy (`set_gitlab_runner_busy`:
        // `status='busy'`, `busy=1`), and so is a GitHub runner GitHub says
        // is running a job GridOps never matched. A GitHub runner's job is
        // its `workflow_jobs` row by `current_job_id`; a GitLab runner's is
        // the `gitlab_jobs` row running on it (#253): its `gitlab_runner_id`
        // on the runner's GitLab instance (runner ids are unique per
        // instance), status `running`, newest first, read on the partial
        // index of active jobs. Another platform's job ids are neither. No
        // job row, or a NULL category, is `other`. Runs on
        // `workload_placements`' `(host_id, state)` index; running fix
        // placements (a category no job row has) come from the counts.
        let running: Vec<(Option<String>, i64)> = sqlx::query_as(
            "SELECT COALESCE(wj.category,gj.category),COUNT(*) FROM workload_placements p
               JOIN workload_intents i ON i.id=p.intent_id
               JOIN runners r ON r.id=p.runner_id
               LEFT JOIN workflow_jobs wj
                 ON wj.id=r.current_job_id AND r.ci_platform='github'
               LEFT JOIN gitlab_jobs gj ON r.ci_platform='gitlab' AND gj.id=(
                    SELECT job.id FROM gitlab_jobs job
                      JOIN gitlab_connections job_connection ON job_connection.id=job.connection_id
                     WHERE job.status IN ('created','waiting_for_resource','preparing','pending','running')
                       AND job.status='running'
                       AND job.gitlab_runner_id=r.gitlab_runner_id
                       AND job_connection.base_url=(
                             SELECT runner_connection.base_url FROM gitlab_connections runner_connection
                              WHERE runner_connection.id=r.gitlab_connection_id)
                     ORDER BY job.status_changed_at DESC,job.id DESC LIMIT 1)
              WHERE p.host_id=? AND p.state='running' AND i.kind='ci_runner'
                AND (r.current_job_id IS NOT NULL OR (r.status='busy' AND r.busy=1))
              GROUP BY COALESCE(wj.category,gj.category)",
        )
        .bind(&id)
        .fetch_all(db)
        .await?;
        let mut running_by_category = BTreeMap::new();
        for (category, jobs) in running {
            *running_by_category
                .entry(JobCategory::from_column(category.as_deref()))
                .or_insert(0) += u32::try_from(jobs)?;
        }
        let fix_runs = u32::try_from(read.counts.running_fix_placement_count.get())?;
        if fix_runs > 0 {
            *running_by_category
                .entry(JobCategory::FixAgent)
                .or_insert(0) += fix_runs;
        }
        for (category, jobs) in &running_by_category {
            *fleet_running.entry(*category).or_insert(0) += *jobs;
        }

        // Its start failures in the last hour, as the start gate and the
        // machine rollup count them (`is_start_failure`): not every
        // `placement_failed` event, which `failed_placements_last_hour`
        // counts for `/fleet/hosts` (#253).
        let failures =
            start_failures_since(db, &id, now - START_FAILURE_WINDOW_MILLIS, now).await?;
        let start_failures = u32::try_from(failures.len())?;

        let measurement = read.latest_root_measurement.as_ref();
        let narrow = |value: Option<u64>| -> anyhow::Result<Option<u32>> {
            value.map(u32::try_from).transpose().map_err(Into::into)
        };
        hosts.push(HostLiveStats {
            host_id: id.clone(),
            online: recent.is_some(),
            last_seen_at,
            cpu_used_millis: narrow(
                measurement
                    .and_then(|row| row.cpu_used_millis)
                    .map(|value| value.get()),
            )?,
            memory_used_mib: narrow(
                measurement
                    .and_then(|row| row.memory_used_mib)
                    .map(|value| value.get()),
            )?,
            disk_free_bytes: measurement
                .and_then(|row| row.disk_free_bytes)
                .map(|value| value.get()),
            running_by_category,
            cpu_reserved_millis: u32::try_from(read.physical_allocation.charged.cpu_millis.get())?,
            memory_reserved_mib: u32::try_from(read.physical_allocation.charged.memory_mib.get())?,
            start_failures_last_hour: start_failures,
        });

        // `start-failures`: at least 3 start failures in the hour that are
        // at least a quarter of its starts (critical from half). A start is a
        // placement whose `start_runner` command was issued in the hour, as
        // the machine rollup counts `start_attempts`; a failure of a start
        // issued before the hour counts as one of them, so the share is never
        // over 100 %.
        if start_failures >= START_FAILURES_MIN {
            let starts: i64 = sqlx::query_scalar(
                "SELECT COUNT(DISTINCT placement_id) FROM host_commands
                  WHERE host_id=? AND kind='start_runner' AND issued_at>=? AND issued_at<=?",
            )
            .bind(&id)
            .bind(now - START_FAILURE_WINDOW_MILLIS)
            .bind(now)
            .fetch_one(db)
            .await?;
            let starts = u32::try_from(starts)?.max(start_failures);
            let percent = start_failures * 100 / starts;
            let severity = if start_failures * 100 >= START_FAILURE_CRITICAL_PERCENT * starts {
                Some(Severity::Critical)
            } else if start_failures * 100 >= START_FAILURE_WARN_PERCENT * starts {
                Some(Severity::Warning)
            } else {
                None
            };
            if let Some(severity) = severity {
                concerns.push(Concern {
                    key: format!("start-failures:{id}"),
                    severity,
                    title: format!(
                        "{name} failed {start_failures} of {starts} runner starts in the last hour"
                    ),
                    detail: format!(
                        "{percent}% of the runner starts on {name} in the last hour failed on the machine; an operator's release or a runner removed before it started does not count."
                    ),
                    subject: ConcernSubject::Host(id.clone()),
                    link: Some(format!("/fleet/health#machine-{id}")),
                    since: failures.iter().copied().min(),
                });
            }
        }

        // `host-offline`: a budgeted machine with no sample in five minutes.
        if read.physical_budget.cpu_millis.get() > 0 && recent.is_none() {
            let silence = last_seen_at.map_or_else(
                || format!("{name} has never sent a sample"),
                |at| {
                    format!(
                        "{name} has sent no sample for {}",
                        span_text(now.saturating_sub(at))
                    )
                },
            );
            concerns.push(Concern {
                key: format!("host-offline:{id}"),
                severity: Severity::Critical,
                title: format!("{name} is offline"),
                detail: format!("{silence}, so nothing can start there."),
                subject: ConcernSubject::Host(id.clone()),
                link: Some(machine.clone()),
                since: last_seen_at,
            });
        }

        // `start-gate`: the machine is set aside from new placements.
        if let Some(gate) = &read.start_gate {
            concerns.push(Concern {
                key: format!("start-gate:{id}"),
                severity: Severity::Warning,
                title: format!("{name}'s starts are paused"),
                detail: gate.message.clone(),
                subject: ConcernSubject::Host(id.clone()),
                link: Some(machine.clone()),
                since: Some(gate.since.millis()),
            });
        }

        // `disk`: the Overview's disk alarm, from the same read
        // (`fleet_root_disks`): the newest sample's free space against the
        // root disk's observed size (not its admin-set budget), and only for
        // a machine with a heartbeat in the last 120 s.
        if let Some((alarm, free, disk_total)) = disks.get(&id).and_then(|disk| {
            disk::alarm_for(disk.total_bytes, disk.free_bytes)
                .map(|alarm| (alarm, disk.free_bytes, disk.total_bytes))
        }) {
            let (severity, level) = match alarm {
                disk::DiskAlarm::Critical => (Severity::Critical, "critical"),
                disk::DiskAlarm::Warning => (Severity::Warning, "warning"),
            };
            concerns.push(Concern {
                key: format!("disk:{id}"),
                severity,
                title: format!("{name} is low on disk"),
                detail: format!(
                    "{name} has {} GiB free of {} GiB, under its {level} floor.",
                    gib_text(free),
                    gib_text(disk_total),
                ),
                subject: ConcernSubject::Host(id.clone()),
                link: Some(machine.clone()),
                since: None,
            });
        }

        // `uncertain`: the machine has placements in state `uncertain`.
        let uncertain = u32::try_from(read.counts.uncertain_placement_count.get())?;
        if uncertain > 0 {
            let plural = |one: &str, many: &str| {
                if uncertain == 1 {
                    one.to_owned()
                } else {
                    many.replace("{n}", &uncertain.to_string())
                }
            };
            concerns.push(Concern {
                key: format!("uncertain:{id}"),
                severity: Severity::Warning,
                title: format!(
                    "{name} has {}",
                    plural("an uncertain placement", "{n} uncertain placements")
                ),
                detail: format!(
                    "{} on {name} {} uncertain: GridOps can't tell whether {} runner {} running.",
                    plural("1 placement", "{n} placements"),
                    plural("is", "are"),
                    plural("its", "their"),
                    plural("is", "are"),
                ),
                subject: ConcernSubject::Host(id.clone()),
                link: Some(machine),
                since: None,
            });
        }
    }

    // The GitLab jobs waiting for a runner (#253): the Overview's split is
    // GitHub's, so they are added to it here, each under the pool the same
    // ownership rule gives it, or unmatched. The oldest one an autoscaling
    // pool will start a runner for is a candidate for the oldest waiting
    // job, as a GitHub job is.
    let mut gitlab_by_pool: HashMap<String, i64> = HashMap::new();
    let mut gitlab_by_category: BTreeMap<JobCategory, i64> = BTreeMap::new();
    let mut gitlab_unmatched = 0_i64;
    let mut gitlab_oldest: Option<&gridops_core::autoscaling::WaitingGitlabJob> = None;
    for job in &gitlab_waiting {
        *gitlab_by_category
            .entry(JobCategory::from_column(job.category.as_deref()))
            .or_default() += 1;
        match &job.owner {
            Some(owner) => {
                *gitlab_by_pool.entry(owner.pool_id.clone()).or_default() += 1;
                if !owner.manual
                    && gitlab_oldest.is_none_or(|oldest| job.queued_at < oldest.queued_at)
                {
                    gitlab_oldest = Some(job);
                }
            }
            None => gitlab_unmatched += 1,
        }
    }
    let gitlab_oldest_wait = gitlab_oldest.map(|job| now.saturating_sub(job.queued_at));

    // The oldest job waiting for capacity, with its name and repository for
    // the concern's sentence (a lookup by id; the breakdown only counts). The
    // breakdown's job clause already requires the run and repository rows, so
    // the lookup only misses if the job left the queue between the two reads;
    // the concern below still fires then, unnamed. A GitLab job that has
    // waited longer is the oldest instead, named by its job name and project
    // path (or its id, for a row written before they were recorded).
    let mut oldest_queued = None;
    let mut oldest_unnamed = None;
    let github_oldest = queue
        .queue
        .oldest_waiting_job
        .zip(queue.queue.oldest_waiting_millis);
    if let Some(job) =
        gitlab_oldest.filter(|_| gitlab_oldest_wait > github_oldest.map(|(_, wait_ms)| wait_ms))
    {
        oldest_queued = Some(OldestQueued {
            job_id: job.job_id,
            job_name: job
                .job_name
                .clone()
                .unwrap_or_else(|| format!("GitLab job {}", job.job_id)),
            repository: job
                .project_path
                .clone()
                .unwrap_or_else(|| job.base_url.clone()),
            queued_at: job.queued_at,
            wait_ms: now.saturating_sub(job.queued_at),
        });
    } else if let Some((job_id, wait_ms)) = github_oldest {
        let job = sqlx::query(
            "SELECT wj.name AS name,wj.created_at AS queued_at,repo.full_name AS repository
               FROM workflow_jobs wj
               JOIN workflow_runs wr ON wr.id=wj.run_id
               JOIN repositories repo ON repo.id=wr.repository_id
              WHERE wj.id=?",
        )
        .bind(job_id)
        .fetch_optional(db)
        .await?;
        match job {
            Some(row) => {
                oldest_queued = Some(OldestQueued {
                    job_id,
                    job_name: row.get("name"),
                    repository: row.get("repository"),
                    queued_at: row.get("queued_at"),
                    wait_ms,
                });
            }
            None => oldest_unnamed = Some((job_id, wait_ms)),
        }
    }

    // `unmatched-jobs`: queued jobs no pool can run, as the Overview splits
    // them, and the GitLab jobs no pool will run. Why a GitLab job has no
    // pool is not split out, so with any of them the sentence is the
    // general one.
    let unmatched = queue.queue.unmatched + gitlab_unmatched;
    if unmatched > 0 {
        let one = unmatched == 1;
        let job = if one { "job" } else { "jobs" };
        let asks = if one { "asks" } else { "ask" };
        let matches_ = if one { "matches" } else { "match" };
        let detail = if gitlab_unmatched == 0 && queue.queue.unavailable == 0 {
            format!(
                "{unmatched} queued {job} {asks} for labels no runner pool offers; {} will wait until one does.",
                if one { "it" } else { "they" }
            )
        } else if gitlab_unmatched == 0
            && queue
                .queue
                .missing_labels
                .iter()
                .map(|missing| missing.jobs)
                .sum::<i64>()
                == 0
        {
            format!(
                "{unmatched} queued {job} {asks} for a pool that is paused, tripped or on another platform; nothing will start for {}.",
                if one { "it" } else { "them" }
            )
        } else {
            format!(
                "{unmatched} queued {job} {matches_} no pool that can run {}; they will wait until one does.",
                if one { "it" } else { "them" }
            )
        };
        concerns.push(Concern {
            key: "unmatched-jobs".to_owned(),
            severity: Severity::Critical,
            title: format!("{unmatched} queued {job} {matches_} no pool"),
            detail,
            subject: ConcernSubject::Fleet,
            link: Some("/queued-jobs".to_owned()),
            since: None,
        });
    }

    // `oldest-queued`: the oldest waiting job's wait, over five minutes.
    let oldest_wait = oldest_queued
        .as_ref()
        .map(|job| job.wait_ms)
        .or(oldest_unnamed.map(|(_, wait_ms)| wait_ms));
    if let Some(wait_ms) = oldest_wait.filter(|wait| *wait > QUEUED_WARN_MILLIS) {
        let severity = if wait_ms > QUEUED_CRITICAL_MILLIS {
            Severity::Critical
        } else {
            Severity::Warning
        };
        let wait = span_text(wait_ms);
        let detail = match &oldest_queued {
            Some(job) => format!(
                "{} in {} has waited {wait} to start.",
                job.job_name, job.repository
            ),
            None => format!(
                "Job {} has waited {wait} to start.",
                oldest_unnamed.map_or(-1, |(job_id, _)| job_id)
            ),
        };
        concerns.push(Concern {
            key: "oldest-queued".to_owned(),
            severity,
            title: format!("A job has waited {wait}"),
            detail,
            subject: ConcernSubject::Fleet,
            link: Some("/queued-jobs".to_owned()),
            since: oldest_queued.as_ref().map(|job| job.queued_at),
        });
    }

    let names: HashMap<String, String> =
        sqlx::query_as::<_, (String, String)>("SELECT id,name FROM runner_pools")
            .fetch_all(db)
            .await?
            .into_iter()
            .collect();
    let mut by_pool = queue.queue.by_pool.clone();
    for (pool_id, jobs) in gitlab_by_pool {
        *by_pool.entry(pool_id).or_default() += jobs;
    }
    let mut queued_by_pool = by_pool
        .iter()
        .filter(|(_, jobs)| **jobs > 0)
        .map(|(pool_id, queued)| -> anyhow::Result<PoolQueue> {
            Ok(PoolQueue {
                pool_id: pool_id.clone(),
                pool_name: names
                    .get(pool_id)
                    .cloned()
                    .unwrap_or_else(|| pool_id.clone()),
                queued: u32::try_from(*queued)?,
            })
        })
        .collect::<anyhow::Result<Vec<_>>>()?;
    queued_by_pool.sort_by(|left, right| {
        right
            .queued
            .cmp(&left.queued)
            .then_with(|| left.pool_id.cmp(&right.pool_id))
    });

    let mut queued_by_category = queue.by_category.clone();
    for (category, jobs) in gitlab_by_category {
        *queued_by_category.entry(category).or_default() += jobs;
    }
    let fleet = FleetLiveStats {
        queued_total: u32::try_from(queue.queue.total())? + u32::try_from(gitlab_waiting.len())?,
        queued_by_category: queued_by_category
            .iter()
            .map(|(category, jobs)| Ok((*category, u32::try_from(*jobs)?)))
            .collect::<anyhow::Result<BTreeMap<_, _>>>()?,
        queued_by_pool,
        queued_unmatched: u32::try_from(unmatched)?,
        oldest_queued,
        running_total: fleet_running.values().sum(),
        running_by_category: fleet_running,
    };
    Ok((fleet, hosts, concerns))
}

/// The times of `host_id`'s start failures in `[from, to]`: its
/// `placement_failed` events that `start_health::is_start_failure` puts on
/// the machine, read and judged as `start_gate` and the machine rollup read
/// them. One bounded read on `fleet_events_host_kind_time_idx`.
async fn start_failures_since(
    db: &SqlitePool,
    host_id: &str,
    from: i64,
    to: i64,
) -> anyhow::Result<Vec<i64>> {
    let rows: Vec<(i64, String)> = sqlx::query_as(
        "SELECT created_at,detail_json FROM fleet_events
          WHERE host_id=? AND kind=? AND created_at>=? AND created_at<=?",
    )
    .bind(host_id)
    .bind(PLACEMENT_FAILED_EVENT)
    .bind(from)
    .bind(to)
    .fetch_all(db)
    .await?;
    Ok(rows
        .into_iter()
        .filter(|(_, detail_json)| {
            let detail: Value = serde_json::from_str(detail_json).unwrap_or_default();
            is_start_failure(
                PlacementFailureReason::from_code(
                    detail.get("reason").and_then(Value::as_str).unwrap_or(""),
                ),
                detail.get("agent_code").and_then(Value::as_str),
            )
        })
        .map(|(created_at, _)| created_at)
        .collect())
}

/// A wait as a person reads it, rounded down to its two largest units:
/// `42 s`, `7 min`, `10 min 1 s` under an hour, then `1 h 8 min` and
/// `3 d 1 h`, as the live strip's "Oldest job waited 1 h 8 min" reads
/// (#258); `1 h` and `2 d` when the smaller unit is zero.
fn span_text(millis: i64) -> String {
    let seconds = millis.max(0) / 1_000;
    let (minutes, seconds) = (seconds / 60, seconds % 60);
    let (hours, minutes) = (minutes / 60, minutes % 60);
    let (days, hours) = (hours / 24, hours % 24);
    let pair = |large: i64, large_unit: &str, small: i64, small_unit: &str| {
        if small == 0 {
            format!("{large} {large_unit}")
        } else {
            format!("{large} {large_unit} {small} {small_unit}")
        }
    };
    if days > 0 {
        pair(days, "d", hours, "h")
    } else if hours > 0 {
        pair(hours, "h", minutes, "min")
    } else if minutes > 0 {
        pair(minutes, "min", seconds, "s")
    } else {
        format!("{seconds} s")
    }
}

/// A byte count in GiB, without a trailing `.0` on round numbers.
fn gib_text(bytes: u64) -> String {
    let value = disk::gib(bytes);
    let rounded = value.round();
    if (value - rounded).abs() < 0.05 {
        format!("{}", rounded as u64)
    } else {
        format!("{value:.1}")
    }
}

#[cfg(test)]
mod tests {
    //! #240: the live read's cost on a long placement history (item 4), the
    //! disk alarm's source and heartbeat rule (item 5), and busy runners with
    //! no `current_job_id` (item 6).

    use std::path::PathBuf;

    use anyhow::Result;
    use gridops_core::fleet_health::{
        JobCategory, Severity, speed_check,
        testdata::{self, JobSeed, SampleSeed, SeededFleet},
    };
    use sqlx::SqlitePool;

    use super::*;

    const NOW: i64 = 1_800_000_000_000;
    const SECOND: i64 = 1_000;
    const MINUTE: i64 = 60 * SECOND;
    const DAY: i64 = 24 * 60 * MINUTE;
    const GIB: i64 = 1024 * 1024 * 1024;

    struct Seeded {
        directory: PathBuf,
        db: SqlitePool,
        fleet: SeededFleet,
    }

    impl Seeded {
        /// The seeded fleet, each budgeted machine with a sample (and so a
        /// heartbeat) 30 s ago and half its root disk free.
        async fn new() -> Result<Self> {
            let directory =
                std::env::temp_dir().join(format!("gridops-live-240-{}", uuid::Uuid::new_v4()));
            let db = gridops_core::connect_database_path(&directory.join("db.sqlite")).await?;
            let fleet = testdata::seed_fleet(&db).await?;
            let seeded = Self {
                directory,
                db,
                fleet,
            };
            for host in &seeded.fleet.hosts {
                if host.cpu_budget_millis > 0 {
                    seeded
                        .sample(&host.name, NOW - 30 * SECOND, 50 * GIB)
                        .await?;
                }
            }
            Ok(seeded)
        }

        fn id(&self, name: &str) -> Result<String> {
            Ok(self.fleet.host(name)?.id.clone())
        }

        async fn sample(&self, name: &str, observed_at: i64, disk_free_bytes: i64) -> Result<()> {
            let mut sample = SampleSeed::at(observed_at);
            sample.disk_free_bytes = Some(disk_free_bytes);
            testdata::insert_samples(&self.db, self.fleet.host(name)?, &[sample]).await
        }

        /// `name`'s running jobs now, by category.
        async fn running(&self, name: &str) -> Result<BTreeMap<JobCategory, u32>> {
            let id = self.id(name)?;
            let (_, hosts, _) = collect_live(&self.db, NOW).await?;
            Ok(hosts
                .into_iter()
                .find(|stats| stats.host_id == id)
                .map(|stats| stats.running_by_category)
                .unwrap_or_default())
        }

        async fn close(self) -> Result<()> {
            self.db.close().await;
            std::fs::remove_dir_all(self.directory)?;
            Ok(())
        }
    }

    /// How many SQLite virtual machine steps `work` takes on the database at
    /// `path`: it runs on a one-connection pool of its own whose progress
    /// handler counts every step. Unlike a time, the count depends only on
    /// the data and the query plans, so it shows a read that walks a
    /// machine's history even when that is still fast (#245).
    async fn vm_steps<T, Work, Done>(path: &std::path::Path, work: Work) -> Result<(T, u64)>
    where
        Work: FnOnce(SqlitePool) -> Done,
        Done: std::future::Future<Output = Result<T>>,
    {
        use std::sync::{
            Arc,
            atomic::{AtomicU64, Ordering},
        };
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(sqlx::sqlite::SqliteConnectOptions::new().filename(path))
            .await?;
        let steps = Arc::new(AtomicU64::new(0));
        {
            let mut connection = pool.acquire().await?;
            let mut handle = connection.lock_handle().await?;
            let counter = Arc::clone(&steps);
            handle.set_progress_handler(1, move || {
                counter.fetch_add(1, Ordering::Relaxed);
                true
            });
        }
        let value = work(pool.clone()).await?;
        let count = steps.load(Ordering::Relaxed);
        pool.close().await;
        Ok((value, count))
    }

    /// The VM steps of `host_live_reads` and of `collect_live` at `NOW`.
    async fn live_path_steps(path: &std::path::Path) -> Result<(u64, u64)> {
        let (_, reads) = vm_steps(path, |db| async move {
            host_live_reads(&db, NOW).await.map_err(anyhow::Error::from)
        })
        .await?;
        let (_, live) = vm_steps(path, |db| async move { collect_live(&db, NOW).await }).await?;
        Ok((reads, live))
    }

    /// Item 4: the live read reads only the placements that may still be on
    /// a machine, however long its history. On top of the synthetic week, io
    /// has four more weeks of finished jobs (each a stopped placement), and
    /// both `host_live_reads` and `collect_live` stay under the speed check.
    ///
    /// #245: the speed check alone also passed before the fix (the old
    /// statement read io's whole history in under 100 ms), so the test also
    /// counts the work SQLite does: four more weeks of history must leave
    /// both reads' VM steps within 5 % of the week's. Reading the history
    /// (the whole-history count statement, `state!='released'` on the
    /// allocations, or the queue breakdown's queued jobs without the
    /// open-jobs index, which scanned every job ever recorded) multiplies
    /// them.
    #[tokio::test]
    async fn the_live_path_passes_the_speed_check_however_long_the_placement_history() -> Result<()>
    {
        let directory =
            std::env::temp_dir().join(format!("gridops-live-history-{}", uuid::Uuid::new_v4()));
        let path = directory.join("db.sqlite");
        let db = gridops_core::connect_database_path(&path).await?;
        let week = testdata::build_week(&db, NOW).await?;
        let (reads_on_week, live_on_week) = live_path_steps(&path).await?;
        let io = week.fleet.host("io")?.id.clone();
        let start = NOW - 35 * DAY;
        let history: Vec<JobSeed> = (0..40_000_i64)
            .map(|index| {
                let created = start + index * MINUTE;
                JobSeed::completed(
                    10_000_000 + index,
                    "old job",
                    Some(&io),
                    created,
                    created + SECOND,
                    created + 30 * SECOND,
                )
            })
            .collect();
        testdata::insert_jobs(&db, &week.fleet, &history).await?;
        let placements: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM workload_placements WHERE host_id=?")
                .bind(&io)
                .fetch_one(&db)
                .await?;
        assert!(
            placements >= 40_000,
            "io's history: {placements} placements"
        );
        let (reads_with_history, live_with_history) = live_path_steps(&path).await?;
        for (read, on_week, with_history) in [
            ("host_live_reads", reads_on_week, reads_with_history),
            ("collect_live", live_on_week, live_with_history),
        ] {
            assert!(
                with_history <= on_week + on_week / 20,
                "{read}: {on_week} VM steps on the week, {with_history} with io's {placements} placements"
            );
        }

        let reads = speed_check::check(&db, async {
            host_live_reads(&db, NOW).await.map_err(anyhow::Error::from)
        })
        .await?;
        let io_read = reads
            .iter()
            .find(|read| read.id.to_string() == io)
            .ok_or_else(|| anyhow::anyhow!("io was not read"))?;
        assert_eq!(io_read.counts.running_ci_placement_count.get(), 0);
        assert_eq!(io_read.counts.uncertain_placement_count.get(), 0);
        let (fleet, _, _) = speed_check::check(&db, collect_live(&db, NOW)).await?;
        assert_eq!(
            fleet.running_total, 0,
            "every job of the history is finished"
        );
        db.close().await;
        std::fs::remove_dir_all(directory)?;
        Ok(())
    }

    /// Item 5: the disk alarm is the Overview's: the disk's observed size,
    /// not the admin-set budget. A 2 TiB root with a 500 GiB budget and
    /// 60 GiB free is critical (under 5 % of 2 TiB), as the Overview says.
    #[tokio::test]
    async fn the_disk_alarm_uses_the_observed_disk_not_the_admin_budget() -> Result<()> {
        let seeded = Seeded::new().await?;
        let io = seeded.id("io")?;
        sqlx::query("UPDATE host_resource_domains SET disk_bytes=? WHERE host_id=? AND parent_domain_id IS NULL")
            .bind(500 * GIB)
            .bind(&io)
            .execute(&seeded.db)
            .await?;
        sqlx::query(
            "UPDATE fleet_observed_domains SET disk_bytes=? WHERE host_id=? AND local_key='root'",
        )
        .bind(2048 * GIB)
        .bind(&io)
        .execute(&seeded.db)
        .await?;
        seeded.sample("io", NOW - 10 * SECOND, 60 * GIB).await?;
        let (_, _, concerns) = collect_live(&seeded.db, NOW).await?;
        let disk = concerns
            .iter()
            .find(|concern| concern.key == format!("disk:{io}"))
            .ok_or_else(|| anyhow::anyhow!("no disk concern for io: {concerns:?}"))?;
        assert_eq!(disk.severity, Severity::Critical);
        assert!(disk.detail.contains("of 2048 GiB"), "{}", disk.detail);

        // The other way round: a budget over the disk alarms on the disk.
        sqlx::query("UPDATE host_resource_domains SET disk_bytes=? WHERE host_id=? AND parent_domain_id IS NULL")
            .bind(4096 * GIB)
            .bind(&io)
            .execute(&seeded.db)
            .await?;
        sqlx::query(
            "UPDATE fleet_observed_domains SET disk_bytes=? WHERE host_id=? AND local_key='root'",
        )
        .bind(100 * GIB)
        .bind(&io)
        .execute(&seeded.db)
        .await?;
        let (_, _, concerns) = collect_live(&seeded.db, NOW).await?;
        assert!(
            concerns
                .iter()
                .all(|concern| concern.key != format!("disk:{io}")),
            "60 GiB free of an observed 100 GiB is fine: {concerns:?}"
        );
        seeded.close().await
    }

    /// Item 5: like the Overview, only a machine whose heartbeat is at most
    /// 120 s old raises a disk alarm.
    #[tokio::test]
    async fn a_machine_with_an_old_heartbeat_raises_no_disk_alarm() -> Result<()> {
        let seeded = Seeded::new().await?;
        let io = seeded.id("io")?;
        seeded.sample("io", NOW - 10 * SECOND, GIB).await?;
        for (heartbeat, alarms) in [(NOW - 121 * SECOND, false), (NOW - 120 * SECOND, true)] {
            sqlx::query("UPDATE fleet_hosts SET last_heartbeat_at=? WHERE id=?")
                .bind(heartbeat)
                .bind(&io)
                .execute(&seeded.db)
                .await?;
            let (_, _, concerns) = collect_live(&seeded.db, NOW).await?;
            assert_eq!(
                concerns
                    .iter()
                    .any(|concern| concern.key == format!("disk:{io}")),
                alarms,
                "heartbeat {} s ago: {concerns:?}",
                (NOW - heartbeat) / SECOND
            );
        }
        seeded.close().await
    }

    /// Item 6: a GitLab runner is running a job while it is busy
    /// (`status='busy'`, `busy=1`), which is all `set_gitlab_runner_busy`
    /// writes: it never sets `current_job_id`. It counts once, and not while
    /// it is online. #253 item 4: it counts under the category of the
    /// `gitlab_jobs` row running on it (by `gitlab_runner_id` on the
    /// runner's GitLab instance), never as `other` because its job is not a
    /// `workflow_jobs` row.
    #[tokio::test]
    async fn a_busy_gitlab_runner_counts_as_running_once_and_an_online_one_does_not() -> Result<()>
    {
        let seeded = Seeded::new().await?;
        let io = seeded.id("io")?;
        let mut github = JobSeed::running(
            1,
            "unit tests",
            Some(&io),
            NOW - 3 * MINUTE,
            NOW - 2 * MINUTE,
        );
        github.category = Some(JobCategory::Test);
        let mut gitlab =
            JobSeed::running(2, "rspec", Some(&io), NOW - 3 * MINUTE, NOW - 2 * MINUTE);
        gitlab.gitlab = true;
        gitlab.category = Some(JobCategory::Build);
        testdata::insert_jobs(&seeded.db, &seeded.fleet, &[github, gitlab]).await?;
        let set_gitlab = |busy: bool, current_job_id: Option<i64>| {
            sqlx::query(
                "UPDATE runners SET busy=?,status=CASE WHEN ? THEN 'busy' ELSE 'online' END,current_job_id=?
                  WHERE ci_platform='gitlab'",
            )
            .bind(busy)
            .bind(busy)
            .bind(current_job_id)
            .execute(&seeded.db)
        };

        set_gitlab(true, None).await?;
        assert_eq!(
            seeded.running("io").await?,
            BTreeMap::from([(JobCategory::Test, 1), (JobCategory::Build, 1)]),
            "the GitHub job, and the busy GitLab runner's job by its own category"
        );

        set_gitlab(false, None).await?;
        assert_eq!(
            seeded.running("io").await?,
            BTreeMap::from([(JobCategory::Test, 1)]),
            "an online GitLab runner runs nothing"
        );

        // With `current_job_id` set (here the GitHub job's id) it still
        // counts once, and never takes that unrelated job's category.
        set_gitlab(true, Some(1)).await?;
        assert_eq!(
            seeded.running("io").await?,
            BTreeMap::from([(JobCategory::Test, 1), (JobCategory::Build, 1)]),
        );

        // A job row with no category yet (the backfill hasn't run) is
        // `other`, as a GitHub job's is.
        sqlx::query("UPDATE gitlab_jobs SET category=NULL")
            .execute(&seeded.db)
            .await?;
        assert_eq!(
            seeded.running("io").await?,
            BTreeMap::from([(JobCategory::Test, 1), (JobCategory::Other, 1)]),
        );

        // A busy runner whose job is not running there (finished, or a job
        // with the same runner id on another GitLab instance) is `other`.
        sqlx::query("UPDATE gitlab_jobs SET category='build',status='success'")
            .execute(&seeded.db)
            .await?;
        assert_eq!(
            seeded.running("io").await?,
            BTreeMap::from([(JobCategory::Test, 1), (JobCategory::Other, 1)]),
            "a finished GitLab job is not what the runner runs now"
        );
        sqlx::query("UPDATE gitlab_jobs SET status='running'")
            .execute(&seeded.db)
            .await?;
        sqlx::query(
            "INSERT INTO gitlab_connections (id,name,base_url,scope_kind,scope_id,scope_path,access_token_key,webhook_secret_key,created_at,updated_at)
             VALUES ('other-instance','other','https://gitlab.other','project',1,'acme/app','other.token','other.webhook',1,1)",
        )
        .execute(&seeded.db)
        .await?;
        sqlx::query("UPDATE gitlab_jobs SET connection_id='other-instance'")
            .execute(&seeded.db)
            .await?;
        assert_eq!(
            seeded.running("io").await?,
            BTreeMap::from([(JobCategory::Test, 1), (JobCategory::Other, 1)]),
            "a job on another GitLab instance is not this runner's"
        );
        seeded.close().await
    }

    /// The live stats of the machine `name`.
    async fn host_stats(seeded: &Seeded, name: &str) -> Result<HostLiveStats> {
        let id = seeded.id(name)?;
        let (_, hosts, _) = collect_live(&seeded.db, NOW).await?;
        hosts
            .into_iter()
            .find(|stats| stats.host_id == id)
            .ok_or_else(|| anyhow::anyhow!("no live stats for {name}"))
    }

    /// #253 item 1: a machine's start failures in the last hour are the
    /// `placement_failed` events the start gate and the machine rollup count
    /// (`start_health::is_start_failure`): an operator's release, a runner
    /// removed before its start, a registration timeout or a cancelled
    /// command are not the machine's start failures.
    #[tokio::test]
    async fn start_failures_count_only_what_the_start_gate_counts() -> Result<()> {
        use gridops_core::fleet::failure::PlacementFailureReason as Reason;

        let seeded = Seeded::new().await?;
        let io = seeded.fleet.host("io")?.clone();
        for (at, reason, agent_code) in [
            (NOW - 10 * MINUTE, Reason::AgentSessionLost, None),
            (NOW - 9 * MINUTE, Reason::OperatorReleased, None),
            (NOW - 8 * MINUTE, Reason::RunnerRemovedBeforeStart, None),
            (NOW - 7 * MINUTE, Reason::RegistrationTimedOut, None),
            (NOW - 6 * MINUTE, Reason::AgentReported, Some("cancelled")),
            // A real one, but over an hour ago.
            (NOW - 61 * MINUTE, Reason::StartDeadlineExpired, None),
        ] {
            testdata::insert_placement_failure(&seeded.db, &io, at, reason, agent_code).await?;
        }
        assert_eq!(host_stats(&seeded, "io").await?.start_failures_last_hour, 1);
        seeded.close().await
    }

    /// `n` runner starts on `name` in the last hour, `failed` of them failing
    /// on the machine (a `placement_failed` event each, at the start).
    async fn starts(seeded: &Seeded, name: &str, n: i64, failed: i64) -> Result<()> {
        use gridops_core::fleet::failure::PlacementFailureReason as Reason;
        use gridops_core::fleet_health::testdata::PlacementEnd;

        let host = seeded.fleet.host(name)?.clone();
        for index in 0..n {
            let at = NOW - 50 * MINUTE + index * MINUTE;
            let fails = index < failed;
            let end = if fails {
                PlacementEnd::Failed
            } else {
                PlacementEnd::Stopped
            };
            testdata::insert_start_attempt(&seeded.db, &seeded.fleet, &host, at, end).await?;
            if fails {
                testdata::insert_placement_failure(
                    &seeded.db,
                    &host,
                    at + SECOND,
                    Reason::StartDeadlineExpired,
                    None,
                )
                .await?;
            }
        }
        Ok(())
    }

    /// #253 item 2: a machine with at least 3 start failures that are at
    /// least 25 % of its runner starts in the last hour raises
    /// `start-failures:<host>`, a warning, critical from 50 %, worded as the
    /// epic words it and linking to the machine's row.
    #[tokio::test]
    async fn a_machine_failing_a_share_of_its_starts_raises_a_start_failure_concern() -> Result<()>
    {
        let seeded = Seeded::new().await?;
        starts(&seeded, "io", 12, 3).await?;
        starts(&seeded, "Jupiter", 6, 3).await?;
        starts(&seeded, "callisto", 13, 3).await?;
        starts(&seeded, "europa", 2, 2).await?;
        // Operator releases are not start failures, so they raise nothing
        // however many there are.
        let macbook = seeded.fleet.host("MacBookPro")?.clone();
        for minute in 1..=5 {
            testdata::insert_placement_failure(
                &seeded.db,
                &macbook,
                NOW - minute * MINUTE,
                gridops_core::fleet::failure::PlacementFailureReason::OperatorReleased,
                None,
            )
            .await?;
        }
        let (_, _, concerns) = collect_live(&seeded.db, NOW).await?;
        let concern = |name: &str| -> Result<Option<Concern>> {
            let key = format!("start-failures:{}", seeded.id(name)?);
            Ok(concerns.iter().find(|concern| concern.key == key).cloned())
        };

        let io = concern("io")?.ok_or_else(|| anyhow::anyhow!("io: {concerns:?}"))?;
        assert_eq!(io.severity, Severity::Warning, "3 of 12 is 25 %");
        assert_eq!(io.title, "io failed 3 of 12 runner starts in the last hour");
        assert_eq!(
            io.link.as_deref(),
            Some(format!("/fleet/health#machine-{}", seeded.id("io")?).as_str())
        );
        assert_eq!(io.subject, ConcernSubject::Host(seeded.id("io")?));
        assert_eq!(
            io.since,
            Some(NOW - 50 * MINUTE + SECOND),
            "the oldest failure"
        );
        assert!(io.detail.contains("25%"), "{}", io.detail);

        let jupiter = concern("Jupiter")?.ok_or_else(|| anyhow::anyhow!("{concerns:?}"))?;
        assert_eq!(jupiter.severity, Severity::Critical, "3 of 6 is 50 %");
        assert_eq!(
            jupiter.title,
            "Jupiter failed 3 of 6 runner starts in the last hour"
        );

        assert!(concern("callisto")?.is_none(), "3 of 13 is under 25 %");
        assert!(concern("europa")?.is_none(), "2 failures are under 3");
        assert!(
            concern("MacBookPro")?.is_none(),
            "releases are not failures"
        );
        seeded.close().await
    }

    /// #253 item 3: a GitLab job waiting for a runner (`pending`) is queued
    /// like a GitHub one: in the total, by its category, under the pool that
    /// will run it (by the same rule the Overview's GitHub jobs are split
    /// by), as unmatched when no pool will, and as the oldest queued job. A
    /// job recorded under two connections of one GitLab instance counts once;
    /// a `created` job (waiting for earlier stages) or a running one is not
    /// queued.
    #[tokio::test]
    async fn pending_gitlab_jobs_are_queued_like_github_jobs() -> Result<()> {
        let seeded = Seeded::new().await?;
        let mut github = JobSeed::queued(1, "unit tests", NOW - 2 * MINUTE);
        github.category = Some(JobCategory::Test);
        let mut served = JobSeed::queued(2, "compile", NOW - 8 * MINUTE);
        served.gitlab = true;
        served.category = Some(JobCategory::Build);
        let mut unmatched = JobSeed::queued(3, "train", NOW - MINUTE);
        unmatched.gitlab = true;
        unmatched.labels = vec!["gpu".to_owned()];
        unmatched.category = None;
        let mut created = JobSeed::queued(4, "later stage", NOW - 30 * MINUTE);
        created.gitlab = true;
        testdata::insert_jobs(
            &seeded.db,
            &seeded.fleet,
            &[github, served, unmatched, created],
        )
        .await?;
        sqlx::query("UPDATE gitlab_jobs SET status='created' WHERE job_id=4")
            .execute(&seeded.db)
            .await?;
        sqlx::query(
            "INSERT INTO runner_pool_gitlab_connections (pool_id,connection_id,created_at) VALUES (?,'testdata-gitlab',1)",
        )
        .bind(&seeded.fleet.pool_id)
        .execute(&seeded.db)
        .await?;
        // The same pending job seen through the project's group connection.
        sqlx::query(
            "INSERT INTO gitlab_connections (id,name,base_url,scope_kind,scope_id,scope_path,access_token_key,webhook_secret_key,created_at,updated_at)
             VALUES ('group-connection','group','https://gitlab.test','group',2,'acme','group.token','group.webhook',1,1)",
        )
        .execute(&seeded.db)
        .await?;
        sqlx::query(
            "INSERT INTO gitlab_jobs (id,connection_id,project_id,job_id,pipeline_id,status,tag_list,status_changed_at,created_at,updated_at,category,job_name,project_path)
             SELECT 'group-copy',  'group-connection',project_id,job_id,pipeline_id,status,tag_list,status_changed_at,created_at+1000,updated_at,category,job_name,project_path
               FROM gitlab_jobs WHERE job_id=2",
        )
        .execute(&seeded.db)
        .await?;

        let (fleet, _, concerns) = collect_live(&seeded.db, NOW).await?;
        assert_eq!(fleet.queued_total, 3, "{fleet:?}");
        assert_eq!(
            fleet.queued_by_category,
            BTreeMap::from([
                (JobCategory::Test, 1),
                (JobCategory::Build, 1),
                (JobCategory::Other, 1)
            ])
        );
        assert_eq!(fleet.queued_unmatched, 1);
        assert_eq!(
            fleet
                .queued_by_pool
                .iter()
                .map(|pool| (pool.pool_id.as_str(), pool.queued))
                .collect::<Vec<_>>(),
            vec![(seeded.fleet.pool_id.as_str(), 2)]
        );
        let oldest = fleet
            .oldest_queued
            .ok_or_else(|| anyhow::anyhow!("no oldest queued job"))?;
        assert_eq!(
            (
                oldest.job_name.as_str(),
                oldest.repository.as_str(),
                oldest.wait_ms
            ),
            ("compile", seeded.fleet.repository.as_str(), 8 * MINUTE)
        );
        let oldest_concern = concerns
            .iter()
            .find(|concern| concern.key == "oldest-queued")
            .ok_or_else(|| anyhow::anyhow!("no oldest-queued concern: {concerns:?}"))?;
        assert_eq!(oldest_concern.severity, Severity::Warning);
        assert!(
            oldest_concern.detail.starts_with("compile in "),
            "{}",
            oldest_concern.detail
        );
        let unmatched_concern = concerns
            .iter()
            .find(|concern| concern.key == "unmatched-jobs")
            .ok_or_else(|| anyhow::anyhow!("no unmatched-jobs concern: {concerns:?}"))?;
        assert_eq!(unmatched_concern.title, "1 queued job matches no pool");
        seeded.close().await
    }

    /// Links the seeded pool to the testdata GitLab connection, so it runs
    /// that connection's jobs tagged `self-hosted`, `gridops`.
    async fn link_gitlab_pool(seeded: &Seeded) -> Result<()> {
        sqlx::query(
            "INSERT INTO runner_pool_gitlab_connections (pool_id,connection_id,created_at) VALUES (?,'testdata-gitlab',1)",
        )
        .bind(&seeded.fleet.pool_id)
        .execute(&seeded.db)
        .await?;
        Ok(())
    }

    /// A GitLab job waiting for a runner, created at `created_at`.
    fn pending_gitlab(id: i64, name: &str, created_at: i64) -> JobSeed {
        let mut job = JobSeed::queued(id, name, created_at);
        job.gitlab = true;
        job
    }

    /// #253 review: a GitLab job is queued only while `pending` is the
    /// furthest along of its rows: one pending on one connection but already
    /// running on another (the project's and its group's) has its runner.
    #[tokio::test]
    async fn a_gitlab_job_running_on_another_connection_is_not_queued() -> Result<()> {
        let seeded = Seeded::new().await?;
        testdata::insert_jobs(
            &seeded.db,
            &seeded.fleet,
            &[pending_gitlab(2, "compile", NOW - 8 * MINUTE)],
        )
        .await?;
        link_gitlab_pool(&seeded).await?;
        sqlx::query(
            "INSERT INTO gitlab_connections (id,name,base_url,scope_kind,scope_id,scope_path,access_token_key,webhook_secret_key,created_at,updated_at)
             VALUES ('group-connection','group','https://gitlab.test','group',2,'acme','group.token','group.webhook',1,1)",
        )
        .execute(&seeded.db)
        .await?;
        sqlx::query(
            "INSERT INTO gitlab_jobs (id,connection_id,project_id,job_id,pipeline_id,status,tag_list,status_changed_at,created_at,updated_at,category,job_name,project_path)
             SELECT 'group-copy','group-connection',project_id,job_id,pipeline_id,'running',tag_list,status_changed_at,created_at,updated_at,category,job_name,project_path
               FROM gitlab_jobs WHERE job_id=2",
        )
        .execute(&seeded.db)
        .await?;
        let (fleet, _, concerns) = collect_live(&seeded.db, NOW).await?;
        assert_eq!(fleet.queued_total, 0, "{fleet:?}");
        assert!(fleet.queued_by_pool.is_empty(), "{fleet:?}");
        assert_eq!(fleet.oldest_queued, None);
        assert!(
            concerns
                .iter()
                .all(|concern| concern.key != "oldest-queued"),
            "{concerns:?}"
        );
        seeded.close().await
    }

    /// #253 review: a GitLab job only a manual-mode pool can run is under
    /// that pool and not unmatched, and, as a GitHub job on a manual pool,
    /// is not the oldest job waiting for capacity (nothing starts a runner
    /// for it).
    #[tokio::test]
    async fn a_gitlab_job_only_a_manual_pool_runs_is_under_it_and_not_the_oldest() -> Result<()> {
        let seeded = Seeded::new().await?;
        testdata::insert_jobs(
            &seeded.db,
            &seeded.fleet,
            &[pending_gitlab(2, "compile", NOW - 20 * MINUTE)],
        )
        .await?;
        link_gitlab_pool(&seeded).await?;
        sqlx::query("UPDATE runner_pools SET autoscaling_enabled=0 WHERE id=?")
            .bind(&seeded.fleet.pool_id)
            .execute(&seeded.db)
            .await?;
        let (fleet, _, concerns) = collect_live(&seeded.db, NOW).await?;
        assert_eq!((fleet.queued_total, fleet.queued_unmatched), (1, 0));
        assert_eq!(
            fleet
                .queued_by_pool
                .iter()
                .map(|pool| (pool.pool_id.as_str(), pool.queued))
                .collect::<Vec<_>>(),
            vec![(seeded.fleet.pool_id.as_str(), 1)]
        );
        assert_eq!(fleet.oldest_queued, None);
        assert!(
            concerns
                .iter()
                .all(|concern| concern.key != "oldest-queued" && concern.key != "unmatched-jobs"),
            "{concerns:?}"
        );
        seeded.close().await
    }

    /// #253 review: a GitHub job that has waited longer than every GitLab
    /// job is the oldest queued job, named from its `workflow_jobs` row.
    #[tokio::test]
    async fn a_github_job_older_than_every_gitlab_job_is_the_oldest() -> Result<()> {
        let seeded = Seeded::new().await?;
        let mut github = JobSeed::queued(1, "unit tests", NOW - 12 * MINUTE);
        github.category = Some(JobCategory::Test);
        testdata::insert_jobs(
            &seeded.db,
            &seeded.fleet,
            &[
                github,
                pending_gitlab(2, "compile", NOW - 8 * MINUTE),
                pending_gitlab(3, "package", NOW - 11 * MINUTE),
            ],
        )
        .await?;
        link_gitlab_pool(&seeded).await?;
        let (fleet, _, concerns) = collect_live(&seeded.db, NOW).await?;
        assert_eq!(fleet.queued_total, 3);
        let oldest = fleet
            .oldest_queued
            .ok_or_else(|| anyhow::anyhow!("no oldest queued job"))?;
        assert_eq!(
            (oldest.job_id, oldest.job_name.as_str(), oldest.wait_ms),
            (1, "unit tests", 12 * MINUTE)
        );
        let concern = concerns
            .iter()
            .find(|concern| concern.key == "oldest-queued")
            .ok_or_else(|| anyhow::anyhow!("{concerns:?}"))?;
        assert_eq!(concern.severity, Severity::Critical, "12 min is over 10");
        assert!(
            concern.detail.starts_with("unit tests in "),
            "{}",
            concern.detail
        );
        seeded.close().await
    }

    /// #253 review: a failure of a start issued before the hour, or of a
    /// placement that never got a `start_runner` command (a prepare that
    /// ran out of time), is a start failure but not one of the hour's
    /// starts; the share's denominator is never less than the failures, so
    /// 4 failures and 1 start read "4 of 4", critical.
    #[tokio::test]
    async fn more_start_failures_than_starts_reads_as_every_start_failing() -> Result<()> {
        use gridops_core::fleet::failure::PlacementFailureReason as Reason;

        let seeded = Seeded::new().await?;
        starts(&seeded, "io", 1, 1).await?;
        let io = seeded.fleet.host("io")?.clone();
        for minute in [5, 6, 7] {
            testdata::insert_placement_failure(
                &seeded.db,
                &io,
                NOW - minute * MINUTE,
                Reason::PrepareDeadlineExpired,
                None,
            )
            .await?;
        }
        let (_, hosts, concerns) = collect_live(&seeded.db, NOW).await?;
        let io_id = seeded.id("io")?;
        assert_eq!(
            hosts
                .iter()
                .find(|stats| stats.host_id == io_id)
                .map(|stats| stats.start_failures_last_hour),
            Some(4)
        );
        let concern = concerns
            .iter()
            .find(|concern| concern.key == format!("start-failures:{io_id}"))
            .ok_or_else(|| anyhow::anyhow!("{concerns:?}"))?;
        assert_eq!(concern.severity, Severity::Critical);
        assert_eq!(
            concern.title,
            "io failed 4 of 4 runner starts in the last hour"
        );
        assert!(concern.detail.starts_with("100% "), "{}", concern.detail);
        seeded.close().await
    }

    /// #253 review: an agent-reported failure counts unless its code says
    /// the request was at fault (`cancelled`, `configuration`,
    /// `unsupported`): `dependency` is the machine's.
    #[tokio::test]
    async fn an_agent_reported_failure_with_a_machine_code_counts() -> Result<()> {
        use gridops_core::fleet::failure::PlacementFailureReason as Reason;

        let seeded = Seeded::new().await?;
        let io = seeded.fleet.host("io")?.clone();
        for (minute, code) in [
            (5, Some("dependency")),
            (6, None),
            (7, Some("configuration")),
            (8, Some("unsupported")),
        ] {
            testdata::insert_placement_failure(
                &seeded.db,
                &io,
                NOW - minute * MINUTE,
                Reason::AgentReported,
                code,
            )
            .await?;
        }
        assert_eq!(host_stats(&seeded, "io").await?.start_failures_last_hour, 2);
        seeded.close().await
    }

    /// 5,000 more finished GitLab jobs on io (each with its runner and
    /// placement), completed 20 days before `NOW`, and 5,000 more
    /// `placement_failed` events on io between 23 and 21.6 hours before it,
    /// numbered from `first`.
    async fn old_history(db: &SqlitePool, fleet: &SeededFleet, first: i64) -> Result<()> {
        let io = fleet.host("io")?.id.clone();
        let history: Vec<JobSeed> = (first..first + 5_000)
            .map(|index| {
                let created = NOW - 20 * DAY + index * SECOND;
                let mut job = JobSeed::completed(
                    100 + index,
                    "old job",
                    Some(&io),
                    created,
                    created + SECOND,
                    created + 30 * SECOND,
                );
                job.gitlab = true;
                job
            })
            .collect();
        testdata::insert_jobs(db, fleet, &history).await?;
        let mut tx = db.begin().await?;
        for index in first..first + 5_000 {
            sqlx::query(
                "INSERT INTO fleet_events (id,host_id,kind,detail_json,created_at) VALUES (?,?,'placement_failed','{\"reason\":\"agent_session_lost\"}',?)",
            )
            .bind(uuid::Uuid::new_v4().to_string())
            .bind(&io)
            .bind(NOW - 23 * 60 * MINUTE + index / 2)
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await?;
        Ok(())
    }

    /// #253 items 1 and 3: the live path's reads stay bounded: its work is
    /// the same with 10,000 finished GitLab jobs and 10,000 `placement_failed`
    /// events older than an hour as with 5,000 of each (the first 5,000 make
    /// the indexes as deep as they get here, so only a read that walks the
    /// history shows). Before #253 `host_live_reads` read the machine's whole
    /// last day of failures to count its last hour.
    #[tokio::test]
    async fn the_live_path_does_not_read_gitlab_history_or_old_failures() -> Result<()> {
        use gridops_core::fleet::failure::PlacementFailureReason as Reason;

        let directory =
            std::env::temp_dir().join(format!("gridops-live-253-{}", uuid::Uuid::new_v4()));
        let path = directory.join("db.sqlite");
        let db = gridops_core::connect_database_path(&path).await?;
        let fleet = testdata::seed_fleet(&db).await?;
        let mut pending = JobSeed::queued(1, "compile", NOW - MINUTE);
        pending.gitlab = true;
        testdata::insert_jobs(&db, &fleet, &[pending]).await?;
        testdata::insert_placement_failure(
            &db,
            fleet.host("io")?,
            NOW - MINUTE,
            Reason::AgentSessionLost,
            None,
        )
        .await?;
        old_history(&db, &fleet, 0).await?;
        let (_, before) = vm_steps(&path, |db| async move { collect_live(&db, NOW).await }).await?;
        old_history(&db, &fleet, 5_000).await?;
        let (_, after) = vm_steps(&path, |db| async move { collect_live(&db, NOW).await }).await?;
        assert!(
            after <= before + before / 20,
            "collect_live: {before} VM steps, {after} with 5,000 more old GitLab jobs and failures"
        );
        let (fleet_stats, hosts, _) = collect_live(&db, NOW).await?;
        assert_eq!(fleet_stats.queued_total, 1, "the pending GitLab job");
        let io = fleet.host("io")?.id.clone();
        assert_eq!(
            hosts
                .iter()
                .find(|stats| stats.host_id == io)
                .map(|stats| stats.start_failures_last_hour),
            Some(1)
        );
        db.close().await;
        std::fs::remove_dir_all(directory)?;
        Ok(())
    }

    /// #245: the live path's timings on the synthetic week, for the record in
    /// `docs/handoffs/244-review.md`; a measurement, not a check. Run it in a
    /// release build: `cargo test --release -p gridops-api --bin gridops-api
    /// -- --ignored --nocapture live_path_timings`. The endpoint still serves
    /// the fake source (#204 plugs in the database), so its lines are the
    /// cache and HTTP overhead alone.
    #[tokio::test]
    #[ignore = "a measurement, not a check (#245)"]
    async fn live_path_timings_on_the_synthetic_week() -> Result<()> {
        use std::time::{Duration, Instant};

        use axum::http::StatusCode;

        use crate::fleet::health_test_support::{HEALTH, Harness};

        const RUNS: usize = 21;
        fn summary(name: &str, mut times: Vec<Duration>) {
            times.sort();
            let at = |share: usize| times[(times.len() - 1) * share / 100];
            eprintln!(
                "{name}: median {:?}, p95 {:?}, max {:?} ({} runs)",
                at(50),
                at(95),
                at(100),
                times.len()
            );
        }

        let directory =
            std::env::temp_dir().join(format!("gridops-live-timings-{}", uuid::Uuid::new_v4()));
        let db = gridops_core::connect_database_path(&directory.join("db.sqlite")).await?;
        testdata::build_week(&db, NOW).await?;
        host_live_reads(&db, NOW).await?;
        collect_live(&db, NOW).await?;
        let (mut reads, mut queue, mut live) = (Vec::new(), Vec::new(), Vec::new());
        for _ in 0..RUNS {
            let started = Instant::now();
            host_live_reads(&db, NOW).await?;
            reads.push(started.elapsed());
            let started = Instant::now();
            crate::resources::fleet_queue_breakdown(&db, NOW)
                .await
                .map_err(|error| anyhow::anyhow!("{error:?}"))?;
            queue.push(started.elapsed());
            let started = Instant::now();
            collect_live(&db, NOW).await?;
            live.push(started.elapsed());
        }
        summary("host_live_reads", reads);
        summary("fleet_queue_breakdown", queue);
        summary("collect_live", live);
        db.close().await;
        std::fs::remove_dir_all(directory)?;

        let harness = Harness::new().await?;
        let uri = format!("{HEALTH}?window=24h");
        let (mut cold, mut cached) = (Vec::new(), Vec::new());
        for _ in 0..RUNS {
            harness.clock.advance(31_000);
            let started = Instant::now();
            let (status, body) = harness.admin_get(&uri).await?;
            cold.push(started.elapsed());
            // PR-Agent 0c2908df410d: time the 200s, never an error path.
            assert_eq!(status, StatusCode::OK, "{body}");
            assert_eq!(body.get("stale"), Some(&serde_json::json!(false)));
            let started = Instant::now();
            let (status, body) = harness.admin_get(&uri).await?;
            cached.push(started.elapsed());
            assert_eq!(status, StatusCode::OK, "{body}");
        }
        summary("GET /fleet/health, cold (fake source)", cold);
        summary("GET /fleet/health, cached", cached);
        Ok(())
    }

    /// #258 item 1: a long wait in a concern's text reads in hours and days,
    /// as the live strip's "Oldest job waited 1 h 8 min" does, not as
    /// thousands of minutes; under an hour it keeps its seconds.
    #[test]
    fn span_text_rolls_long_waits_up_into_hours_and_days() {
        const HOUR: i64 = 60 * MINUTE;
        assert_eq!(span_text(0), "0 s");
        assert_eq!(span_text(42 * SECOND + 999), "42 s");
        assert_eq!(span_text(7 * MINUTE), "7 min");
        assert_eq!(span_text(10 * MINUTE + SECOND), "10 min 1 s");
        assert_eq!(span_text(59 * MINUTE + 59 * SECOND), "59 min 59 s");
        assert_eq!(span_text(HOUR), "1 h");
        // The walkthrough's "68 min 27 s" and "4391 min 27 s".
        assert_eq!(span_text(68 * MINUTE + 27 * SECOND), "1 h 8 min");
        assert_eq!(span_text(4391 * MINUTE + 27 * SECOND), "3 d 1 h");
        assert_eq!(
            span_text(23 * HOUR + 59 * MINUTE + 59 * SECOND),
            "23 h 59 min"
        );
        assert_eq!(span_text(DAY), "1 d");
        assert_eq!(span_text(2 * DAY + 59 * MINUTE), "2 d");
        assert_eq!(span_text(-5 * SECOND), "0 s");
    }
}
