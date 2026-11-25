//! Contract tests for #191 (the hourly job rollup), written by #197.
//!
//! #191 removes the `#[ignore = "contract: #191"]` markers and makes these
//! pass; it changes nothing else here (`Fleet health guard`). The rules are
//! #191's "Work" list as `docs/design/fleet-health.md` records them:
//! - a job belongs to the hour its `completed_at` falls in (`[hour, hour +
//!   1 h)`), and only the final attempt of a run counts;
//! - its machine comes from `workflow_jobs.runner_id` →
//!   `runners.github_runner_id` (GitLab: `runners.gitlab_runner_id`) →
//!   `workload_placements`; no placement is `host_id = pool_id =
//!   'unmanaged'` with no CPU-seconds;
//! - per (hour, host, pool, category; NULL is `other`): `job_count` every
//!   job, `completed_count` those that succeeded, `infra_failure_count`
//!   those whose placement ended `failed` or `uncertain` (a failing test is
//!   not one), summed duration and CPU-seconds (`cpu_millis / 1000 ×
//!   seconds`), and the wait (`started_at − created_at`) and duration
//!   histograms in #197's buckets;
//! - `rollup_hour(db, hour, now)` writes one hour (idempotently, and takes
//!   the hour off `fleet_health_recompute`); `run_pass(db, now)` rolls up
//!   listed hours and catches up the last 7 days (one hour per pass at
//!   least; several under a time budget since #204).
//!
//! Mounted from `main.rs` (#197-owned). Jobs are seeded with #197's
//! `testdata`.

use super::*;
use gridops_core::{
    connect_database_path,
    fleet_health::{
        JobCategory, speed_check,
        testdata::{self, JobSeed, PlacementEnd, SeededFleet},
    },
};

const MINUTE: i64 = 60_000;
const HOUR: i64 = 3_600_000;
/// A whole hour; every test reads as of this time.
const NOW: i64 = 1_800_000_000_000;
/// The hour the tests roll up: finished two hours before `NOW`.
const H: i64 = NOW - 2 * HOUR;

/// A row of `fleet_health_hourly`, CPU-seconds in thousandths.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct Row {
    hour: i64,
    host_id: String,
    pool_id: String,
    category: String,
    job_count: i64,
    completed_count: i64,
    infra_failure_count: i64,
    duration_ms_sum: i64,
    cpu_millis_seconds: i64,
    wait: Vec<i64>,
    duration: Vec<i64>,
}

async fn rows(db: &SqlitePool, hour: i64) -> Result<Vec<Row>> {
    let raw = sqlx::query(
        "SELECT hour,host_id,pool_id,category,job_count,completed_count,infra_failure_count,
                duration_ms_sum,cpu_seconds_sum,queue_wait_histogram,duration_histogram
           FROM fleet_health_hourly WHERE hour=? ORDER BY host_id,pool_id,category",
    )
    .bind(hour)
    .fetch_all(db)
    .await?;
    let mut rows = Vec::new();
    for row in raw {
        rows.push(Row {
            hour: row.get("hour"),
            host_id: row.get("host_id"),
            pool_id: row.get("pool_id"),
            category: row.get("category"),
            job_count: row.get("job_count"),
            completed_count: row.get("completed_count"),
            infra_failure_count: row.get("infra_failure_count"),
            duration_ms_sum: row.get("duration_ms_sum"),
            cpu_millis_seconds: (row.get::<f64, _>("cpu_seconds_sum") * 1000.0).round() as i64,
            wait: serde_json::from_str(&row.get::<String, _>("queue_wait_histogram"))?,
            duration: serde_json::from_str(&row.get::<String, _>("duration_histogram"))?,
        });
    }
    rows.sort();
    Ok(rows)
}

/// A histogram with one job in each listed bucket.
fn buckets(indexes: &[usize]) -> Vec<i64> {
    let mut histogram = vec![0; 9];
    for index in indexes {
        histogram[*index] += 1;
    }
    histogram
}

