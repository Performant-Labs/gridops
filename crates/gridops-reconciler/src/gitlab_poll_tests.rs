//! #148: the GitLab pending-jobs poll (`gitlab_poll::poll_once`) against
//! `FakeGitLab`, the shared `gitlab_credentials` loader, the placeholder
//! client, and scale-down of a GitLab pool.
//!
//! The fake keys all state by base URL, so every connection here has its own.
//! `poll_once(app, &mut backoff, now)` takes the clock the rate-limit backoff
//! is measured on, so the tests step it instead of sleeping; the conclusion
//! step's age is measured on `gitlab_jobs.updated_at` in milliseconds.

use super::*;
use crate::gitlab_poll::{
    GITLAB_JOB_CHECK_AGE_MS, GITLAB_JOB_CHECK_LIMIT, UnavailableGitLab, poll_once,
};
use anyhow::Context as _;
use gridops_core::connect_database_path;
use gridops_core::fleet::ExternalId;
use gridops_core::gitlab::fake::{FakeGitLab, FakeMethod};
use gridops_core::gitlab::{
    GitLabApi, GitLabCredentials, GitLabError, Job, JobStatus, Scope, ScopeKind,
};
use secrecy::ExposeSecret as _;
use std::{fs, time::Instant};

const BASE_A: &str = "https://gitlab-a.example.test";
const BASE_B: &str = "https://gitlab-b.example.test";
const BASE_G: &str = "https://gitlab-g.example.test";
const BASE_Z: &str = "https://gitlab-z.example.test";

/// A database, a `FakeGitLab` and a reconciler that calls it.
struct Harness {
    app: Reconciler,
    fake: FakeGitLab,
    directory: std::path::PathBuf,
}

impl Harness {
    /// Installation 1 and these GitLab connections, each with its access
    /// token sealed in `runtime_secrets` under `access_token_key`:
    /// - `conn-a`: project 11 on `BASE_A`;
    /// - `conn-b`: project 12 on `BASE_B`;
    /// - `conn-g`: group 7 on `BASE_G`;
    /// - `conn-z`: project 13 on `BASE_Z`, which no pool targets.
    ///
    /// Then `extra`.
    async fn new(extra: &str) -> Result<Self> {
        let directory =
            std::env::temp_dir().join(format!("gridops-gitlab-poll-test-{}", uuid::Uuid::new_v4()));
        let database = connect_database_path(&directory.join("gridops.sqlite")).await?;
        sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
            r#"
            INSERT INTO installations (id,account_id,account_login,account_type,target_type,repository_selection,created_at,updated_at)
              VALUES (1,1,'octo-org','Organization','Organization','all',1,1);
            INSERT INTO gitlab_connections (id,name,base_url,scope_kind,scope_id,scope_path,access_token_key,webhook_secret_key,created_at,updated_at)
              VALUES
              ('conn-a','a','{BASE_A}','project',11,'team/a','gitlab.conn-a.token','gitlab.conn-a.webhook',1,1),
              ('conn-b','b','{BASE_B}','project',12,'team/b','gitlab.conn-b.token','gitlab.conn-b.webhook',1,1),
              ('conn-g','g','{BASE_G}','group',7,'team','gitlab.conn-g.token','gitlab.conn-g.webhook',1,1),
              ('conn-z','z','{BASE_Z}','project',13,'team/z','gitlab.conn-z.token','gitlab.conn-z.webhook',1,1);
            {extra}
            "#
        )))
        .execute(&database)
        .await?;
        let vault = Vault::new([0; 32], b"reconciler-test".to_vec());
        for connection in ["a", "b", "g", "z"] {
            sqlx::query("INSERT INTO runtime_secrets (key,value,updated_at) VALUES (?,?,1)")
                .bind(format!("gitlab.conn-{connection}.token"))
                .bind(vault.seal(&format!("glpat-{connection}"))?)
                .execute(&database)
                .await?;
        }
        let fake = FakeGitLab::default();
        let config = Config::from_env()?;
        let app = Reconciler {
            github: GitHubClient::new(config.clone())?,
            config,
            database,
            bitbucket: BitbucketClient::new()?,
            gitlab: Arc::new(fake.clone()),
            vault,
            http: reqwest::Client::new(),
            pass: Arc::default(),
            tart: Arc::new(tart_lane::TartLane::new(
                Arc::default(),
                tart_lane::SLOT_WATCH_INTERVAL,
            )),
        };
        Ok(Self {
            app,
            fake,
            directory,
        })
    }

    fn database(&self) -> &SqlitePool {
        &self.app.database
    }

    async fn finish(self) -> Result<()> {
        self.app.database.close().await;
        fs::remove_dir_all(self.directory)?;
        Ok(())
    }

    /// Seeds a job on the fake instance at `base`.
    fn seed(&self, base: &str, job: Job) -> Result<()> {
        self.fake.with_job(&url(base)?, job);
        Ok(())
    }

    /// How many `method` calls the poll made on `base` so far.
    fn calls(&self, base: &str, method: FakeMethod) -> Result<usize> {
        let base = url(base)?;
        Ok(self
            .fake
            .calls()
            .iter()
            .filter(|call| call.base_url == base && call.method == method)
            .count())
    }

    /// The recorded row of `(connection, job_id)`.
    async fn row(&self, connection: &str, job_id: i64) -> Result<Option<JobRow>> {
        Ok(sqlx::query_as::<_, JobRow>(
            "SELECT id,status,project_id,pipeline_id,tag_list,status_changed_at,created_at,updated_at
               FROM gitlab_jobs WHERE connection_id=? AND job_id=?",
        )
        .bind(connection)
        .bind(job_id)
        .fetch_optional(self.database())
        .await?)
    }

    /// The status of `(connection, job_id)`, or an error if it isn't recorded.
    async fn status(&self, connection: &str, job_id: i64) -> Result<String> {
        self.row(connection, job_id)
            .await?
            .map(|row| row.status)
            .with_context(|| format!("{connection}/{job_id} is not recorded"))
    }

    async fn rows(&self) -> Result<i64> {
        Ok(sqlx::query_scalar("SELECT COUNT(*) FROM gitlab_jobs")
            .fetch_one(self.database())
            .await?)
    }
}

