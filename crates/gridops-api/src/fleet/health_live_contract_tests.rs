//! Contract tests for #201 (live machine stats and live concerns), written
//! by #197.
//!
//! #201 removes the `#[ignore = "contract: #201"]` markers and makes these
//! pass; it changes nothing else here (`Fleet health guard`). The rules are
//! #201's live-concern table as `docs/design/fleet-health.md` records it:
//! - `host-offline:<host>` critical: a machine with a CPU budget above 0 has
//!   no `host_samples` row in the last 5 minutes; `since` is its last sample;
//! - `start-gate:<host>` warning: `start_health::start_gate` sets it aside;
//! - `disk:<host>` critical or warning: `disk::alarm_for` on the root disk;
//! - `unmatched-jobs` critical: queued jobs no pool can run;
//! - `oldest-queued` critical over 10 min, warning over 5 min;
//! - `uncertain:<host>` warning: the machine has `uncertain` placements.
//!
//! Every test seeds its own database with #197's `testdata` (five machines,
//! each with a fresh sample unless the test says otherwise) and calls
//! `collect_live(db, now)`. Mounted from `fleet/mod.rs` (#197-owned).

use std::collections::BTreeMap;

use anyhow::{Context as _, Result};
use gridops_core::{
    fleet::failure::PlacementFailureReason,
    fleet_health::{
        Concern, ConcernSubject, JobCategory, Severity, speed_check,
        testdata::{self, JobSeed, PlacementEnd, SampleSeed, SeededFleet},
    },
};
use sqlx::SqlitePool;

use super::health_live::collect_live;

const NOW: i64 = 1_800_000_000_000;
const SECOND: i64 = 1_000;
const MINUTE: i64 = 60 * SECOND;
const GIB: i64 = 1024 * 1024 * 1024;

struct Seeded {
    directory: std::path::PathBuf,
    db: SqlitePool,
    fleet: SeededFleet,
}

impl Seeded {
    /// The seeded fleet, every machine with a CPU budget reporting a sample
    /// 30 s ago with half its 100 GiB root disk free.
    async fn new() -> Result<Self> {
        let directory = std::env::temp_dir().join(format!("gridops-live-{}", uuid::Uuid::new_v4()));
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

    async fn concerns(&self) -> Result<Vec<Concern>> {
        Ok(collect_live(&self.db, NOW).await?.2)
    }

    async fn close(self) -> Result<()> {
        self.db.close().await;
        std::fs::remove_dir_all(self.directory)?;
        Ok(())
    }
}

fn find<'a>(concerns: &'a [Concern], key: &str) -> Option<&'a Concern> {
    concerns.iter().find(|concern| concern.key == key)
}

fn keys_starting(concerns: &[Concern], prefix: &str) -> Vec<String> {
    concerns
        .iter()
        .filter(|concern| concern.key.starts_with(prefix))
        .map(|concern| concern.key.clone())
        .collect()
}

fn assert_readable(concern: &Concern) {
    assert!(!concern.title.trim().is_empty(), "{concern:?}");
    assert!(!concern.detail.trim().is_empty(), "{concern:?}");
    assert!(
        concern
            .link
            .as_deref()
            .is_some_and(|link| link.starts_with('/')),
        "{concern:?} links somewhere in the app"
    );
}

