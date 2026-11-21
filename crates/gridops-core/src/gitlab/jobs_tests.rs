//! D18 (#140 and #148): the shared `gitlab_jobs` writer, `gitlab::jobs`.
//!
//! Mounted from `gitlab/mod.rs` with
//! `#[cfg(test)] #[path = "jobs_tests.rs"] mod jobs_tests;` rather than from
//! `jobs.rs`, so that whichever of #140's and #148's copies of `jobs.rs` ends
//! up on master, these tests keep running against it. They use only the
//! public D18 API, against a migrated database.
//!
//! The rules pinned here are D18's as both briefs state them
//! (`148-brief.md`, "Amendments, round 3"; `140-brief.md`, "Amendments
//! (contract decision D18)"):
//! - tiers: 0 `created`/`waiting_for_resource`/`preparing`/`manual`/`scheduled`,
//!   1 `pending`, 2 `running`, 3 `success`/`failed`/`canceled`/`skipped`,
//!   `unknown` none;
//! - a status write applies only to a non-terminal row, and only for a higher
//!   tier or the same tier with a strictly newer `status_changed_at`;
//! - `pipeline_id` and a `Some` `retries_count` move with an applied status;
//! - `gitlab_runner_id` is `COALESCE(stored, incoming)` and `tag_list` is
//!   replaced only by non-empty tags, both filled on a rejected status write
//!   of a non-terminal row (`FilledOnly`);
//! - a terminal row never changes, `unknown` writes nothing, and `updated_at`
//!   is `now_ms` whenever anything changed;
//! - `set_gitlab_runner_busy` matches live GitLab runners on any connection
//!   with the job connection's `base_url`.

use std::fs;

use anyhow::{Context as _, Result};
use sqlx::SqlitePool;

use crate::connect_database_path;
use crate::gitlab::JobStatus;
use crate::gitlab::jobs::{
    JobObservation, JobWrite, JobWriteOutcome, record_gitlab_job, set_gitlab_runner_busy,
    status_tier, status_write_applies,
};

/// The time every write in these tests happens at, after every seeded time.
const NOW: i64 = 1_000_000;

/// Every status the writer accepts, with its D18 tier.
const TIERS: [(JobStatus, u8); 11] = [
    (JobStatus::Created, 0),
    (JobStatus::WaitingForResource, 0),
    (JobStatus::Preparing, 0),
    (JobStatus::Manual, 0),
    (JobStatus::Scheduled, 0),
    (JobStatus::Pending, 1),
    (JobStatus::Running, 2),
    (JobStatus::Success, 3),
    (JobStatus::Failed, 3),
    (JobStatus::Canceled, 3),
    (JobStatus::Skipped, 3),
];

struct Fixture {
    directory: std::path::PathBuf,
    database: SqlitePool,
}

impl Fixture {
    /// Installation 1, pool `p1`, and three connections: `conn-a` (project 11)
    /// and `conn-a-group` (group 5) on the same instance, and `conn-b`
    /// (project 12) on another one.
    async fn new() -> Result<Self> {
        let directory =
            std::env::temp_dir().join(format!("gridops-gitlab-jobs-test-{}", uuid::Uuid::new_v4()));
        let database = connect_database_path(&directory.join("gridops.sqlite")).await?;
        sqlx::raw_sql(
            r#"
            INSERT INTO installations (id,account_id,account_login,account_type,target_type,repository_selection,created_at,updated_at)
              VALUES (1,1,'octo-org','Organization','Organization','all',1,1);
            INSERT INTO runner_pools (id,installation_id,name,scope,labels,image,created_at,updated_at)
              VALUES ('p1',1,'p1','organization','["docker"]','runner:latest',1,1);
            INSERT INTO gitlab_connections (id,name,base_url,scope_kind,scope_id,scope_path,access_token_key,webhook_secret_key,created_at,updated_at)
              VALUES
              ('conn-a','a','https://gitlab-a.example.test','project',11,'team/a','gitlab.conn-a.token','gitlab.conn-a.webhook',1,1),
              ('conn-a-group','a group','https://gitlab-a.example.test','group',5,'team','gitlab.conn-a-group.token','gitlab.conn-a-group.webhook',1,1),
              ('conn-b','b','https://gitlab-b.example.test','project',12,'team/b','gitlab.conn-b.token','gitlab.conn-b.webhook',1,1);
            "#,
        )
        .execute(&database)
        .await?;
        Ok(Self {
            directory,
            database,
        })
    }

