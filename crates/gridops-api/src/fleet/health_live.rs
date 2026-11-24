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
//! only queries here are the ones those two can't answer, each
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
        // is running a job GridOps never matched. A job is categorised from
        // its `workflow_jobs` row only for a GitHub runner (another
        // platform's job ids are not `workflow_jobs` ids); with no row it is
        // `other`, like a NULL category. Runs on `workload_placements`'
        // `(host_id, state)` index; running fix placements (a category no
        // job row has) come from the counts.
        let running: Vec<(Option<String>, i64)> = sqlx::query_as(
            "SELECT wj.category,COUNT(*) FROM workload_placements p
               JOIN workload_intents i ON i.id=p.intent_id
               JOIN runners r ON r.id=p.runner_id
               LEFT JOIN workflow_jobs wj
                 ON wj.id=r.current_job_id AND r.ci_platform='github'
              WHERE p.host_id=? AND p.state='running' AND i.kind='ci_runner'
                AND (r.current_job_id IS NOT NULL OR (r.status='busy' AND r.busy=1))
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

    // The oldest job waiting for capacity, with its name and repository for
    // the concern's sentence (a lookup by id; the breakdown only counts). The
    // breakdown's job clause already requires the run and repository rows, so
    // the lookup only misses if the job left the queue between the two reads;
    // the concern below still fires then, unnamed.
    let mut oldest_queued = None;
    let mut oldest_unnamed = None;
    if let Some((job_id, wait_ms)) = queue
        .queue
        .oldest_waiting_job
        .zip(queue.queue.oldest_waiting_millis)
    {
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
    // them.
    if queue.queue.unmatched > 0 {
        let unmatched = queue.queue.unmatched;
        let one = unmatched == 1;
        let job = if one { "job" } else { "jobs" };
        let asks = if one { "asks" } else { "ask" };
        let matches_ = if one { "matches" } else { "match" };
        let detail = if queue.queue.unavailable == 0 {
            format!(
                "{unmatched} queued {job} {asks} for labels no runner pool offers; {} will wait until one does.",
                if one { "it" } else { "they" }
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
    /// (`counts` instead of `live_counts`, `state!='released'` on the
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
    /// writes: it never sets `current_job_id`. It counts once, as `other`
    /// (its job is no `workflow_jobs` row), and not while it is online.
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
            BTreeMap::from([(JobCategory::Test, 1), (JobCategory::Other, 1)]),
            "the GitHub job, and the busy GitLab runner's"
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
            BTreeMap::from([(JobCategory::Test, 1), (JobCategory::Other, 1)]),
        );
        seeded.close().await
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
}
