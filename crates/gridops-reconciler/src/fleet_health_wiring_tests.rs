//! #197: the fleet health wiring in the reconciler that is live before any
//! wave-1 issue lands:
//! - `cleanup_retention` keeps 90 days of `fleet_health_hourly`,
//!   `fleet_host_hourly` and `fleet_health_snapshots`;
//! - the poll's `upsert_workflow_job` binds `category` only when it first
//!   writes a row: a later poll keeps the stored one (W-15);
//! - every fleet health pass (#190, #191, #200, #204) succeeds on an empty
//!   database, which is how `reconcile()` calls them, log-and-continue.
//!
//! Mounted from `main.rs`.

use super::*;
use gridops_core::connect_database_path;

const DAY: i64 = 86_400_000;
const HOUR: i64 = 3_600_000;

async fn reconciler(directory: &std::path::Path) -> Result<Reconciler> {
    let database = connect_database_path(&directory.join("gridops.sqlite")).await?;
    let config = Config::from_env()?;
    Ok(Reconciler {
        github: GitHubClient::new(config.clone())?,
        config,
        database,
        bitbucket: BitbucketClient::new()?,
        gitlab: Arc::new(gridops_core::gitlab::fake::FakeGitLab::default()),
        vault: Vault::new([0; 32], b"reconciler-test".to_vec()),
        http: reqwest::Client::new(),
        pass: Arc::default(),
        tart: Arc::new(tart_lane::TartLane::new(
            Arc::default(),
            tart_lane::SLOT_WATCH_INTERVAL,
        )),
    })
}

#[tokio::test]
async fn retention_keeps_90_days_of_fleet_health_rows() -> Result<()> {
    let directory =
        std::env::temp_dir().join(format!("gridops-health-retention-{}", uuid::Uuid::new_v4()));
    let app = reconciler(&directory).await?;
    let now = now_millis();
    let old = (now - 91 * DAY) - (now - 91 * DAY).rem_euclid(HOUR);
    let kept = (now - 89 * DAY) - (now - 89 * DAY).rem_euclid(HOUR);
    let nine = "[0,0,0,0,0,0,0,0,0]";
    for hour in [old, kept] {
        sqlx::query(
            "INSERT INTO fleet_health_hourly VALUES (?,'unmanaged','unmanaged','other',1,1,0,10,0.5,?,?,?)",
        )
        .bind(hour)
        .bind(nine)
        .bind(nine)
        .bind(hour)
        .execute(&app.database)
        .await?;
        sqlx::query(
            "INSERT INTO fleet_host_hourly VALUES (?,'h',0,NULL,NULL,NULL,NULL,0,'{}',0,?)",
        )
        .bind(hour)
        .bind(hour)
        .execute(&app.database)
        .await?;
        sqlx::query("INSERT INTO fleet_health_snapshots VALUES (?,'24h',NULL,'{}')")
            .bind(hour)
            .execute(&app.database)
            .await?;
    }

    cleanup_retention(&app).await?;

    for (table, column) in [
        ("fleet_health_hourly", "hour"),
        ("fleet_host_hourly", "hour"),
        ("fleet_health_snapshots", "recorded_at"),
    ] {
        let left: Vec<i64> = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
            "SELECT {column} FROM {table} ORDER BY {column}"
        )))
        .fetch_all(&app.database)
        .await?;
        assert_eq!(left, [kept], "{table}: 91 days old is deleted, 89 is kept");
    }
    app.database.close().await;
    std::fs::remove_dir_all(directory)?;
    Ok(())
}

#[tokio::test]
async fn a_later_poll_keeps_the_stored_category() -> Result<()> {
    let directory =
        std::env::temp_dir().join(format!("gridops-poll-category-{}", uuid::Uuid::new_v4()));
    let app = reconciler(&directory).await?;
    sqlx::raw_sql(
        r#"
        INSERT INTO installations (id,account_id,account_login,account_type,target_type,repository_selection,created_at,updated_at)
          VALUES (1,1,'octo','User','User','all',1,1);
        INSERT INTO repositories (id,installation_id,owner,name,full_name,private,default_branch,html_url,last_synced_at,created_at,updated_at)
          VALUES (10,1,'octo','app','octo/app',0,'main','https://github.com/octo/app',1,1,1);
        INSERT INTO workflow_runs (id,repository_id,workflow_name,run_number,event,status,head_sha,html_url,github_created_at,github_updated_at,created_at,updated_at)
          VALUES (20,10,'CI',1,'push','in_progress','abc','https://github.com/octo/app/actions/runs/20',1,1,1,1);
        "#,
    )
    .execute(&app.database)
    .await?;
    let job = |status: &str, conclusion: Option<&str>| -> Result<GitHubWorkflowJob> {
        Ok(serde_json::from_value(json!({
            "id": 30, "run_id": 20, "name": "unit tests", "status": status,
            "conclusion": conclusion, "runner_id": null, "runner_name": null,
            "runner_group_id": null, "runner_group_name": null,
            "labels": ["self-hosted"], "html_url": "https://github.com/octo/app/actions/runs/20/job/30",
            "started_at": "2026-07-21T00:00:00Z",
            "completed_at": conclusion.map(|_| "2026-07-21T00:01:00Z")
        }))?)
    };
    let mut connection = app.database.acquire().await?;
    let now = now_millis();
    upsert_workflow_job(&mut connection, &job("in_progress", None)?, now).await?;
    sqlx::query("UPDATE workflow_jobs SET category='deploy' WHERE id=30")
        .execute(&mut *connection)
        .await?;
    upsert_workflow_job(
        &mut connection,
        &job("completed", Some("success"))?,
        now + 1,
    )
    .await?;
    let (status, category): (String, Option<String>) =
        sqlx::query_as("SELECT status,category FROM workflow_jobs WHERE id=30")
            .fetch_one(&mut *connection)
            .await?;
    assert_eq!(status, "completed", "the poll was applied");
    assert_eq!(
        category.as_deref(),
        Some("deploy"),
        "the stored category is kept"
    );
    drop(connection);
    app.database.close().await;
    std::fs::remove_dir_all(directory)?;
    Ok(())
}

#[tokio::test]
async fn every_fleet_health_pass_succeeds_on_an_empty_database() -> Result<()> {
    let directory =
        std::env::temp_dir().join(format!("gridops-health-passes-{}", uuid::Uuid::new_v4()));
    let app = reconciler(&directory).await?;
    let now = now_millis();
    fleet_health_backfill::run_pass(&app.database, now).await?;
    fleet_health_rollup::run_pass(&app.database, now).await?;
    fleet_host_rollup::run_pass(&app.database, now).await?;
    fleet_health_snapshot::run_pass(&app.database, now).await?;
    app.database.close().await;
    std::fs::remove_dir_all(directory)?;
    Ok(())
}