#[derive(Debug, FromRow)]
struct JobRow {
    id: String,
    status: String,
    project_id: i64,
    pipeline_id: i64,
    tag_list: String,
    status_changed_at: i64,
    created_at: i64,
    updated_at: i64,
}

fn url(base: &str) -> Result<reqwest::Url> {
    Ok(reqwest::Url::parse(base)?)
}

fn id(value: u64) -> Result<ExternalId> {
    Ok(ExternalId::new(value)?)
}

/// A GitLab job of pipeline 1.
fn job(project_id: u64, job_id: u64, status: JobStatus, tags: &[&str]) -> Result<Job> {
    Ok(Job {
        id: id(job_id)?,
        project_id: id(project_id)?,
        pipeline_id: id(1)?,
        status,
        tag_list: tags.iter().map(|tag| (*tag).to_owned()).collect(),
    })
}

/// An autoscaling pool configuring `docker`, linked to `connection`.
fn pool(id: &str, connection: &str) -> String {
    format!(
        "INSERT INTO runner_pools (id,installation_id,name,scope,labels,image,autoscaling_enabled,created_at,updated_at)
           VALUES ('{id}',1,'{id}','organization','[\"docker\"]','runner:latest',1,1,1);
         INSERT INTO runner_pool_gitlab_connections (pool_id,connection_id,created_at) VALUES ('{id}','{connection}',1);"
    )
}

/// A recorded `gitlab_jobs` row tagged `docker`, written at `at` (its
/// `created_at`, `updated_at` and `status_changed_at`).
fn row(connection: &str, project_id: i64, job_id: i64, status: &str, at: i64) -> String {
    format!(
        "INSERT INTO gitlab_jobs (id,connection_id,project_id,job_id,pipeline_id,status,tag_list,status_changed_at,created_at,updated_at)
           VALUES ('{}','{connection}',{project_id},{job_id},1,'{status}','[\"docker\"]',{at},{at},{at});",
        uuid::Uuid::new_v4()
    )
}

/// A time older than the conclusion step's age, by `extra` milliseconds.
fn stale(extra: i64) -> i64 {
    now_millis() - GITLAB_JOB_CHECK_AGE_MS - 60_000 - extra
}

#[test]
fn the_conclusion_step_bounds_are_the_briefed_values() {
    assert_eq!(GITLAB_JOB_CHECK_LIMIT, 10);
    assert_eq!(GITLAB_JOB_CHECK_AGE_MS, 5 * 60 * 1_000);
}