#[tokio::test]
#[ignore = "contract: #201"]
async fn a_machine_silent_for_6_minutes_is_offline_since_its_last_sample() -> Result<()> {
    let seeded = Seeded::new().await?;
    let io = seeded.id("io")?;
    let jupiter = seeded.id("Jupiter")?;
    // io's newest sample is 6 minutes old; Jupiter's is 4 min 59 s old.
    let last = NOW - 6 * MINUTE;
    sqlx::query("DELETE FROM host_samples WHERE host_id IN (?,?)")
        .bind(&io)
        .bind(&jupiter)
        .execute(&seeded.db)
        .await?;
    seeded.sample("io", last - MINUTE, 50 * GIB).await?;
    seeded.sample("io", last, 50 * GIB).await?;
    seeded
        .sample("Jupiter", NOW - 5 * MINUTE + SECOND, 50 * GIB)
        .await?;

    let (_, hosts, concerns) = collect_live(&seeded.db, NOW).await?;
    let offline = find(&concerns, &format!("host-offline:{io}"))
        .with_context(|| format!("io is offline: {concerns:?}"))?;
    assert_eq!(offline.severity, Severity::Critical);
    assert_eq!(offline.since, Some(last), "since is the last sample");
    assert_eq!(offline.subject, ConcernSubject::Host(io.clone()));
    assert_readable(offline);
    assert_eq!(
        keys_starting(&concerns, "host-offline:"),
        [format!("host-offline:{io}")],
        "Jupiter (4 min 59 s) and the budget-0 machine are not offline"
    );
    let stats = |id: &str| hosts.iter().find(|stats| stats.host_id == id);
    let io_stats = stats(&io).context("io has live stats")?;
    assert!(!io_stats.online);
    assert_eq!(io_stats.last_seen_at, Some(last));
    let jupiter_stats = stats(&jupiter).context("Jupiter has live stats")?;
    assert!(jupiter_stats.online);
    seeded.close().await
}

#[tokio::test]
#[ignore = "contract: #201"]
async fn a_machine_with_no_cpu_budget_never_raises_host_offline() -> Result<()> {
    let seeded = Seeded::new().await?;
    let laptop = seeded.fleet.host("MacBookPro")?;
    assert_eq!(laptop.cpu_budget_millis, 0, "the seeded budget-0 machine");
    let laptop_id = laptop.id.clone();
    // No samples ever, and an old one: neither is a concern for it.
    let concerns = seeded.concerns().await?;
    assert!(find(&concerns, &format!("host-offline:{laptop_id}")).is_none());
    seeded
        .sample("MacBookPro", NOW - 60 * MINUTE, 50 * GIB)
        .await?;
    // The control: io (budgeted) silent just as long is offline in the same
    // call, so an empty concern list can't pass this test.
    let io = seeded.id("io")?;
    sqlx::query("DELETE FROM host_samples WHERE host_id = ?")
        .bind(&io)
        .execute(&seeded.db)
        .await?;
    seeded.sample("io", NOW - 60 * MINUTE, 50 * GIB).await?;
    let concerns = seeded.concerns().await?;
    assert!(
        find(&concerns, &format!("host-offline:{laptop_id}")).is_none(),
        "{concerns:?}"
    );
    assert!(
        find(&concerns, &format!("host-offline:{io}")).is_some(),
        "io, budgeted and silent 60 min, is offline: {concerns:?}"
    );
    seeded.close().await
}

#[tokio::test]
#[ignore = "contract: #201"]
async fn three_start_failures_set_the_start_gate_concern_and_two_do_not() -> Result<()> {
    let seeded = Seeded::new().await?;
    let europa = seeded.fleet.host("europa")?.clone();
    let callisto = seeded.fleet.host("callisto")?.clone();
    for minutes in [3, 6, 9] {
        testdata::insert_placement_failure(
            &seeded.db,
            &europa,
            NOW - minutes * MINUTE,
            PlacementFailureReason::StartDeadlineExpired,
            None,
        )
        .await?;
    }
    for minutes in [3, 6] {
        testdata::insert_placement_failure(
            &seeded.db,
            &callisto,
            NOW - minutes * MINUTE,
            PlacementFailureReason::StartDeadlineExpired,
            None,
        )
        .await?;
    }
    let (_, hosts, concerns) = collect_live(&seeded.db, NOW).await?;
    let gate = find(&concerns, &format!("start-gate:{}", europa.id))
        .with_context(|| format!("europa is gated: {concerns:?}"))?;
    assert_eq!(gate.severity, Severity::Warning);
    assert_readable(gate);
    assert_eq!(
        keys_starting(&concerns, "start-gate:"),
        [format!("start-gate:{}", europa.id)],
        "two failures on callisto are not a gate"
    );
    let failures = |id: &str| {
        hosts
            .iter()
            .find(|stats| stats.host_id == id)
            .map(|stats| stats.start_failures_last_hour)
    };
    assert_eq!(failures(&europa.id), Some(3));
    assert_eq!(failures(&callisto.id), Some(2));
    seeded.close().await
}

