//! Contract tests for #193 (the relative speed index), written by #197.
//!
//! #193 removes the `#[ignore = "contract: #193"]` markers and makes these
//! pass; it changes nothing else here (`Fleet health guard`). The rules are
//! #193's "Work" list as `docs/design/fleet-health.md` records them:
//! - job key = repository + workflow name + job name, shard suffix stripped;
//! - per key, the fleet median is the median duration of its successful runs
//!   on every machine; each (machine, runner size) has its own median;
//! - the index is the run-count-weighted geometric mean of (own median ÷
//!   fleet median) over keys with at least 5 successful runs on the machine
//!   at that size and at least 5 on the rest of the fleet;
//! - fewer than 3 qualifying keys is "not enough data" (`index: None`).
//!
//! Mounted from `fleet_health/mod.rs` (#197-owned). The jobs are seeded with
//! #197's `testdata`.

use anyhow::Result;
use sqlx::SqlitePool;

use super::{
    HealthWindow, SpeedResult, speed, speed_check,
    testdata::{self, JobSeed, SeededFleet},
};

const HOUR: i64 = 3_600_000;
/// The end of the window every test reads.
const NOW: i64 = 1_800_000_000_000;

struct Harness {
    directory: std::path::PathBuf,
    db: SqlitePool,
    fleet: SeededFleet,
    next_id: i64,
}

impl Harness {
    async fn new() -> Result<Self> {
        let directory =
            std::env::temp_dir().join(format!("gridops-speed-{}", uuid::Uuid::new_v4()));
        let db = crate::connect_database_path(&directory.join("db.sqlite")).await?;
        let fleet = testdata::seed_fleet(&db).await?;
        Ok(Self {
            directory,
            db,
            fleet,
            next_id: 1,
        })
    }

    /// `runs` successful runs of `name` on `host` at `cpu_millis`, each
    /// lasting `duration` ms, finishing inside the last day.
    async fn runs(
        &mut self,
        host: &str,
        cpu_millis: u32,
        name: &str,
        runs: usize,
        duration: i64,
    ) -> Result<()> {
        let host_id = self.fleet.host(host)?.id.clone();
        let mut jobs = Vec::new();
        for _ in 0..runs {
            let id = self.next_id;
            self.next_id += 1;
            let completed = NOW - HOUR - id * 1_000;
            let mut job = JobSeed::completed(
                id,
                name,
                Some(&host_id),
                completed - duration - 5_000,
                completed - duration,
                completed,
            );
            job.cpu_millis = cpu_millis;
            jobs.push(job);
        }
        testdata::insert_jobs(&self.db, &self.fleet, &jobs).await
    }

    async fn indexes(&self) -> Result<Vec<SpeedResult>> {
        speed::speed_indexes(&self.db, HealthWindow::Day, NOW).await
    }

    async fn close(self) -> Result<()> {
        self.db.close().await;
        std::fs::remove_dir_all(self.directory)?;
        Ok(())
    }
}

fn result<'a>(
    results: &'a [SpeedResult],
    fleet: &SeededFleet,
    host: &str,
    cpu_millis: u32,
) -> Result<&'a SpeedResult> {
    let id = &fleet.host(host)?.id;
    results
        .iter()
        .find(|result| &result.host_id == id && result.cpu_millis == cpu_millis)
        .ok_or_else(|| anyhow::anyhow!("no speed result for {host} at {cpu_millis}"))
}

/// Durations (ms) of the four keys every test runs.
const KEYS: [(&str, i64); 4] = [
    ("unit", 120_000),
    ("lint", 60_000),
    ("e2e", 600_000),
    ("build", 300_000),
];

/// `io`, `europa` and `callisto` run every key 6 times at its typical
/// duration at 2 CPUs; `Jupiter` runs every key 6 times at `factor` × that.
async fn typical_fleet(harness: &mut Harness, factor: f64) -> Result<()> {
    for (name, duration) in KEYS {
        for host in ["io", "europa", "callisto"] {
            harness.runs(host, 2_000, name, 6, duration).await?;
        }
        harness
            .runs("Jupiter", 2_000, name, 6, (duration as f64 * factor) as i64)
            .await?;
    }
    Ok(())
}

#[test]
fn each_shard_suffix_form_normalizes_to_one_key() {
    let plain = speed::job_key("acme/app", "CI", "unit");
    for sharded in [
        "unit (1/4)",
        "unit [2/4]",
        "unit 3/4",
        "unit (12/16) ",
        "  unit [1/2]",
    ] {
        assert_eq!(
            speed::job_key("acme/app", "CI", sharded),
            plain,
            "{sharded:?}"
        );
    }
    // Only a trailing shard counter is stripped.
    assert_ne!(speed::job_key("acme/app", "CI", "unit 3/4 extra"), plain);
    assert_ne!(speed::job_key("acme/app", "CI", "unit (a/b)"), plain);
    // Repository and workflow are part of the key.
    assert_ne!(speed::job_key("acme/other", "CI", "unit"), plain);
    assert_ne!(speed::job_key("acme/app", "Nightly", "unit"), plain);
}