#[tokio::test]
async fn a_pending_job_with_no_webhook_row_is_recorded_once_and_scales_its_pool() -> Result<()> {
    let harness = Harness::new(&pool("gl", "conn-a")).await?;
    harness.seed(
        BASE_A,
        job(11, 501, JobStatus::Pending, &["docker", "Gpu"])?,
    )?;
    // conn-z has a pending job too, but no pool targets conn-z.
    harness.seed(BASE_Z, job(13, 901, JobStatus::Pending, &["docker"])?)?;
    sqlx::raw_sql("UPDATE runner_pools SET labels='[\"docker\",\"gpu\"]' WHERE id='gl';")
        .execute(harness.database())
        .await?;
    let mut backoff = HashMap::new();

    let before = now_millis();
    poll_once(&harness.app, &mut backoff, Instant::now()).await?;
    let after = now_millis();

    let row = harness
        .row("conn-a", 501)
        .await?
        .context("the pending job is recorded")?;
    assert_eq!(uuid::Uuid::parse_str(&row.id)?.get_version_num(), 4);
    assert_eq!(row.status, "pending");
    assert_eq!((row.project_id, row.pipeline_id), (11, 1));
    // The contract encoding: serde_json of the Vec<String>, in GitLab's order.
    assert_eq!(row.tag_list, r#"["docker","Gpu"]"#);
    for written in [row.status_changed_at, row.created_at, row.updated_at] {
        assert!((before..=after).contains(&written), "{row:?}");
    }
    assert_eq!(assigned_queued_jobs(harness.database(), "gl").await?, 1);
    assert_eq!(harness.calls(BASE_Z, FakeMethod::PendingJobs)?, 0);
    assert!(harness.row("conn-z", 901).await?.is_none());

    // Polling again records nothing twice.
    poll_once(&harness.app, &mut backoff, Instant::now()).await?;
    assert_eq!(harness.rows().await?, 1);
    let again = harness
        .row("conn-a", 501)
        .await?
        .context("still recorded")?;
    assert_eq!(again.id, row.id);
    assert_eq!(again.status, "pending");
    assert_eq!(assigned_queued_jobs(harness.database(), "gl").await?, 1);

    harness.finish().await
}

#[tokio::test]
async fn a_group_connection_is_polled_through_its_projects() -> Result<()> {
    let harness = Harness::new(&pool("gl", "conn-g")).await?;
    let base = url(BASE_G)?;
    harness.fake.with_group_project(&base, id(7)?, id(21)?);
    harness.fake.with_group_project(&base, id(7)?, id(22)?);
    harness.seed(BASE_G, job(21, 601, JobStatus::Pending, &["docker"])?)?;
    harness.seed(BASE_G, job(22, 602, JobStatus::Pending, &["docker"])?)?;
    // Project 23 is not in the group.
    harness.seed(BASE_G, job(23, 603, JobStatus::Pending, &["docker"])?)?;

    poll_once(&harness.app, &mut HashMap::new(), Instant::now()).await?;

    assert_eq!(harness.calls(BASE_G, FakeMethod::GroupProjects)?, 1);
    assert_eq!(harness.calls(BASE_G, FakeMethod::PendingJobs)?, 2);
    // Recorded under the group connection, with the project each came from.
    let projects = (
        harness.row("conn-g", 601).await?.map(|row| row.project_id),
        harness.row("conn-g", 602).await?.map(|row| row.project_id),
    );
    assert_eq!(projects, (Some(21), Some(22)));
    assert!(harness.row("conn-g", 603).await?.is_none());
    assert_eq!(assigned_queued_jobs(harness.database(), "gl").await?, 2);

    harness.finish().await
}

#[tokio::test]
async fn a_stale_recorded_job_is_concluded_through_each_job_outcome() -> Result<()> {
    // 701 finished on GitLab without a webhook reaching us; GitLab no longer
    // has 702 at all.
    let harness = Harness::new(&format!(
        "{}{}{}",
        pool("gl", "conn-a"),
        row("conn-a", 11, 701, "pending", stale(0)),
        row("conn-a", 11, 702, "running", stale(0)),
    ))
    .await?;
    harness.seed(BASE_A, job(11, 701, JobStatus::Success, &["docker"])?)?;

    let before = now_millis();
    poll_once(&harness.app, &mut HashMap::new(), Instant::now()).await?;
    let after = now_millis();

    assert_eq!(harness.calls(BASE_A, FakeMethod::Job)?, 2);
    let success = harness.row("conn-a", 701).await?.context("701")?;
    assert_eq!(success.status, JobStatus::Success.as_str());
    let gone = harness.row("conn-a", 702).await?.context("702")?;
    assert_eq!(gone.status, "canceled");
    for concluded in [&success, &gone] {
        assert!(
            (before..=after).contains(&concluded.status_changed_at),
            "{concluded:?}"
        );
    }
    assert_eq!(assigned_queued_jobs(harness.database(), "gl").await?, 0);

    harness.finish().await
}

#[tokio::test]
async fn a_rate_limited_connection_is_skipped_until_its_retry_after_while_others_are_polled()
-> Result<()> {
    // conn-a is told to wait 60 s; conn-b is not limited; conn-g is limited
    // with no Retry-After, so it waits the poll's documented default.
    let harness = Harness::new(&format!(
        "{}{}{}",
        pool("pool-a", "conn-a"),
        pool("pool-b", "conn-b"),
        pool("pool-g", "conn-g"),
    ))
    .await?;
    harness.seed(BASE_A, job(11, 501, JobStatus::Pending, &["docker"])?)?;
    harness.seed(BASE_B, job(12, 502, JobStatus::Pending, &["docker"])?)?;
    harness
        .fake
        .with_group_project(&url(BASE_G)?, id(7)?, id(21)?);
    harness.seed(BASE_G, job(21, 503, JobStatus::Pending, &["docker"])?)?;
    harness.fake.fail_next(
        &url(BASE_A)?,
        FakeMethod::PendingJobs,
        GitLabError::RateLimited {
            retry_after: Some(Duration::from_secs(60)),
        },
    );
    harness.fake.fail_next(
        &url(BASE_G)?,
        FakeMethod::GroupProjects,
        GitLabError::RateLimited { retry_after: None },
    );
    let mut backoff = HashMap::new();
    let start = Instant::now();

    // A limited connection does not fail the poll or hold up the others.
    poll_once(&harness.app, &mut backoff, start).await?;
    assert!(harness.row("conn-a", 501).await?.is_none());
    assert!(harness.row("conn-b", 502).await?.is_some());
    let a_pending = harness.calls(BASE_A, FakeMethod::PendingJobs)?;
    assert_eq!(a_pending, 1);
    let g_calls = (
        harness.calls(BASE_G, FakeMethod::GroupProjects)?,
        harness.calls(BASE_G, FakeMethod::PendingJobs)?,
    );
    assert_eq!(g_calls, (1, 0));

    // A second later both limited connections are skipped; conn-b is polled.
    let b_before = harness.calls(BASE_B, FakeMethod::PendingJobs)?;
    poll_once(&harness.app, &mut backoff, start + Duration::from_secs(1)).await?;
    assert_eq!(harness.calls(BASE_A, FakeMethod::PendingJobs)?, a_pending);
    assert_eq!(
        (
            harness.calls(BASE_G, FakeMethod::GroupProjects)?,
            harness.calls(BASE_G, FakeMethod::PendingJobs)?,
        ),
        g_calls
    );
    assert_eq!(
        harness.calls(BASE_B, FakeMethod::PendingJobs)?,
        b_before + 1
    );
    assert!(harness.row("conn-a", 501).await?.is_none());

    // Once the retry-after has passed, conn-a is polled again.
    poll_once(&harness.app, &mut backoff, start + Duration::from_secs(61)).await?;
    assert_eq!(
        harness.calls(BASE_A, FakeMethod::PendingJobs)?,
        a_pending + 1
    );
    assert_eq!(
        harness.row("conn-a", 501).await?.map(|row| row.status),
        Some("pending".to_owned())
    );

    harness.finish().await
}

#[tokio::test]
async fn the_conclusion_step_checks_at_most_the_limit_and_only_rows_older_than_the_age()
-> Result<()> {
    // LIMIT + 2 stale rows (job ids in age order, oldest first) and one fresh
    // row, none of them pending on GitLab any more.
    let limit = GITLAB_JOB_CHECK_LIMIT;
    let stale_rows = (0..limit + 2)
        .map(|index| {
            row(
                "conn-a",
                11,
                1_000 + index,
                "pending",
                stale(limit + 2 - index),
            )
        })
        .collect::<Vec<_>>()
        .concat();
    let harness = Harness::new(&format!(
        "{}{stale_rows}{}",
        pool("gl", "conn-a"),
        row("conn-a", 11, 2_000, "pending", now_millis()),
    ))
    .await?;

    poll_once(&harness.app, &mut HashMap::new(), Instant::now()).await?;

    assert_eq!(
        i64::try_from(harness.calls(BASE_A, FakeMethod::Job)?)?,
        limit
    );
    for index in 0..limit {
        assert_eq!(harness.status("conn-a", 1_000 + index).await?, "canceled");
    }
    for index in limit..limit + 2 {
        assert_eq!(harness.status("conn-a", 1_000 + index).await?, "pending");
    }
    assert_eq!(harness.status("conn-a", 2_000).await?, "pending");

    harness.finish().await
}

#[tokio::test]
async fn stale_pending_rows_are_checked_first_and_running_rows_last() -> Result<()> {
    // LIMIT running rows older than a `created` row, older than a `pending`
    // row. Pending goes first, then created, then the oldest running rows:
    // the two newest running rows wait for the next poll.
    let limit = GITLAB_JOB_CHECK_LIMIT;
    let running = (0..limit)
        .map(|index| row("conn-a", 11, 100 + index, "running", stale(1_000 - index)))
        .collect::<Vec<_>>()
        .concat();
    let harness = Harness::new(&format!(
        "{}{running}{}{}",
        pool("gl", "conn-a"),
        row("conn-a", 11, 200, "created", stale(100)),
        row("conn-a", 11, 300, "pending", stale(0)),
    ))
    .await?;

    poll_once(&harness.app, &mut HashMap::new(), Instant::now()).await?;

    assert_eq!(harness.status("conn-a", 300).await?, "canceled");
    assert_eq!(harness.status("conn-a", 200).await?, "canceled");
    for index in 0..limit - 2 {
        assert_eq!(harness.status("conn-a", 100 + index).await?, "canceled");
    }
    for index in limit - 2..limit {
        assert_eq!(harness.status("conn-a", 100 + index).await?, "running");
    }

    harness.finish().await
}

#[tokio::test]
async fn a_checked_row_is_not_asked_about_again_within_the_age_whatever_the_answer() -> Result<()> {
    // 801 is still running on GitLab; asking about 802 fails.
    let harness = Harness::new(&format!(
        "{}{}{}",
        pool("gl", "conn-a"),
        row("conn-a", 11, 801, "running", stale(0)),
        row("conn-a", 11, 802, "running", stale(0)),
    ))
    .await?;
    harness.seed(BASE_A, job(11, 801, JobStatus::Running, &["docker"])?)?;
    harness.seed(BASE_A, job(11, 802, JobStatus::Running, &["docker"])?)?;
    harness.fake.fail_next(
        &url(BASE_A)?,
        FakeMethod::Job,
        GitLabError::Upstream("GitLab answered 502".into()),
    );

    let before = now_millis();
    poll_once(&harness.app, &mut HashMap::new(), Instant::now()).await?;
    assert_eq!(harness.calls(BASE_A, FakeMethod::Job)?, 2);
    for job_id in [801, 802] {
        let checked = harness.row("conn-a", job_id).await?.context("recorded")?;
        assert_eq!(checked.status, "running");
        assert!(checked.updated_at >= before, "{checked:?}");
    }

    poll_once(&harness.app, &mut HashMap::new(), Instant::now()).await?;
    assert_eq!(harness.calls(BASE_A, FakeMethod::Job)?, 2);

    harness.finish().await
}

#[tokio::test]
async fn the_poll_never_moves_a_running_row_back_to_pending() -> Result<()> {
    // A webhook wrote `running` for 811 after this poll's observation time,
    // and for 812 before it. GitLab's pending list still has both. 813 has no
    // row, so the poll visibly ran.
    let later = now_millis() + 3_600_000;
    let earlier = now_millis() - 3_600_000;
    let harness = Harness::new(&format!(
        "{}{}{}",
        pool("gl", "conn-a"),
        row("conn-a", 11, 811, "running", later),
        row("conn-a", 11, 812, "running", earlier),
    ))
    .await?;
    for job_id in [811, 812, 813] {
        harness.seed(BASE_A, job(11, job_id, JobStatus::Pending, &["docker"])?)?;
    }

    poll_once(&harness.app, &mut HashMap::new(), Instant::now()).await?;

    for (job_id, written) in [(811, later), (812, earlier)] {
        let kept = harness.row("conn-a", job_id).await?.context("recorded")?;
        assert_eq!(
            (kept.status.as_str(), kept.status_changed_at),
            ("running", written),
            "{job_id}"
        );
    }
    assert_eq!(harness.status("conn-a", 813).await?, "pending");
    assert_eq!(assigned_queued_jobs(harness.database(), "gl").await?, 1);

    harness.finish().await
}

#[tokio::test]
async fn the_poll_never_resurrects_a_terminal_row() -> Result<()> {
    // 821 finished (a webhook said so) though GitLab's pending list still
    // has it; 822 has no row, so the poll visibly ran.
    let written = now_millis() - 1_000;
    let harness = Harness::new(&format!(
        "{}{}",
        pool("gl", "conn-a"),
        row("conn-a", 11, 821, "success", written),
    ))
    .await?;
    for job_id in [821, 822] {
        harness.seed(BASE_A, job(11, job_id, JobStatus::Pending, &["docker"])?)?;
    }

    poll_once(&harness.app, &mut HashMap::new(), Instant::now()).await?;

    let kept = harness.row("conn-a", 821).await?.context("recorded")?;
    assert_eq!(
        (kept.status.as_str(), kept.status_changed_at),
        ("success", written)
    );
    assert_eq!(harness.status("conn-a", 822).await?, "pending");
    // Only 822 is demand.
    assert_eq!(assigned_queued_jobs(harness.database(), "gl").await?, 1);
    assert_eq!(harness.rows().await?, 2);

    harness.finish().await
}

#[tokio::test]
async fn a_manual_or_scheduled_row_gitlab_now_lists_as_pending_becomes_pending_demand() -> Result<()>
{
    let harness = Harness::new(&format!(
        "{}{}{}",
        pool("gl", "conn-a"),
        "INSERT INTO gitlab_jobs (id,connection_id,project_id,job_id,pipeline_id,status,tag_list,status_changed_at,created_at,updated_at)
           VALUES ('0b6c3a52-6f7e-4d3c-9a31-5f0c2c1d0831','conn-a',11,831,1,'manual','[\"docker\"]',1,1,1);",
        row("conn-a", 11, 832, "scheduled", 1),
    ))
    .await?;
    for job_id in [831, 832] {
        harness.seed(BASE_A, job(11, job_id, JobStatus::Pending, &["docker"])?)?;
    }
    assert_eq!(assigned_queued_jobs(harness.database(), "gl").await?, 0);

    poll_once(&harness.app, &mut HashMap::new(), Instant::now()).await?;

    let played = harness.row("conn-a", 831).await?.context("recorded")?;
    assert_eq!(played.status, "pending");
    // The same row, under the same (connection_id, job_id).
    assert_eq!(played.id, "0b6c3a52-6f7e-4d3c-9a31-5f0c2c1d0831");
    assert_eq!(harness.status("conn-a", 832).await?, "pending");
    assert_eq!(harness.rows().await?, 2);
    assert_eq!(assigned_queued_jobs(harness.database(), "gl").await?, 2);

    harness.finish().await
}

#[tokio::test]
async fn concluding_a_running_row_clears_its_runners_busy_flag_and_records_one_runner_event()
-> Result<()> {
    // GitLab runner 55 on conn-a ran 841, which finished with no webhook.
    // Only that live runner is freed (D18's `set_gitlab_runner_busy`): not
    // runner 56, not runner 55 on another GitLab instance (conn-b), not a
    // deleted runner that had the same id. 842 was running with no known
    // runner, so concluding it frees nobody and records no event.
    let harness = Harness::new(&format!(
        "{}{}{}{}{}",
        pool("gl", "conn-a"),
        row("conn-a", 11, 841, "running", stale(1)),
        row("conn-a", 11, 842, "running", stale(0)),
        "UPDATE gitlab_jobs SET gitlab_runner_id=55 WHERE job_id=841;",
        "INSERT INTO runners (id,pool_id,name,status,busy,ci_platform,gitlab_connection_id,gitlab_runner_id,deleted_at,created_at,updated_at)
           VALUES
           ('ran-it','gl','ran-it','busy',1,'gitlab','conn-a',55,NULL,1,1),
           ('other-runner','gl','other-runner','busy',1,'gitlab','conn-a',56,NULL,1,1),
           ('other-connection','gl','other-connection','busy',1,'gitlab','conn-b',55,NULL,1,1),
           ('deleted','gl','deleted','busy',1,'gitlab','conn-a',55,5,1,1);",
    ))
    .await?;
    harness.seed(BASE_A, job(11, 841, JobStatus::Failed, &["docker"])?)?;
    harness.seed(BASE_A, job(11, 842, JobStatus::Success, &["docker"])?)?;

    poll_once(&harness.app, &mut HashMap::new(), Instant::now()).await?;

    assert_eq!(harness.status("conn-a", 841).await?, "failed");
    assert_eq!(harness.status("conn-a", 842).await?, "success");
    let busy = sqlx::query_as::<_, (String, bool, String)>(
        "SELECT id,busy,status FROM runners ORDER BY id",
    )
    .fetch_all(harness.database())
    .await?;
    assert_eq!(
        busy,
        vec![
            ("deleted".to_owned(), true, "busy".to_owned()),
            ("other-connection".to_owned(), true, "busy".to_owned()),
            ("other-runner".to_owned(), true, "busy".to_owned()),
            ("ran-it".to_owned(), false, "online".to_owned()),
        ]
    );

    // One event, on the freed runner, shaped like #140's.
    let events = sqlx::query_as::<_, (Option<String>, Option<String>, String, String, String)>(
        "SELECT runner_id,pool_id,level,event,metadata FROM runner_events WHERE event LIKE 'GitLab job %'",
    )
    .fetch_all(harness.database())
    .await?;
    assert_eq!(events.len(), 1, "{events:?}");
    let (runner_id, pool_id, level, event, metadata) = &events[0];
    assert_eq!(
        (
            runner_id.as_deref(),
            pool_id.as_deref(),
            level.as_str(),
            event.as_str()
        ),
        (Some("ran-it"), Some("gl"), "info", "GitLab job failed")
    );
    // The ids may be stored as JSON numbers or strings; their values are pinned.
    let metadata: serde_json::Value = serde_json::from_str(metadata)?;
    let field = |key: &str| match &metadata[key] {
        serde_json::Value::String(text) => Some(text.clone()),
        serde_json::Value::Number(number) => Some(number.to_string()),
        _ => None,
    };
    assert_eq!(
        ["jobId", "projectId", "pipelineId", "connectionId"].map(field),
        [
            Some("841".to_owned()),
            Some("11".to_owned()),
            Some("1".to_owned()),
            Some("conn-a".to_owned()),
        ],
        "{metadata}"
    );

    harness.finish().await
}

#[tokio::test]
async fn rows_of_a_project_whose_pending_jobs_failed_are_not_concluded() -> Result<()> {
    // Group 7 has projects 21 and 22, each with a stale row GitLab no longer
    // lists. Listing one project's pending jobs fails: its row is "not
    // pending" only because the answer is missing, so it is left alone; the
    // other project's row is still concluded.
    let harness = Harness::new(&format!(
        "{}{}{}",
        pool("gl", "conn-g"),
        row("conn-g", 21, 851, "pending", stale(0)),
        row("conn-g", 22, 852, "pending", stale(0)),
    ))
    .await?;
    let base = url(BASE_G)?;
    harness.fake.with_group_project(&base, id(7)?, id(21)?);
    harness.fake.with_group_project(&base, id(7)?, id(22)?);
    harness.fake.fail_next(
        &base,
        FakeMethod::PendingJobs,
        GitLabError::Upstream("GitLab answered 500".into()),
    );

    // Another error is logged for the connection; the poll still succeeds.
    poll_once(&harness.app, &mut HashMap::new(), Instant::now()).await?;

    let mut statuses = vec![
        harness.status("conn-g", 851).await?,
        harness.status("conn-g", 852).await?,
    ];
    statuses.sort();
    assert_eq!(statuses, vec!["canceled".to_owned(), "pending".to_owned()]);
    assert_eq!(harness.calls(BASE_G, FakeMethod::Job)?, 1);

    // The next poll has both answers and concludes the other row too.
    poll_once(&harness.app, &mut HashMap::new(), Instant::now()).await?;
    assert_eq!(harness.status("conn-g", 851).await?, "canceled");
    assert_eq!(harness.status("conn-g", 852).await?, "canceled");

    harness.finish().await
}

#[tokio::test]
async fn gitlab_credentials_reads_the_base_url_and_unseals_the_access_token() -> Result<()> {
    let harness = Harness::new("").await?;

    let credentials = gitlab_credentials(&harness.app, "conn-a")
        .await?
        .context("conn-a has credentials")?;
    assert_eq!(credentials.base_url, url(BASE_A)?);
    assert_eq!(credentials.access_token.expose_secret(), "glpat-a");

    assert!(
        gitlab_credentials(&harness.app, "no-such-connection")
            .await?
            .is_none()
    );
    sqlx::query("DELETE FROM runtime_secrets WHERE key='gitlab.conn-b.token'")
        .execute(harness.database())
        .await?;
    assert!(gitlab_credentials(&harness.app, "conn-b").await?.is_none());

    harness.finish().await
}

#[tokio::test]
async fn the_placeholder_client_refuses_every_call_by_name() -> Result<()> {
    let client: Arc<dyn GitLabApi> = Arc::new(UnavailableGitLab);
    let credentials = GitLabCredentials {
        base_url: url(BASE_A)?,
        access_token: secrecy::SecretString::from("glpat-a".to_owned()),
    };
    let scope = Scope::Project {
        id: id(11)?,
        path: "team/a".into(),
    };
    let unavailable = GitLabError::Upstream("GitLab client not available yet".into());

    assert_eq!(
        client
            .create_runner(&credentials, &scope, &[], false, "runner")
            .await
            .err(),
        Some(unavailable.clone())
    );
    assert_eq!(
        client.delete_runner(&credentials, id(1)?).await.err(),
        Some(unavailable.clone())
    );
    assert_eq!(
        client.job(&credentials, id(11)?, id(1)?).await.err(),
        Some(unavailable.clone())
    );
    assert_eq!(
        client.pending_jobs(&credentials, id(11)?).await.err(),
        Some(unavailable.clone())
    );
    assert_eq!(
        client.group_projects(&credentials, id(7)?).await.err(),
        Some(unavailable.clone())
    );
    assert_eq!(
        client
            .resolve_scope(&credentials, ScopeKind::Project, "team/a")
            .await
            .err(),
        Some(unavailable.clone())
    );
    assert_eq!(
        client.verify_access(&credentials, &scope).await.err(),
        Some(unavailable)
    );
    Ok(())
}

#[tokio::test]
async fn an_idle_gitlab_pool_scales_down_only_once_it_owns_no_gitlab_job() -> Result<()> {
    // The Bitbucket guard in maybe_scale_down keys on `bitbucket` runners, so
    // a pool of idle GitLab runners reaches the queue check and idle timer.
    let harness = Harness::new(&format!(
        "{}{}{}",
        pool("gl", "conn-a"),
        "UPDATE runner_pools SET desired_count=2,min_count=0,idle_timeout_minutes=1 WHERE id='gl';
         INSERT INTO runners (id,pool_id,name,status,busy,ci_platform,gitlab_connection_id,gitlab_runner_id,created_at,updated_at)
           VALUES ('gl-idle','gl','gl-idle','online',0,'gitlab','conn-a',61,1,1);",
        row("conn-a", 11, 861, "pending", now_millis()),
    ))
    .await?;
    let scale_down = async || -> Result<i64> {
        let pool = load_pools(harness.database())
            .await?
            .into_iter()
            .find(|pool| pool.id == "gl")
            .context("the pool loads")?;
        let current = runners(&harness.app, "gl").await?;
        maybe_scale_down(&harness.app, &pool, &current).await?;
        Ok(
            sqlx::query_scalar("SELECT desired_count FROM runner_pools WHERE id='gl'")
                .fetch_one(harness.database())
                .await?,
        )
    };

    // A pending GitLab job it owns keeps its runners.
    assert_eq!(scale_down().await?, 2);

    // Once the job is over, the idle pool scales down to its minimum.
    sqlx::raw_sql("UPDATE gitlab_jobs SET status='success' WHERE job_id=861;")
        .execute(harness.database())
        .await?;
    assert_eq!(scale_down().await?, 0);

    harness.finish().await
}
