//! Contract tests for #190 (job categories) in `gridops-reconciler`,
//! written by #197: the poll insert, the backfill and the reclassify.
//!
//! #190 removes the `#[ignore = "contract: #190"]` markers and makes these
//! pass; it changes nothing else here (`Fleet health guard`). The rules are
//! #190's "Work" list as `docs/design/fleet-health.md` records them:
//! - the poll's `upsert_workflow_job` sets a new row's `category` through
//!   `classifier_cache::category_for_insert` (#197 binds it; the stub answers
//!   `None`), reading the workflow name and repository from the run;
//! - `fleet_health_backfill::run_pass(db, now)` classifies rows with
//!   `category IS NULL` (GitHub and GitLab) in batches of 500, one short
//!   transaction each;
//! - when the rules change, it reclassifies the rows completed in the last
//!   7 days and lists their hours in `fleet_health_recompute`.
//!
//! These classify through the process-wide cache, so they hold
//! `testdata::CLASSIFIER_TEST_LOCK`. Mounted from `main.rs` (#197-owned).

use std::collections::{BTreeMap, BTreeSet};

use super::*;
use gridops_core::{
    connect_database_path,
    fleet_health::{
        speed_check,
        testdata::{self, JobSeed, SeededFleet},
    },
};

const MINUTE: i64 = 60_000;
const HOUR: i64 = 60 * MINUTE;
const DAY: i64 = 24 * HOUR;
/// Job names and the category the default rules give each.
const NAMES: [(&str, &str); 5] = [
    ("unit tests", "test"),
    ("docker image", "build"),
    ("clippy", "check"),
    ("publish", "deploy"),
    ("notify slack", "other"),
];

struct Harness {
    directory: std::path::PathBuf,
    db: SqlitePool,
    fleet: SeededFleet,
}

impl Harness {
    async fn new() -> Result<Self> {
        let directory =
            std::env::temp_dir().join(format!("gridops-backfill-{}", uuid::Uuid::new_v4()));
        let db = connect_database_path(&directory.join("gridops.sqlite")).await?;
        let fleet = testdata::seed_fleet(&db).await?;
        Ok(Self {
            directory,
            db,
            fleet,
        })
    }

    /// Passes until no row is left unclassified (at most `limit`).
    async fn backfill(&self, now: i64, limit: usize) -> Result<usize> {
        for pass in 1..=limit {
            fleet_health_backfill::run_pass(&self.db, now).await?;
            if self.unclassified().await? == 0 {
                return Ok(pass);
            }
        }
        anyhow::bail!("rows still unclassified after {limit} passes")
    }

    async fn unclassified(&self) -> Result<i64> {
        Ok(sqlx::query_scalar(
            "SELECT (SELECT COUNT(*) FROM workflow_jobs WHERE category IS NULL)
                  + (SELECT COUNT(*) FROM gitlab_jobs WHERE category IS NULL)",
        )
        .fetch_one(&self.db)
        .await?)
    }

    /// Every job's (table, id, category, `updated_at`), for "nothing changed".
    async fn state(&self) -> Result<String> {
        Ok(sqlx::query_scalar(
            "SELECT json_array(
               (SELECT json_group_array(json_array(id,category,updated_at)) FROM (SELECT * FROM workflow_jobs ORDER BY id)),
               (SELECT json_group_array(json_array(id,category,updated_at)) FROM (SELECT * FROM gitlab_jobs ORDER BY id)),
               (SELECT json_group_array(hour) FROM (SELECT hour FROM fleet_health_recompute ORDER BY hour)))",
        )
        .fetch_one(&self.db)
        .await?)
    }

    async fn finish(self) -> Result<()> {
        self.db.close().await;
        std::fs::remove_dir_all(self.directory)?;
        Ok(())
    }
}

#[tokio::test]
async fn a_polled_job_is_stored_with_its_category() -> Result<()> {
    let _serial = testdata::CLASSIFIER_TEST_LOCK.lock().await;
    let harness = Harness::new().await?;
    let now = now_millis();
    sqlx::query(
        "INSERT INTO workflow_runs (id,repository_id,workflow_name,run_number,event,status,head_sha,html_url,github_created_at,github_updated_at,created_at,updated_at)
         SELECT 900,id,'CI',1,'push','in_progress','abc','https://example.test/r/900',1,1,1,1 FROM repositories WHERE full_name=?",
    )
    .bind(&harness.fleet.repository)
    .execute(&harness.db)
    .await?;
    let mut connection = harness.db.acquire().await?;
    for (offset, (name, expected)) in NAMES.iter().enumerate() {
        let id = 9_000 + i64::try_from(offset)?;
        let job: GitHubWorkflowJob = serde_json::from_value(json!({
            "id": id, "run_id": 900, "name": name, "status": "queued",
            "conclusion": null, "runner_id": null, "runner_name": null,
            "runner_group_id": null, "runner_group_name": null,
            "labels": ["self-hosted", "gridops"],
            "html_url": format!("https://example.test/j/{id}"),
            "started_at": null, "completed_at": null
        }))?;
        upsert_workflow_job(&mut connection, &job, now).await?;
        let stored: Option<String> =
            sqlx::query_scalar("SELECT category FROM workflow_jobs WHERE id=?")
                .bind(id)
                .fetch_one(&mut *connection)
                .await?;
        assert_eq!(stored.as_deref(), Some(*expected), "{name}");
    }
    drop(connection);
    harness.finish().await
}