    async fn finish(self) -> Result<()> {
        self.database.close().await;
        fs::remove_dir_all(self.directory)?;
        Ok(())
    }

    /// A `conn-a` row of project 11 and pipeline 1, as another writer left
    /// it: `status` since `at`, tagged `docker`, no retries, no runner, owned
    /// by pool `p1`, created at 1 and last written at `at`. `set` is an
    /// optional `UPDATE ... SET` list applied after the insert.
    async fn seed(&self, job_id: i64, status: &str, at: i64, set: &str) -> Result<Row> {
        sqlx::query(
            "INSERT INTO gitlab_jobs (id,connection_id,project_id,job_id,pipeline_id,status,tag_list,status_changed_at,retries_count,gitlab_runner_id,pool_id,created_at,updated_at)
               VALUES (?,'conn-a',11,?,1,?,'[\"docker\"]',?,0,NULL,'p1',1,?)",
        )
        .bind(uuid::Uuid::new_v4().to_string())
        .bind(job_id)
        .bind(status)
        .bind(at)
        .bind(at)
        .execute(&self.database)
        .await?;
        if !set.is_empty() {
            sqlx::query(sqlx::AssertSqlSafe(format!(
                "UPDATE gitlab_jobs SET {set} WHERE connection_id='conn-a' AND job_id=?"
            )))
            .bind(job_id)
            .execute(&self.database)
            .await?;
        }
        self.row(job_id)
            .await?
            .with_context(|| format!("seeded {job_id}"))
    }

    /// The `conn-a` row of `job_id`, every column.
    async fn row(&self, job_id: i64) -> Result<Option<Row>> {
        Ok(sqlx::query_as::<_, Row>(
            "SELECT id,connection_id,project_id,job_id,pipeline_id,status,tag_list,status_changed_at,
                    retries_count,gitlab_runner_id,pool_id,created_at,updated_at
               FROM gitlab_jobs WHERE connection_id='conn-a' AND job_id=?",
        )
        .bind(job_id)
        .fetch_optional(&self.database)
        .await?)
    }

    async fn record(&self, observation: &JobObservation<'_>) -> Result<JobWriteOutcome> {
        let mut connection = self.database.acquire().await?;
        Ok(record_gitlab_job(&mut connection, observation, NOW).await?)
    }

    async fn runners(&self) -> Result<Vec<RunnerRow>> {
        Ok(sqlx::query_as::<_, RunnerRow>(
            "SELECT id,pool_id,name,status,busy,ci_platform,gitlab_connection_id,gitlab_runner_id,
                    deleted_at,last_heartbeat_at,current_job_id,registered_at,created_at,updated_at
               FROM runners ORDER BY id",
        )
        .fetch_all(&self.database)
        .await?)
    }