#[tokio::test]
#[ignore = "contract: #201"]
async fn the_root_disk_alarm_follows_disk_alarm_for_at_each_threshold() -> Result<()> {
    // Every seeded root disk is 100 GiB: critical below 5 GiB free, warning
    // below 10 GiB (`disk::alarm_for`).
    assert_eq!(testdata::DISK_BYTES, 100 * GIB);
    for (free, expected) in [
        (5 * GIB - 1, Some(Severity::Critical)),
        (5 * GIB, Some(Severity::Warning)),
        (10 * GIB - 1, Some(Severity::Warning)),
        (10 * GIB, None),
    ] {
        let seeded = Seeded::new().await?;
        let io = seeded.id("io")?;
        seeded.sample("io", NOW - 10 * SECOND, free).await?;
        let concerns = seeded.concerns().await?;
        let disk = find(&concerns, &format!("disk:{io}"));
        assert_eq!(
            disk.map(|concern| concern.severity),
            expected,
            "{free} bytes free: {concerns:?}"
        );
        if let Some(disk) = disk {
            assert_readable(disk);
        }
        assert_eq!(
            keys_starting(&concerns, "disk:").len(),
            usize::from(expected.is_some()),
            "only io's disk is low"
        );
        seeded.close().await?;
    }
    Ok(())
}

#[tokio::test]
#[ignore = "contract: #201"]
async fn a_queued_job_no_pool_can_run_is_an_unmatched_jobs_concern() -> Result<()> {
    let seeded = Seeded::new().await?;
    let queued = JobSeed::queued(1, "unit tests", NOW - 30 * SECOND);
    testdata::insert_jobs(&seeded.db, &seeded.fleet, &[queued]).await?;
    let concerns = seeded.concerns().await?;
    assert!(
        find(&concerns, "unmatched-jobs").is_none(),
        "a job the seeded pool runs is matched: {concerns:?}"
    );

    let mut gpu = JobSeed::queued(2, "train", NOW - 20 * SECOND);
    gpu.labels = vec!["self-hosted".to_owned(), "gpu".to_owned()];
    testdata::insert_jobs(&seeded.db, &seeded.fleet, &[gpu]).await?;
    let (fleet, _, concerns) = collect_live(&seeded.db, NOW).await?;
    let unmatched = find(&concerns, "unmatched-jobs")
        .with_context(|| format!("one unmatched job: {concerns:?}"))?;
    assert_eq!(unmatched.severity, Severity::Critical);
    assert_eq!(unmatched.subject, ConcernSubject::Fleet);
    assert_readable(unmatched);
    assert_eq!(fleet.queued_total, 2);
    seeded.close().await
}

#[tokio::test]
#[ignore = "contract: #201"]
async fn the_oldest_queued_job_warns_over_5_minutes_and_is_critical_over_10() -> Result<()> {
    for (wait, expected) in [
        (5 * MINUTE, None),
        (5 * MINUTE + SECOND, Some(Severity::Warning)),
        (10 * MINUTE, Some(Severity::Warning)),
        (10 * MINUTE + SECOND, Some(Severity::Critical)),
    ] {
        let seeded = Seeded::new().await?;
        let jobs = [
            JobSeed::queued(1, "integration tests", NOW - wait),
            JobSeed::queued(2, "lint", NOW - 10 * SECOND),
        ];
        testdata::insert_jobs(&seeded.db, &seeded.fleet, &jobs).await?;
        let concerns = seeded.concerns().await?;
        let oldest = find(&concerns, "oldest-queued");
        assert_eq!(
            oldest.map(|concern| concern.severity),
            expected,
            "waited {wait} ms: {concerns:?}"
        );
        if let Some(oldest) = oldest {
            assert_readable(oldest);
            assert!(
                oldest.detail.contains("integration tests")
                    && oldest.detail.contains(&seeded.fleet.repository),
                "the detail names the job and the repository: {}",
                oldest.detail
            );
        }
        seeded.close().await?;
    }
    Ok(())
}