struct Harness {
    directory: std::path::PathBuf,
    db: SqlitePool,
    fleet: SeededFleet,
}

impl Harness {
    async fn new() -> Result<Self> {
        let directory =
            std::env::temp_dir().join(format!("gridops-job-rollup-{}", uuid::Uuid::new_v4()));
        let db = connect_database_path(&directory.join("gridops.sqlite")).await?;
        let fleet = testdata::seed_fleet(&db).await?;
        Ok(Self {
            directory,
            db,
            fleet,
        })
    }

    fn id(&self, name: &str) -> Result<String> {
        Ok(self.fleet.host(name)?.id.clone())
    }

    async fn finish(self) -> Result<()> {
        self.db.close().await;
        std::fs::remove_dir_all(self.directory)?;
        Ok(())
    }
}

/// A finished job: created at `H + created`, waiting `wait`, running
/// `duration` (all ms), on `host` (`None`: a runner `GridOps` doesn't manage).
fn job(
    id: i64,
    name: &str,
    host: Option<&str>,
    category: Option<JobCategory>,
    created: i64,
    wait: i64,
    duration: i64,
) -> JobSeed {
    let created_at = H + created;
    let mut job = JobSeed::completed(
        id,
        name,
        host,
        created_at,
        created_at + wait,
        created_at + wait + duration,
    );
    job.category = category;
    job
}

/// The seed every row test uses, and the rows it must give for `H`.
async fn seed(harness: &Harness) -> Result<Vec<Row>> {
    let io = harness.id("io")?;
    let jupiter = harness.id("Jupiter")?;
    let europa = harness.id("europa")?;
    let callisto = harness.id("callisto")?;
    let pool = harness.fleet.pool_id.clone();
    let mut jobs = vec![
        // io, test: one success and one failing test (not an infra failure).
        job(
            1,
            "unit tests",
            Some(&io),
            Some(JobCategory::Test),
            10 * MINUTE,
            MINUTE,
            9 * MINUTE,
        ),
        {
            let mut failing = job(
                2,
                "unit tests",
                Some(&io),
                Some(JobCategory::Test),
                12 * MINUTE,
                20_000,
                2 * MINUTE,
            );
            failing.conclusion = Some("failure".to_owned());
            failing
        },
        // Jupiter, build: the placement failed, an infrastructure failure.
        {
            let mut failed = job(
                3,
                "docker image",
                Some(&jupiter),
                Some(JobCategory::Build),
                30 * MINUTE,
                5_000,
                MINUTE,
            );
            failed.conclusion = Some("failure".to_owned());
            failed.placement = PlacementEnd::Failed;
            failed
        },
        // A runner GridOps doesn't manage.
        job(
            4,
            "deploy",
            None,
            Some(JobCategory::Deploy),
            40 * MINUTE,
            MINUTE,
            4 * MINUTE,
        ),
        // europa, check: one completes 1 ms before the hour ends, one exactly
        // at the next hour (that one is not in H).
        job(
            5,
            "lint",
            Some(&europa),
            Some(JobCategory::Check),
            58 * MINUTE,
            MINUTE,
            MINUTE - 1,
        ),
        job(
            6,
            "lint",
            Some(&europa),
            Some(JobCategory::Check),
            58 * MINUTE,
            MINUTE,
            MINUTE,
        ),
        // io, category not set yet: counted as other.
        job(9, "misc", Some(&io), None, 50 * MINUTE, 10_000, MINUTE),
    ];
    // A re-run on callisto: attempt 1 failed, attempt 2 passed. Only the
    // final attempt counts.
    let mut first = job(
        7,
        "e2e",
        Some(&callisto),
        Some(JobCategory::Test),
        5 * MINUTE,
        MINUTE,
        10 * MINUTE,
    );
    first.run_id = 700;
    first.run_attempt = 1;
    first.conclusion = Some("failure".to_owned());
    let mut second = job(
        8,
        "e2e",
        Some(&callisto),
        Some(JobCategory::Test),
        22 * MINUTE,
        30_000,
        150_000,
    );
    second.run_id = 700;
    second.run_attempt = 2;
    jobs.extend([first, second]);
    // A GitLab job on io.
    let mut gitlab = job(
        10,
        "build",
        Some(&io),
        Some(JobCategory::Build),
        5 * MINUTE,
        MINUTE,
        10 * MINUTE,
    );
    gitlab.gitlab = true;
    jobs.push(gitlab);
    testdata::insert_jobs(&harness.db, &harness.fleet, &jobs).await?;

    let row = |host: &str,
               pool: &str,
               category: &str,
               counts: (i64, i64, i64),
               duration: i64,
               cpu: i64,
               wait: &[usize],
               durations: &[usize]| Row {
        hour: H,
        host_id: host.to_owned(),
        pool_id: pool.to_owned(),
        category: category.to_owned(),
        job_count: counts.0,
        completed_count: counts.1,
        infra_failure_count: counts.2,
        duration_ms_sum: duration,
        cpu_millis_seconds: cpu,
        wait: buckets(wait),
        duration: buckets(durations),
    };
    // Every managed job runs at the seed's 2,000 CPU millis: 2 CPU-seconds
    // per second. Buckets: 0–10 s, 10–30 s, 30–60 s, 1–2, 2–5, 5–10, 10–20,
    // 20–40, 40+ min, lower bound inclusive.
    let mut expected = vec![
        row(
            &io,
            &pool,
            "test",
            (2, 1, 0),
            660_000,
            1_320_000,
            &[3, 1],
            &[5, 4],
        ),
        row(
            &io,
            &pool,
            "build",
            (1, 1, 0),
            600_000,
            1_200_000,
            &[3],
            &[6],
        ),
        row(&io, &pool, "other", (1, 1, 0), 60_000, 120_000, &[1], &[3]),
        row(
            &jupiter,
            &pool,
            "build",
            (1, 0, 1),
            60_000,
            120_000,
            &[0],
            &[3],
        ),
        row(
            "unmanaged",
            "unmanaged",
            "deploy",
            (1, 1, 0),
            240_000,
            0,
            &[3],
            &[4],
        ),
        row(
            &europa,
            &pool,
            "check",
            (1, 1, 0),
            59_999,
            119_998,
            &[3],
            &[2],
        ),
        row(
            &callisto,
            &pool,
            "test",
            (1, 1, 0),
            150_000,
            300_000,
            &[2],
            &[4],
        ),
    ];
    expected.sort();
    Ok(expected)
}

