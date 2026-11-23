//! The live half of the fleet health response (#201): what is true of each
//! machine and of the queue right now, and the live areas of concern
//! (offline, start gate, disk, unmatched jobs, oldest queued, uncertain
//! placements).
//!
//! Everything a principal would gate is read through #197's fleet-wide reads
//! with none: `gridops_core::fleet::registry::host_live_reads` (per-machine
//! counts, root budget and allocation, latest root measurement, placement
//! failures, start gate) and `crate::resources::fleet_queue_breakdown` (the
//! Overview's queue split over every installation, with the queued jobs by
//! category). The only queries here are the ones those two can't answer, each
//! a bounded index read (#179): a machine's newest sample (per host, on
//! `host_samples`' `(host_id, observed_at, id)` index), the categories of the
//! jobs its runners are running now (per host, on `workload_placements`'
//! `(host_id, state)` index), the oldest waiting job's name and repository
//! (by id), and the pool names (a table of a few rows).

use std::collections::{BTreeMap, HashMap};

use gridops_core::disk;
use gridops_core::fleet::registry::host_live_reads;
use gridops_core::fleet_health::{
    Concern, ConcernSubject, FleetLiveStats, HostLiveStats, JobCategory, OldestQueued, PoolQueue,
    Severity,
};
use sqlx::{Row as _, SqlitePool};

/// A machine is online while it has a sample this recent.
const ONLINE_WINDOW_MILLIS: i64 = 5 * 60_000;
/// The oldest waiting job warns past this wait and is critical past twice it.
const QUEUED_WARN_MILLIS: i64 = 5 * 60_000;
const QUEUED_CRITICAL_MILLIS: i64 = 10 * 60_000;

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

        // What its runners are running now, by category: a running CI
        // placement's runner's current job, NULL category as `other`. Runs on
        // `workload_placements`' `(host_id, state)` index; running
        // fix placements (a category no job row has) come from the counts.
        let running: Vec<(Option<String>, i64)> = sqlx::query_as(
            "SELECT wj.category,COUNT(*) FROM workload_placements p
               JOIN workload_intents i ON i.id=p.intent_id
               JOIN runners r ON r.id=p.runner_id
               LEFT JOIN workflow_jobs wj ON wj.id=r.current_job_id
              WHERE p.host_id=? AND p.state='running' AND i.kind='ci_runner'
                AND r.current_job_id IS NOT NULL
              GROUP BY wj.category",
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
            start_failures_last_hour: u32::try_from(read.failed_placements_last_hour.get())?,
        });

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

        // `disk`: the root disk's alarm, on the measurement `/fleet/hosts`
        // shows as `latestRootMeasurement` against the root domain's size.
        let disk_total = read.physical_budget.disk_bytes.get();
        let disk_free = measurement
            .and_then(|row| row.disk_free_bytes)
            .map(|value| value.get());
        if let Some(alarm) = disk_free.and_then(|free| disk::alarm_for(disk_total, free)) {
            let (severity, level) = match alarm {
                disk::DiskAlarm::Critical => (Severity::Critical, "critical"),
                disk::DiskAlarm::Warning => (Severity::Warning, "warning"),
            };
            let free = disk_free.unwrap_or_default();
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

    // The oldest job waiting for capacity, with its name and repository for
    // the concern's sentence (a lookup by id; the breakdown only counts).
    let oldest_queued = match (
        queue.queue.oldest_waiting_job,
        queue.queue.oldest_waiting_millis,
    ) {
        (Some(job_id), Some(wait_ms)) => {
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
            job.map(|row| OldestQueued {
                job_id,
                job_name: row.get("name"),
                repository: row.get("repository"),
                queued_at: row.get("queued_at"),
                wait_ms,
            })
        }
        _ => None,
    };

    // `unmatched-jobs`: queued jobs no pool can run, as the Overview splits
    // them.
    if queue.queue.unmatched > 0 {
        let unmatched = queue.queue.unmatched;
        let job = if unmatched == 1 { "job" } else { "jobs" };
        let asks = if unmatched == 1 { "asks" } else { "ask" };
        let detail = if queue.queue.unavailable == 0 {
            format!(
                "{unmatched} queued {job} {asks} for labels no runner pool offers; they will wait until one does."
            )
        } else if queue
            .queue
            .missing_labels
            .iter()
            .map(|missing| missing.jobs)
            .sum::<i64>()
            == 0
        {
            format!(
                "{unmatched} queued {job} {asks} for a pool that is paused, tripped or on another platform; nothing will start for them."
            )
        } else {
            format!(
                "{unmatched} queued {job} match no pool that can run them; they will wait until one does."
            )
        };
        concerns.push(Concern {
            key: "unmatched-jobs".to_owned(),
            severity: Severity::Critical,
            title: format!("{unmatched} queued {job} match no pool"),
            detail,
            subject: ConcernSubject::Fleet,
            link: Some("/queued-jobs".to_owned()),
            since: None,
        });
    }

    // `oldest-queued`: the oldest waiting job's wait, over five minutes.
    if let Some(oldest) = &oldest_queued {
        if oldest.wait_ms > QUEUED_WARN_MILLIS {
            let severity = if oldest.wait_ms > QUEUED_CRITICAL_MILLIS {
                Severity::Critical
            } else {
                Severity::Warning
            };
            let wait = span_text(oldest.wait_ms);
            concerns.push(Concern {
                key: "oldest-queued".to_owned(),
                severity,
                title: format!("A job has waited {wait}"),
                detail: format!(
                    "{} in {} has waited {wait} to start.",
                    oldest.job_name, oldest.repository
                ),
                subject: ConcernSubject::Fleet,
                link: Some("/queued-jobs".to_owned()),
                since: Some(oldest.queued_at),
            });
        }
    }

    let names: HashMap<String, String> =
        sqlx::query_as::<_, (String, String)>("SELECT id,name FROM runner_pools")
            .fetch_all(db)
            .await?
            .into_iter()
            .collect();
    let mut queued_by_pool = queue
        .queue
        .by_pool
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

    let fleet = FleetLiveStats {
        queued_total: u32::try_from(queue.queue.total())?,
        queued_by_category: queue
            .by_category
            .iter()
            .map(|(category, jobs)| Ok((*category, u32::try_from(*jobs)?)))
            .collect::<anyhow::Result<BTreeMap<_, _>>>()?,
        queued_by_pool,
        queued_unmatched: u32::try_from(queue.queue.unmatched)?,
        oldest_queued,
        running_total: fleet_running.values().sum(),
        running_by_category: fleet_running,
    };
    Ok((fleet, hosts, concerns))
}

/// A wait as a person reads it: `42 s`, `7 min`, `10 min 1 s`.
fn span_text(millis: i64) -> String {
    let seconds = millis / 1_000;
    if seconds >= 60 {
        let minutes = seconds / 60;
        let seconds = seconds % 60;
        if seconds == 0 {
            format!("{minutes} min")
        } else {
            format!("{minutes} min {seconds} s")
        }
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