    async fn set_busy(
        &self,
        connection_id: &str,
        gitlab_runner_id: u64,
        busy: bool,
        now: i64,
    ) -> Result<Option<(String, Option<String>)>> {
        let mut connection = self.database.acquire().await?;
        Ok(
            set_gitlab_runner_busy(&mut connection, connection_id, gitlab_runner_id, busy, now)
                .await?,
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
struct Row {
    id: String,
    connection_id: String,
    project_id: i64,
    job_id: i64,
    pipeline_id: i64,
    status: String,
    tag_list: String,
    status_changed_at: i64,
    retries_count: i64,
    gitlab_runner_id: Option<i64>,
    pool_id: Option<String>,
    created_at: i64,
    updated_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
struct RunnerRow {
    id: String,
    pool_id: String,
    name: String,
    status: String,
    busy: bool,
    ci_platform: String,
    gitlab_connection_id: Option<String>,
    gitlab_runner_id: Option<i64>,
    deleted_at: Option<i64>,
    last_heartbeat_at: Option<i64>,
    current_job_id: Option<i64>,
    registered_at: Option<i64>,
    created_at: i64,
    updated_at: i64,
}

/// A `conn-a` observation of project 11 and pipeline 1 with nothing optional.
fn observe(job_id: u64, status: JobStatus, at: i64) -> JobObservation<'static> {
    JobObservation {
        connection_id: "conn-a",
        project_id: 11,
        job_id,
        pipeline_id: 1,
        status,
        status_changed_at: at,
        retries_count: None,
        tag_list: None,
        gitlab_runner_id: None,
    }
}

/// The outcome kind, named, so a wrong one reads clearly in a failure.
fn kind(outcome: &JobWriteOutcome) -> &'static str {
    match outcome.write {
        JobWrite::Inserted => "Inserted",
        JobWrite::StatusApplied => "StatusApplied",
        JobWrite::FilledOnly => "FilledOnly",
        JobWrite::Ignored => "Ignored",
    }
}

fn tags(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| (*value).to_owned()).collect()
}

#[test]
fn status_tier_ranks_every_status_into_its_d18_tier() {
    for (status, tier) in TIERS {
        assert_eq!(status_tier(status.as_str()), Some(tier), "{status:?}");
    }
    assert_eq!(status_tier(JobStatus::Unknown.as_str()), None);
}

#[tokio::test]
async fn a_status_write_applies_only_upward_or_at_the_same_tier_when_newer_and_never_to_a_terminal_row()
-> Result<()> {
    // Every stored status against every incoming status, with the incoming
    // time older than, equal to and newer than the stored one.
    let fixture = Fixture::new().await?;
    let stored_at = 500;
    let mut job_id = 0_i64;
    for (stored, stored_tier) in TIERS {
        for (incoming, incoming_tier) in TIERS {
            for incoming_at in [stored_at - 1, stored_at, stored_at + 1] {
                job_id += 1;
                let applies = stored_tier < 3
                    && (incoming_tier > stored_tier
                        || (incoming_tier == stored_tier && incoming_at > stored_at));
                let case = format!("{stored:?} at {stored_at} <- {incoming:?} at {incoming_at}");
                assert_eq!(
                    status_write_applies(stored.as_str(), stored_at, incoming, incoming_at),
                    applies,
                    "status_write_applies: {case}"
                );

                let before = fixture.seed(job_id, stored.as_str(), stored_at, "").await?;
                let outcome = fixture
                    .record(&JobObservation {
                        pipeline_id: 2,
                        ..observe(job_id as u64, incoming, incoming_at)
                    })
                    .await?;
                let after = fixture.row(job_id).await?.context("still recorded")?;
                if applies {
                    assert_eq!(kind(&outcome), "StatusApplied", "{case}");
                    assert_eq!(
                        outcome.previous_status.as_deref(),
                        Some(stored.as_str()),
                        "{case}"
                    );
                    assert_eq!(outcome.status, incoming.as_str(), "{case}");
                    assert_eq!(
                        after,
                        Row {
                            status: incoming.as_str().to_owned(),
                            status_changed_at: incoming_at,
                            pipeline_id: 2,
                            updated_at: NOW,
                            ..before
                        },
                        "{case}: only status, its time, the pipeline and updated_at move; pool_id is never written"
                    );
                } else {
                    assert_eq!(kind(&outcome), "Ignored", "{case}");
                    assert_eq!(after, before, "{case}: a rejected write changes nothing");
                }
            }
        }
    }
    fixture.finish().await
}

#[tokio::test]
async fn a_new_row_gets_a_uuid_and_the_given_values_or_the_defaults() -> Result<()> {
    let fixture = Fixture::new().await?;

    let given = tags(&["docker", "Gpu"]);
    let outcome = fixture
        .record(&JobObservation {
            pipeline_id: 7,
            retries_count: Some(2),
            tag_list: Some(&given),
            gitlab_runner_id: Some(55),
            ..observe(101, JobStatus::Running, 400)
        })
        .await?;
    assert_eq!(kind(&outcome), "Inserted");
    assert_eq!(outcome.previous_status, None);
    assert_eq!(outcome.status, "running");
    assert_eq!(outcome.gitlab_runner_id, Some(55));
    let row = fixture.row(101).await?.context("101 is recorded")?;
    assert_eq!(uuid::Uuid::parse_str(&row.id)?.get_version_num(), 4);
    assert_eq!(
        (
            row.connection_id.as_str(),
            row.project_id,
            row.pipeline_id,
            row.status.as_str(),
            row.tag_list.as_str(),
            row.status_changed_at,
            row.retries_count,
            row.gitlab_runner_id,
            row.pool_id.as_deref(),
            row.updated_at,
        ),
        (
            "conn-a",
            11,
            7,
            "running",
            r#"["docker","Gpu"]"#,
            400,
            2,
            Some(55),
            None,
            NOW
        )
    );

    // Nothing optional given: no tags, no retries, no runner.
    let outcome = fixture
        .record(&observe(102, JobStatus::Pending, 400))
        .await?;
    assert_eq!(kind(&outcome), "Inserted");
    assert_eq!(outcome.gitlab_runner_id, None);
    let row = fixture.row(102).await?.context("102 is recorded")?;
    assert_eq!(
        (
            row.tag_list.as_str(),
            row.retries_count,
            row.gitlab_runner_id,
            row.pool_id
        ),
        ("[]", 0, None, None)
    );

    // Empty tags are stored as the empty list.
    let empty: Vec<String> = Vec::new();
    fixture
        .record(&JobObservation {
            tag_list: Some(&empty),
            ..observe(103, JobStatus::Pending, 400)
        })
        .await?;
    let row = fixture.row(103).await?.context("103 is recorded")?;
    assert_eq!(row.tag_list, "[]");

    fixture.finish().await
}

#[tokio::test]
async fn unknown_writes_nothing() -> Result<()> {
    let fixture = Fixture::new().await?;
    let offered = tags(&["gpu"]);
    let unknown = |job_id| JobObservation {
        retries_count: Some(4),
        tag_list: Some(&offered),
        gitlab_runner_id: Some(9),
        ..observe(job_id, JobStatus::Unknown, 900)
    };

    // No row: none is created.
    let outcome = fixture.record(&unknown(201)).await?;
    assert_eq!(kind(&outcome), "Ignored");
    assert_eq!(fixture.row(201).await?, None);

    // A row with room for every fill: nothing is filled either.
    let before = fixture.seed(202, "pending", 100, "tag_list='[]'").await?;
    let outcome = fixture.record(&unknown(202)).await?;
    assert_eq!(kind(&outcome), "Ignored");
    assert_eq!(fixture.row(202).await?, Some(before));

    // Control: the same observation with a known status is written, so the
    // writer is live and only `unknown` is dropped.
    let outcome = fixture
        .record(&JobObservation {
            status: JobStatus::Pending,
            ..unknown(201)
        })
        .await?;
    assert_eq!(kind(&outcome), "Inserted");
    assert!(fixture.row(201).await?.is_some());

    fixture.finish().await
}

#[tokio::test]
async fn a_terminal_row_never_changes_in_any_column() -> Result<()> {
    // Each terminal row has room for every fill (no tags, no runner), and is
    // offered a newer observation of every kind with every optional value.
    let fixture = Fixture::new().await?;
    let offered = tags(&["docker", "gpu"]);
    let mut job_id = 300_i64;
    for stored in ["success", "failed", "canceled", "skipped"] {
        for incoming in [
            JobStatus::Pending,
            JobStatus::Running,
            JobStatus::Success,
            JobStatus::Failed,
        ] {
            job_id += 1;
            let before = fixture
                .seed(job_id, stored, 100, "tag_list='[]', pool_id=NULL")
                .await?;
            let outcome = fixture
                .record(&JobObservation {
                    pipeline_id: 9,
                    retries_count: Some(3),
                    tag_list: Some(&offered),
                    gitlab_runner_id: Some(77),
                    ..observe(job_id as u64, incoming, 900)
                })
                .await?;
            assert_eq!(kind(&outcome), "Ignored", "{stored} <- {incoming:?}");
            assert_eq!(
                fixture.row(job_id).await?,
                Some(before),
                "{stored} <- {incoming:?}"
            );
        }
    }

    // Control: the same observation onto a `running` row is applied, so the
    // terminal rows above are frozen by their status, not by a dead writer.
    fixture
        .seed(399, "running", 100, "tag_list='[]', pool_id=NULL")
        .await?;
    let outcome = fixture
        .record(&JobObservation {
            pipeline_id: 9,
            retries_count: Some(3),
            tag_list: Some(&offered),
            gitlab_runner_id: Some(77),
            ..observe(399, JobStatus::Failed, 900)
        })
        .await?;
    assert_eq!(kind(&outcome), "StatusApplied");
    let row = fixture.row(399).await?.context("399")?;
    assert_eq!(
        (
            row.status.as_str(),
            row.gitlab_runner_id,
            row.tag_list.as_str()
        ),
        ("failed", Some(77), r#"["docker","gpu"]"#)
    );

    fixture.finish().await
}

#[tokio::test]
async fn the_runner_id_is_filled_once_and_never_cleared_or_overwritten() -> Result<()> {
    let fixture = Fixture::new().await?;

    // A rejected status write (an older `running`) still fills the runner.
    let before = fixture.seed(401, "running", 100, "").await?;
    let outcome = fixture
        .record(&JobObservation {
            gitlab_runner_id: Some(55),
            ..observe(401, JobStatus::Running, 50)
        })
        .await?;
    assert_eq!(kind(&outcome), "FilledOnly");
    assert_eq!(outcome.previous_status.as_deref(), Some("running"));
    assert_eq!(outcome.status, "running");
    assert_eq!(outcome.gitlab_runner_id, Some(55));
    assert_eq!(
        fixture.row(401).await?,
        Some(Row {
            gitlab_runner_id: Some(55),
            updated_at: NOW,
            ..before
        }),
        "only the runner and updated_at change"
    );

    // An applied write with another runner id keeps the first one.
    let outcome = fixture
        .record(&JobObservation {
            gitlab_runner_id: Some(66),
            ..observe(401, JobStatus::Running, 200)
        })
        .await?;
    assert_eq!(kind(&outcome), "StatusApplied");
    assert_eq!(outcome.gitlab_runner_id, Some(55));
    let row = fixture.row(401).await?.context("401")?;
    assert_eq!(
        (row.status_changed_at, row.gitlab_runner_id),
        (200, Some(55))
    );

    // An applied write with no runner id does not clear it.
    let outcome = fixture
        .record(&observe(401, JobStatus::Success, 300))
        .await?;
    assert_eq!(kind(&outcome), "StatusApplied");
    assert_eq!(outcome.previous_status.as_deref(), Some("running"));
    assert_eq!(outcome.status, "success");
    assert_eq!(outcome.gitlab_runner_id, Some(55));
    let row = fixture.row(401).await?.context("401")?;
    assert_eq!(
        (row.status.as_str(), row.gitlab_runner_id),
        ("success", Some(55))
    );

    // An applied write fills an empty runner id.
    fixture.seed(402, "pending", 100, "").await?;
    let outcome = fixture
        .record(&JobObservation {
            gitlab_runner_id: Some(66),
            ..observe(402, JobStatus::Running, 200)
        })
        .await?;
    assert_eq!(kind(&outcome), "StatusApplied");
    assert_eq!(outcome.gitlab_runner_id, Some(66));
    let row = fixture.row(402).await?.context("402")?;
    assert_eq!(row.gitlab_runner_id, Some(66));

    // A rejected write onto a row that already has a runner fills nothing.
    let before = fixture
        .seed(403, "running", 100, "gitlab_runner_id=55")
        .await?;
    let outcome = fixture
        .record(&JobObservation {
            gitlab_runner_id: Some(66),
            ..observe(403, JobStatus::Pending, 200)
        })
        .await?;
    assert_eq!(kind(&outcome), "Ignored");
    assert_eq!(fixture.row(403).await?, Some(before));

    fixture.finish().await
}

#[tokio::test]
async fn tags_are_replaced_only_by_non_empty_tags_and_filled_on_a_rejected_write() -> Result<()> {
    let fixture = Fixture::new().await?;
    let empty: Vec<String> = Vec::new();
    let docker = tags(&["docker"]);
    let gpu = tags(&["gpu"]);

    // Applied writes: `Some([])` and `None` keep the stored tags; non-empty
    // tags replace them.
    fixture.seed(501, "pending", 100, "").await?;
    for (at, offered, expected) in [
        (200, Some(empty.as_slice()), r#"["docker"]"#),
        (300, None, r#"["docker"]"#),
        (400, Some(gpu.as_slice()), r#"["gpu"]"#),
    ] {
        let outcome = fixture
            .record(&JobObservation {
                tag_list: offered,
                ..observe(501, JobStatus::Running, at)
            })
            .await?;
        assert_eq!(kind(&outcome), "StatusApplied", "{offered:?}");
        let row = fixture.row(501).await?.context("501")?;
        assert_eq!(row.tag_list, expected, "{offered:?}");
    }

    // A rejected write (a lower tier) fills empty stored tags.
    let before = fixture.seed(502, "running", 100, "tag_list='[]'").await?;
    let outcome = fixture
        .record(&JobObservation {
            tag_list: Some(&docker),
            ..observe(502, JobStatus::Pending, 200)
        })
        .await?;
    assert_eq!(kind(&outcome), "FilledOnly");
    assert_eq!(outcome.previous_status.as_deref(), Some("running"));
    assert_eq!(outcome.status, "running");
    assert_eq!(
        fixture.row(502).await?,
        Some(Row {
            tag_list: r#"["docker"]"#.to_owned(),
            updated_at: NOW,
            ..before
        }),
        "only the tags and updated_at change"
    );

    // A rejected write neither replaces stored tags nor fills with nothing.
    let tagged = fixture.seed(503, "running", 100, "").await?;
    let untagged = fixture.seed(504, "running", 100, "tag_list='[]'").await?;
    for (job_id, offered, before) in [
        (503, gpu.as_slice(), tagged),
        (504, empty.as_slice(), untagged),
    ] {
        let outcome = fixture
            .record(&JobObservation {
                tag_list: Some(offered),
                ..observe(job_id as u64, JobStatus::Pending, 200)
            })
            .await?;
        assert_eq!(kind(&outcome), "Ignored", "{job_id}");
        assert_eq!(fixture.row(job_id).await?, Some(before), "{job_id}");
    }

    fixture.finish().await
}

#[tokio::test]
async fn retries_count_and_pipeline_move_only_with_an_applied_status_write() -> Result<()> {
    let fixture = Fixture::new().await?;

    // `None` keeps the stored retries; the pipeline moves with the status.
    fixture.seed(601, "pending", 100, "retries_count=2").await?;
    let outcome = fixture
        .record(&JobObservation {
            pipeline_id: 3,
            ..observe(601, JobStatus::Running, 200)
        })
        .await?;
    assert_eq!(kind(&outcome), "StatusApplied");
    let row = fixture.row(601).await?.context("601")?;
    assert_eq!((row.retries_count, row.pipeline_id), (2, 3));

    // `Some` moves with the status.
    fixture
        .record(&JobObservation {
            pipeline_id: 4,
            retries_count: Some(4),
            ..observe(601, JobStatus::Success, 300)
        })
        .await?;
    let row = fixture.row(601).await?.context("601")?;
    assert_eq!((row.retries_count, row.pipeline_id), (4, 4));

    // A rejected status write moves neither.
    let before = fixture.seed(602, "running", 100, "retries_count=2").await?;
    let outcome = fixture
        .record(&JobObservation {
            pipeline_id: 6,
            retries_count: Some(5),
            ..observe(602, JobStatus::Pending, 200)
        })
        .await?;
    assert_eq!(kind(&outcome), "Ignored");
    assert_eq!(fixture.row(602).await?, Some(before));

    fixture.finish().await
}

#[tokio::test]
async fn set_gitlab_runner_busy_matches_the_runner_id_on_any_connection_of_the_same_instance()
-> Result<()> {
    // GitLab runner ids are unique per instance: the job's connection is
    // `conn-a`, and runner 55 registered through `conn-a-group` (same base
    // URL) is the one that ran it. Runner 55 on another instance, a deleted
    // runner 55, runner 56 and a non-GitLab runner carrying 55 are not.
    let fixture = Fixture::new().await?;
    sqlx::raw_sql(
        "INSERT INTO runners (id,pool_id,name,status,busy,ci_platform,gitlab_connection_id,gitlab_runner_id,deleted_at,last_heartbeat_at,created_at,updated_at)
           VALUES
           ('sibling','p1','sibling','online',0,'gitlab','conn-a-group',55,NULL,1,1,1),
           ('other-instance','p1','other-instance','online',0,'gitlab','conn-b',55,NULL,1,1,1),
           ('deleted','p1','deleted','online',0,'gitlab','conn-a',55,5,1,1,1),
           ('other-id','p1','other-id','online',0,'gitlab','conn-a',56,NULL,1,1,1),
           ('not-gitlab','p1','not-gitlab','online',0,'github','conn-a',55,NULL,1,1,1);",
    )
    .execute(&fixture.database)
    .await?;
    let before = fixture.runners().await?;
    let with_sibling = |busy: bool, status: &str, at: i64| {
        before
            .iter()
            .map(|runner| {
                if runner.id == "sibling" {
                    RunnerRow {
                        busy,
                        status: status.to_owned(),
                        last_heartbeat_at: Some(at),
                        updated_at: at,
                        ..runner.clone()
                    }
                } else {
                    runner.clone()
                }
            })
            .collect::<Vec<_>>()
    };

    let changed = fixture.set_busy("conn-a", 55, true, 2_000).await?;
    assert_eq!(changed, Some(("sibling".to_owned(), Some("p1".to_owned()))));
    assert_eq!(fixture.runners().await?, with_sibling(true, "busy", 2_000));

    let changed = fixture.set_busy("conn-a", 55, false, 3_000).await?;
    assert_eq!(changed, Some(("sibling".to_owned(), Some("p1".to_owned()))));
    assert_eq!(
        fixture.runners().await?,
        with_sibling(false, "online", 3_000)
    );

    // No live GitLab runner 99 on the instance: nothing changes.
    let changed = fixture.set_busy("conn-a", 99, true, 4_000).await?;
    assert_eq!(changed, None);
    assert_eq!(
        fixture.runners().await?,
        with_sibling(false, "online", 3_000)
    );

    fixture.finish().await
}