#[tokio::test]
#[ignore = "contract: #201"]
async fn a_machine_with_an_uncertain_placement_is_a_warning() -> Result<()> {
    let seeded = Seeded::new().await?;
    let europa = seeded.id("europa")?;
    let callisto = seeded.id("callisto")?;
    let mut uncertain = JobSeed::completed(
        1,
        "unit tests",
        Some(&europa),
        NOW - 9 * MINUTE,
        NOW - 8 * MINUTE,
        NOW - MINUTE,
    );
    uncertain.placement = PlacementEnd::Uncertain;
    let stopped = JobSeed::completed(
        2,
        "unit tests",
        Some(&callisto),
        NOW - 9 * MINUTE,
        NOW - 8 * MINUTE,
        NOW - MINUTE,
    );
    testdata::insert_jobs(&seeded.db, &seeded.fleet, &[uncertain, stopped]).await?;
    let concerns = seeded.concerns().await?;
    let concern = find(&concerns, &format!("uncertain:{europa}"))
        .with_context(|| format!("europa has an uncertain placement: {concerns:?}"))?;
    assert_eq!(concern.severity, Severity::Warning);
    assert_readable(concern);
    assert_eq!(
        keys_starting(&concerns, "uncertain:"),
        [format!("uncertain:{europa}")],
        "a stopped placement is not uncertain"
    );
    seeded.close().await
}

#[tokio::test]
#[ignore = "contract: #201"]
async fn the_running_counts_by_category_add_up_to_the_runners_running_now() -> Result<()> {
    let seeded = Seeded::new().await?;
    let io = seeded.id("io")?;
    let jupiter = seeded.id("Jupiter")?;
    let running = |id: i64, name: &str, host: &str, category: Option<JobCategory>| {
        let mut job = JobSeed::running(id, name, Some(host), NOW - 3 * MINUTE, NOW - 2 * MINUTE);
        job.category = category;
        job
    };
    let jobs = [
        running(1, "unit tests", &io, Some(JobCategory::Test)),
        running(2, "e2e", &io, Some(JobCategory::Test)),
        running(3, "docker image", &io, Some(JobCategory::Build)),
        running(4, "misc", &jupiter, None),
        running(5, "lint", &jupiter, Some(JobCategory::Check)),
        // Finished: its runner is not running anything now.
        JobSeed::completed(
            6,
            "unit tests",
            Some(&jupiter),
            NOW - 9 * MINUTE,
            NOW - 8 * MINUTE,
            NOW - MINUTE,
        ),
    ];
    testdata::insert_jobs(&seeded.db, &seeded.fleet, &jobs).await?;
    let busy_runners: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM runners WHERE current_job_id IS NOT NULL")
            .fetch_one(&seeded.db)
            .await?;
    assert_eq!(busy_runners, 5, "the seed: five runners running a job");

    let (fleet, hosts, _) = collect_live(&seeded.db, NOW).await?;
    let by_host = |id: &str| -> BTreeMap<JobCategory, u32> {
        hosts
            .iter()
            .find(|stats| stats.host_id == id)
            .map(|stats| stats.running_by_category.clone())
            .unwrap_or_default()
            .into_iter()
            .filter(|(_, count)| *count > 0)
            .collect()
    };
    assert_eq!(
        by_host(&io),
        BTreeMap::from([(JobCategory::Test, 2), (JobCategory::Build, 1)])
    );
    assert_eq!(
        by_host(&jupiter),
        BTreeMap::from([(JobCategory::Check, 1), (JobCategory::Other, 1)]),
        "NULL is counted as other"
    );
    let per_host: u32 = hosts
        .iter()
        .flat_map(|stats| stats.running_by_category.values())
        .sum();
    let fleet_sum: u32 = fleet.running_by_category.values().sum();
    assert_eq!(i64::from(per_host), busy_runners);
    assert_eq!(fleet_sum, per_host);
    assert_eq!(fleet.running_total, per_host);
    seeded.close().await
}