#[tokio::test]
async fn an_hour_of_seeded_jobs_gives_the_expected_rows() -> Result<()> {
    let harness = Harness::new().await?;
    let expected = seed(&harness).await?;
    fleet_health_rollup::rollup_hour(&harness.db, H, NOW).await?;
    assert_eq!(rows(&harness.db, H).await?, expected);
    // The job completing exactly at the next hour is that hour's.
    fleet_health_rollup::rollup_hour(&harness.db, H + HOUR, NOW).await?;
    let next = rows(&harness.db, H + HOUR).await?;
    assert_eq!(
        next.iter()
            .map(|row| (row.category.as_str(), row.job_count))
            .collect::<Vec<_>>(),
        [("check", 1)]
    );
    harness.finish().await
}

#[tokio::test]
async fn rolling_an_hour_up_again_gives_identical_rows() -> Result<()> {
    let harness = Harness::new().await?;
    let expected = seed(&harness).await?;
    fleet_health_rollup::rollup_hour(&harness.db, H, NOW).await?;
    let first = sqlx::query_scalar::<_, String>(
        "SELECT json_group_array(json_array(hour,host_id,pool_id,category,job_count,completed_count,
                infra_failure_count,duration_ms_sum,cpu_seconds_sum,queue_wait_histogram,
                duration_histogram,computed_at))
           FROM (SELECT * FROM fleet_health_hourly ORDER BY host_id,pool_id,category)",
    )
    .fetch_one(&harness.db)
    .await?;
    fleet_health_rollup::rollup_hour(&harness.db, H, NOW).await?;
    let second = sqlx::query_scalar::<_, String>(
        "SELECT json_group_array(json_array(hour,host_id,pool_id,category,job_count,completed_count,
                infra_failure_count,duration_ms_sum,cpu_seconds_sum,queue_wait_histogram,
                duration_histogram,computed_at))
           FROM (SELECT * FROM fleet_health_hourly ORDER BY host_id,pool_id,category)",
    )
    .fetch_one(&harness.db)
    .await?;
    assert_eq!(first, second, "identical rows, none added");
    assert_eq!(rows(&harness.db, H).await?, expected);
    harness.finish().await
}