#[tokio::test]
async fn ten_thousand_rows_are_backfilled_in_short_transactions_and_a_rerun_changes_nothing()
-> Result<()> {
    let _serial = testdata::CLASSIFIER_TEST_LOCK.lock().await;
    let harness = Harness::new().await?;
    let now = now_millis();
    let io = harness.fleet.host("io")?.id.clone();
    let jobs: Vec<JobSeed> = (0..10_000_i64)
        .map(|id| {
            let (name, _) = NAMES[usize::try_from(id).unwrap_or(0) % NAMES.len()];
            let completed = now - HOUR - id * 50_000;
            let mut job = JobSeed::completed(
                id + 1,
                name,
                Some(&io),
                completed - 120_000,
                completed - 60_000,
                completed,
            );
            job.category = None;
            // One in a hundred is a GitLab job.
            job.gitlab = id % 100 == 0;
            job
        })
        .collect();
    testdata::insert_jobs(&harness.db, &harness.fleet, &jobs).await?;
    assert_eq!(harness.unclassified().await?, 10_000);

    let passes = speed_check::check(&harness.db, harness.backfill(now, 100)).await?;
    assert!(passes >= 1);
    let counts: Vec<(String, i64)> = sqlx::query_as(
        "SELECT category,COUNT(*) FROM (SELECT category FROM workflow_jobs UNION ALL SELECT category FROM gitlab_jobs)
          GROUP BY category ORDER BY category",
    )
    .fetch_all(&harness.db)
    .await?;
    let expected: BTreeMap<String, i64> = NAMES
        .iter()
        .map(|(_, category)| ((*category).to_owned(), 2_000))
        .collect();
    assert_eq!(counts.into_iter().collect::<BTreeMap<_, _>>(), expected);

    let before = harness.state().await?;
    for _ in 0..3 {
        fleet_health_backfill::run_pass(&harness.db, now).await?;
    }
    assert_eq!(harness.state().await?, before, "re-running changes nothing");
    harness.finish().await
}

#[tokio::test]
async fn a_rule_change_reclassifies_the_last_7_days_and_lists_their_hours() -> Result<()> {
    let _serial = testdata::CLASSIFIER_TEST_LOCK.lock().await;
    let harness = Harness::new().await?;
    let now = now_millis();
    let io = harness.fleet.host("io")?.id.clone();
    let seed = |id: i64, completed: i64| {
        let mut job = JobSeed::completed(
            id,
            "notify slack",
            Some(&io),
            completed - 120_000,
            completed - 60_000,
            completed,
        );
        job.category = None;
        job
    };
    let recent = [seed(1, now - 2 * HOUR), seed(2, now - 3 * DAY)];
    let old = seed(3, now - 8 * DAY);
    testdata::insert_jobs(
        &harness.db,
        &harness.fleet,
        &[recent[0].clone(), recent[1].clone(), old],
    )
    .await?;
    harness.backfill(now, 10).await?;
    let category = |id: i64| {
        sqlx::query_scalar::<_, Option<String>>("SELECT category FROM workflow_jobs WHERE id=?")
            .bind(id)
            .fetch_one(&harness.db)
    };
    assert_eq!(category(1).await?.as_deref(), Some("other"));
    sqlx::query("DELETE FROM fleet_health_recompute")
        .execute(&harness.db)
        .await?;

    // A new rule: notifications are deploys now.
    sqlx::query(
        "INSERT INTO job_category_rules (position,category,field,pattern,created_at,updated_at)
         VALUES (4,'deploy','job_name','notify',?,?)",
    )
    .bind(now)
    .bind(now)
    .execute(&harness.db)
    .await?;
    // The cache looks at the rules at most once a minute.
    let later = now + 2 * MINUTE;
    for _ in 0..5 {
        fleet_health_backfill::run_pass(&harness.db, later).await?;
    }
    assert_eq!(category(1).await?.as_deref(), Some("deploy"));
    assert_eq!(category(2).await?.as_deref(), Some("deploy"));
    assert_eq!(
        category(3).await?.as_deref(),
        Some("other"),
        "a row older than 7 days keeps its category"
    );
    let listed: BTreeSet<i64> = sqlx::query_scalar("SELECT hour FROM fleet_health_recompute")
        .fetch_all(&harness.db)
        .await?
        .into_iter()
        .collect();
    let hour_of = |job: &JobSeed| job.completed_at.map(|at| at - at.rem_euclid(HOUR));
    for job in &recent {
        let hour = hour_of(job).context("a completed job")?;
        assert!(
            listed.contains(&hour),
            "the reclassified job's hour {hour} is listed: {listed:?}"
        );
    }
    for hour in &listed {
        assert!(
            *hour >= later - 7 * DAY - HOUR && hour % HOUR == 0,
            "only whole hours of the last 7 days: {hour}"
        );
    }
    harness.finish().await
}