#[tokio::test]
async fn a_machine_running_every_key_at_one_and_a_half_times_gets_1_5() -> Result<()> {
    let mut harness = Harness::new().await?;
    typical_fleet(&mut harness, 1.5).await?;
    let results = harness.indexes().await?;
    let slow = result(&results, &harness.fleet, "Jupiter", 2_000)?;
    let index = slow
        .index
        .ok_or_else(|| anyhow::anyhow!("Jupiter has no index"))?;
    assert!((index - 1.5).abs() <= 0.01, "Jupiter's index is {index}");
    for host in ["io", "europa", "callisto"] {
        let typical = result(&results, &harness.fleet, host, 2_000)?
            .index
            .ok_or_else(|| anyhow::anyhow!("{host} has no index"))?;
        assert!((typical - 1.0).abs() <= 0.01, "{host}'s index is {typical}");
    }
    // The keys behind it, at most 5, with both medians and both counts.
    assert!(!slow.top_keys.is_empty() && slow.top_keys.len() <= 5);
    let e2e = slow
        .top_keys
        .iter()
        .find(|key| key.key == speed::job_key("acme/app", "CI", "e2e"))
        .ok_or_else(|| anyhow::anyhow!("e2e is not among Jupiter's keys"))?;
    assert_eq!(e2e.host_median_ms, 900_000);
    assert_eq!(e2e.fleet_median_ms, 600_000);
    assert_eq!(e2e.host_runs, 6);
    assert!(e2e.fleet_runs >= 18);
    harness.close().await
}

#[tokio::test]
async fn keys_under_the_five_run_minimum_are_ignored() -> Result<()> {
    let mut harness = Harness::new().await?;
    typical_fleet(&mut harness, 1.5).await?;
    // Four runs of a fifth key at 10x on Jupiter: under the minimum there.
    for host in ["io", "europa", "callisto"] {
        harness.runs(host, 2_000, "docs", 6, 30_000).await?;
    }
    harness.runs("Jupiter", 2_000, "docs", 4, 300_000).await?;
    // And a key the rest of the fleet ran only 4 times in total.
    harness.runs("io", 2_000, "rare", 4, 30_000).await?;
    harness.runs("Jupiter", 2_000, "rare", 6, 300_000).await?;
    let results = harness.indexes().await?;
    let slow = result(&results, &harness.fleet, "Jupiter", 2_000)?;
    let index = slow
        .index
        .ok_or_else(|| anyhow::anyhow!("Jupiter has no index"))?;
    assert!((index - 1.5).abs() <= 0.01, "Jupiter's index is {index}");
    for ignored in ["docs", "rare"] {
        let key = speed::job_key("acme/app", "CI", ignored);
        assert!(
            slow.top_keys.iter().all(|top| top.key != key),
            "{ignored} must not count"
        );
    }
    harness.close().await
}

#[tokio::test]
async fn fewer_than_three_qualifying_keys_is_not_enough_data() -> Result<()> {
    let mut harness = Harness::new().await?;
    typical_fleet(&mut harness, 1.0).await?;
    // europa at 4 CPUs ran only two keys enough times.
    harness.runs("europa", 4_000, "unit", 6, 100_000).await?;
    harness.runs("europa", 4_000, "lint", 6, 50_000).await?;
    harness.runs("europa", 4_000, "e2e", 3, 500_000).await?;
    let results = harness.indexes().await?;
    assert_eq!(
        result(&results, &harness.fleet, "europa", 4_000)?.index,
        None,
        "two keys give no index"
    );
    assert!(
        result(&results, &harness.fleet, "europa", 2_000)?
            .index
            .is_some()
    );
    harness.close().await
}

#[tokio::test]
async fn the_same_machine_at_two_runner_sizes_gets_two_indexes() -> Result<()> {
    let mut harness = Harness::new().await?;
    typical_fleet(&mut harness, 1.5).await?;
    // io also runs every key at 1.5 CPUs, twice as slow as typical.
    for (name, duration) in KEYS {
        harness.runs("io", 1_500, name, 6, duration * 2).await?;
    }
    let results = harness.indexes().await?;
    let io_sizes: Vec<u32> = {
        let io = &harness.fleet.host("io")?.id;
        let mut sizes: Vec<u32> = results
            .iter()
            .filter(|result| &result.host_id == io)
            .map(|result| result.cpu_millis)
            .collect();
        sizes.sort_unstable();
        sizes
    };
    assert_eq!(io_sizes, [1_500, 2_000]);
    let small = result(&results, &harness.fleet, "io", 1_500)?
        .index
        .ok_or_else(|| anyhow::anyhow!("io at 1.5 CPU has no index"))?;
    let normal = result(&results, &harness.fleet, "io", 2_000)?
        .index
        .ok_or_else(|| anyhow::anyhow!("io at 2 CPU has no index"))?;
    assert!((small - 2.0).abs() <= 0.01, "io at 1.5 CPU: {small}");
    assert!((normal - 1.0).abs() <= 0.01, "io at 2 CPU: {normal}");
    harness.close().await
}