#[tokio::test]
async fn a_pass_rolls_up_a_listed_hour_and_catches_up_the_last_7_days() -> Result<()> {
    let harness = Harness::new().await?;
    let expected = seed(&harness).await?;
    // A job of three days ago, which only the catch-up reaches.
    let old_hour = NOW - 3 * 24 * HOUR;
    let io = harness.id("io")?;
    let mut old = JobSeed::completed(
        100,
        "unit tests",
        Some(&io),
        old_hour + MINUTE,
        old_hour + 2 * MINUTE,
        old_hour + 3 * MINUTE,
    );
    old.category = Some(JobCategory::Test);
    testdata::insert_jobs(&harness.db, &harness.fleet, &[old]).await?;
    // A listed hour is rolled up and taken off the list.
    sqlx::query("INSERT INTO fleet_health_recompute (hour,requested_at) VALUES (?,?)")
        .bind(H)
        .bind(NOW - MINUTE)
        .execute(&harness.db)
        .await?;
    fleet_health_rollup::run_pass(&harness.db, NOW).await?;
    assert_eq!(rows(&harness.db, H).await?, expected);
    let listed: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM fleet_health_recompute")
        .fetch_one(&harness.db)
        .await?;
    assert_eq!(listed, 0);

    // Catch-up (several hours a pass under its time budget since #204, one
    // at least): within 7 days' worth of passes the job of three days ago is
    // rolled up too.
    for _ in 0..(7 * 24 + 2) {
        fleet_health_rollup::run_pass(&harness.db, NOW).await?;
    }
    let old_rows = rows(&harness.db, old_hour).await?;
    assert_eq!(
        old_rows
            .iter()
            .map(|row| (row.host_id.as_str(), row.job_count))
            .collect::<Vec<_>>(),
        [(io.as_str(), 1)]
    );
    // Nothing older than 7 days, and no unfinished hour.
    let outside: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM fleet_health_hourly WHERE hour < ? OR hour + 3600000 > ?",
    )
    .bind(NOW - 7 * 24 * HOUR)
    .bind(NOW)
    .fetch_one(&harness.db)
    .await?;
    assert_eq!(outside, 0);
    harness.finish().await
}

#[tokio::test]
async fn seven_days_of_50000_jobs_roll_up_with_no_long_write_transaction() -> Result<()> {
    let directory =
        std::env::temp_dir().join(format!("gridops-job-rollup-week-{}", uuid::Uuid::new_v4()));
    let db = connect_database_path(&directory.join("gridops.sqlite")).await?;
    let week = testdata::build_week(&db, NOW).await?;
    speed_check::check(&db, async {
        for hour in (0..7 * 24).map(|index| NOW - 7 * 24 * HOUR + index * HOUR) {
            fleet_health_rollup::rollup_hour(&db, hour, NOW).await?;
        }
        Ok(())
    })
    .await?;
    let counted: i64 =
        sqlx::query_scalar("SELECT COALESCE(SUM(job_count),0) FROM fleet_health_hourly")
            .fetch_one(&db)
            .await?;
    assert_eq!(
        counted,
        i64::try_from(week.jobs)?,
        "every job of the week in exactly one row"
    );
    db.close().await;
    std::fs::remove_dir_all(directory)?;
    Ok(())
}