/// #197 section 7: the contract tests for #201 run on realistic volume, so CI
/// enforces the #179 lock rules on the real reads and not just in
/// instructions.
///
/// The synthetic week is 50,400 finished jobs and a sample every 30 s per
/// machine. On top of it, 7 jobs are running on two machines and 40 are
/// queued; the live read must count exactly those, by category, and finish
/// with no statement over 100 ms, with all the finished jobs behind it.
#[tokio::test]
#[ignore = "contract: #201"]
async fn the_live_read_over_a_week_of_jobs_counts_the_running_and_queued_with_no_slow_statement()
-> Result<()> {
    let directory =
        std::env::temp_dir().join(format!("gridops-live-week-{}", uuid::Uuid::new_v4()));
    let db = gridops_core::connect_database_path(&directory.join("db.sqlite")).await?;
    let week = testdata::build_week(&db, NOW).await?;
    assert!(week.jobs >= 50_000, "{} jobs", week.jobs);
    let io = week.fleet.host("io")?.id.clone();
    let jupiter = week.fleet.host("Jupiter")?.id.clone();

    let mut jobs = Vec::new();
    let mut next_id = 1_000_000;
    let mut running = |name: &str, host: &str, category: Option<JobCategory>| {
        next_id += 1;
        let mut job = JobSeed::running(
            next_id,
            name,
            Some(host),
            NOW - 3 * MINUTE,
            NOW - 2 * MINUTE,
        );
        job.category = category;
        job
    };
    for _ in 0..3 {
        jobs.push(running("unit tests", &io, Some(JobCategory::Test)));
    }
    for _ in 0..2 {
        jobs.push(running("docker image", &io, Some(JobCategory::Build)));
    }
    jobs.push(running("lint", &jupiter, Some(JobCategory::Check)));
    jobs.push(running("misc", &jupiter, None));
    // 40 waiting for a runner for under 3 minutes: 20 test, 10 build, 10 uncategorized.
    for index in 0..40_i64 {
        let mut job = JobSeed::queued(
            2_000_000 + index,
            "queued job",
            NOW - MINUTE - index * 1_000,
        );
        job.category = match index % 4 {
            0 | 1 => Some(JobCategory::Test),
            2 => Some(JobCategory::Build),
            _ => None,
        };
        jobs.push(job);
    }
    testdata::insert_jobs(&db, &week.fleet, &jobs).await?;

    let (fleet, hosts, _) = speed_check::check(&db, collect_live(&db, NOW)).await?;

    assert_eq!(
        fleet.running_total, 7,
        "the seeded running jobs, no finished one"
    );
    assert_eq!(
        fleet.running_by_category.values().sum::<u32>(),
        fleet.running_total,
        "the categories add up to the total"
    );
    assert_eq!(fleet.queued_total, 40);
    assert_eq!(
        fleet.queued_by_category.values().sum::<u32>(),
        fleet.queued_total,
        "NULL is counted as other, so the categories add up to the total"
    );
    let io_running: u32 = hosts
        .iter()
        .filter(|stats| stats.host_id == io)
        .flat_map(|stats| stats.running_by_category.values())
        .sum();
    assert_eq!(io_running, 5, "io's five running jobs");
    db.close().await;
    std::fs::remove_dir_all(directory)?;
    Ok(())
}