#[tokio::test]
async fn only_successful_runs_inside_the_window_count_and_shards_compare_together() -> Result<()> {
    let mut harness = Harness::new().await?;
    typical_fleet(&mut harness, 1.0).await?;
    let jupiter = harness.fleet.host("Jupiter")?.id.clone();
    let mut extra = Vec::new();
    // Failed runs and runs outside the last day, all very slow: ignored.
    for id in 10_000..10_010 {
        let mut failed = JobSeed::completed(
            id,
            "unit",
            Some(&jupiter),
            NOW - 2 * HOUR,
            NOW - 2 * HOUR,
            NOW - HOUR,
        );
        failed.conclusion = Some("failure".to_owned());
        extra.push(failed);
        let old = NOW - 2 * 86_400_000;
        extra.push(JobSeed::completed(
            id + 100,
            "lint",
            Some(&jupiter),
            old - 9_000_000,
            old - 8_000_000,
            old,
        ));
    }
    // Sharded runs of "build" on callisto, 2 of each form: one key, 6 runs.
    for (offset, name) in ["build (1/3)", "build [2/3]", "build 3/3"]
        .iter()
        .cycle()
        .take(6)
        .enumerate()
    {
        let offset = i64::try_from(offset)?;
        let completed = NOW - 3 * HOUR - offset * 1_000;
        let mut shard = JobSeed::completed(
            20_000 + offset,
            name,
            Some(&harness.fleet.host("callisto")?.id),
            completed - 400_000,
            completed - 300_000,
            completed,
        );
        shard.cpu_millis = 3_000;
        extra.push(shard);
    }
    testdata::insert_jobs(&harness.db, &harness.fleet, &extra).await?;
    harness.runs("callisto", 3_000, "unit", 6, 120_000).await?;
    harness.runs("callisto", 3_000, "lint", 6, 60_000).await?;
    let results = harness.indexes().await?;
    let index = result(&results, &harness.fleet, "Jupiter", 2_000)?
        .index
        .ok_or_else(|| anyhow::anyhow!("Jupiter has no index"))?;
    assert!(
        (index - 1.0).abs() <= 0.01,
        "failed and old runs leaked in: {index}"
    );
    let shards = result(&results, &harness.fleet, "callisto", 3_000)?;
    assert!(
        shards.index.is_some(),
        "three keys at 3 CPUs, one of them from six shard runs"
    );
    harness.close().await
}

/// #197 section 7: the contract tests for #193 run on realistic volume, so CI
/// enforces the #179 lock rules on the real read and not just in instructions.
///
/// The synthetic week is 50,400 successful jobs over the four budgeted
/// machines, every machine at both runner sizes (2 and 3 CPUs) on all of the
/// week's keys, with durations within 25% of typical. Jupiter is then made
/// 1.5 times slower on every job. The index read for the whole week must
/// finish with no statement over 100 ms, and must tell Jupiter apart.
#[tokio::test]
async fn a_week_of_50000_jobs_finds_the_slow_machine_with_no_statement_over_100_ms() -> Result<()> {
    let directory =
        std::env::temp_dir().join(format!("gridops-speed-week-{}", uuid::Uuid::new_v4()));
    let db = crate::connect_database_path(&directory.join("db.sqlite")).await?;
    let week = testdata::build_week(&db, NOW).await?;
    assert!(week.jobs >= 50_000, "{} jobs", week.jobs);
    let jupiter = week.fleet.host("Jupiter")?.id.clone();
    // Jupiter's jobs all start 1.5 times as long before they complete.
    let slowed = sqlx::query(
        "UPDATE workflow_jobs SET started_at = completed_at - (completed_at - started_at) * 3 / 2
         WHERE runner_name IN (
           SELECT r.name FROM runners r
           JOIN workload_placements p ON p.runner_id = r.id
           WHERE p.host_id = ?)",
    )
    .bind(&jupiter)
    .execute(&db)
    .await?;
    assert!(
        slowed.rows_affected() > 10_000,
        "the week gives Jupiter a quarter of its jobs, not {}",
        slowed.rows_affected()
    );

    let results =
        speed_check::check(&db, speed::speed_indexes(&db, HealthWindow::Week, NOW)).await?;

    for size in [2_000, 3_000] {
        let slow = result(&results, &week.fleet, "Jupiter", size)?
            .index
            .ok_or_else(|| anyhow::anyhow!("Jupiter at {size} has no index"))?;
        assert!(
            slow > 1.2,
            "Jupiter at {size} runs 1.5x slower than the rest, its index is {slow}"
        );
        for host in ["io", "europa", "callisto"] {
            let typical = result(&results, &week.fleet, host, size)?
                .index
                .ok_or_else(|| anyhow::anyhow!("{host} at {size} has no index"))?;
            assert!(
                typical < 1.1 && typical < slow,
                "{host} at {size} is not slow: {typical} against Jupiter's {slow}"
            );
        }
    }
    db.close().await;
    std::fs::remove_dir_all(directory)?;
    Ok(())
}
