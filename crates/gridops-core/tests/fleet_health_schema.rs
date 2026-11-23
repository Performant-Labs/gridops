//! #197: migration 0050, the fleet health schema.
//!
//! Pins what every wave-1 issue (#190, #191, #199, #200, #204) reads and
//! writes, so their contract tests and this schema can't drift apart:
//! - `job_category_rules`, seeded with #197 section 2's four default rules;
//! - `workflow_jobs.category` and `gitlab_jobs.category` (NULL = not
//!   classified yet), and D-1's `gitlab_jobs` name, path and timing columns;
//! - `fleet_health_hourly`, `fleet_host_hourly`, `fleet_health_recompute` and
//!   `fleet_health_snapshots`, all `STRICT` with explicit `CHECK`s;
//! - the `completed_at` indexes the rollups and the speed index read through.
//!   (The 90-day retention deletes of the three tables are batched through
//!   `gridops_core::retention` since #222; `tests/retention.rs` pins their
//!   plans with every other `RetentionTable`'s.)
//!
//! The column lists are the contract (`docs/design/fleet-health.md`); the
//! wave-1 contract tests use these exact names.

use std::{path::PathBuf, time::Duration};

use anyhow::Result;
use sqlx::{
    Row as _, SqlitePool,
    migrate::Migrator,
    sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions},
};

static MIGRATOR: Migrator = sqlx::migrate!("../../migrations");

/// The migration this story adds.
const FLEET_HEALTH_MIGRATION: i64 = 50;

struct Database {
    directory: PathBuf,
    pool: SqlitePool,
}

impl Database {
    async fn at(version: Option<i64>) -> Result<Self> {
        let directory =
            std::env::temp_dir().join(format!("gridops-fleet-health-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&directory)?;
        let pool = SqlitePoolOptions::new()
            .max_connections(2)
            .connect_with(
                SqliteConnectOptions::new()
                    .filename(directory.join("health.sqlite"))
                    .create_if_missing(true)
                    .foreign_keys(true)
                    .journal_mode(SqliteJournalMode::Wal)
                    .busy_timeout(Duration::from_secs(5)),
            )
            .await?;
        match version {
            Some(version) => MIGRATOR.run_to(version, &pool).await?,
            None => MIGRATOR.run(&pool).await?,
        }
        Ok(Self { directory, pool })
    }

    async fn close(self) -> Result<()> {
        self.pool.close().await;
        std::fs::remove_dir_all(self.directory)?;
        Ok(())
    }
}

/// (name, upper-cased declared type, notnull, pk position) per column, in order.
async fn columns(pool: &SqlitePool, table: &str) -> Result<Vec<(String, String, bool, i64)>> {
    Ok(sqlx::query(
        "SELECT name,upper(type) AS type,\"notnull\",pk FROM pragma_table_info(?) ORDER BY cid",
    )
    .bind(table)
    .fetch_all(pool)
    .await?
    .iter()
    .map(|row| {
        (
            row.get("name"),
            row.get("type"),
            row.get::<i64, _>("notnull") == 1,
            row.get("pk"),
        )
    })
    .collect())
}

fn expected(columns: &[(&str, &str, bool, i64)]) -> Vec<(String, String, bool, i64)> {
    columns
        .iter()
        .map(|(name, kind, notnull, pk)| ((*name).to_owned(), (*kind).to_owned(), *notnull, *pk))
        .collect()
}

async fn is_strict(pool: &SqlitePool, table: &str) -> Result<bool> {
    let strict: Option<i64> =
        sqlx::query_scalar("SELECT strict FROM pragma_table_list WHERE name=?")
            .bind(table)
            .fetch_optional(pool)
            .await?;
    Ok(strict == Some(1))
}

async fn refused(pool: &SqlitePool, sql: &str) -> bool {
    sqlx::query(sqlx::AssertSqlSafe(sql.to_owned()))
        .execute(pool)
        .await
        .is_err()
}

async fn accepted(pool: &SqlitePool, sql: &str) -> Result<()> {
    sqlx::query(sqlx::AssertSqlSafe(sql.to_owned()))
        .execute(pool)
        .await
        .map(|_| ())
        .map_err(|error| anyhow::anyhow!("{sql}: {error}"))
}

/// `EXPLAIN QUERY PLAN` details for `sql`.
async fn plan(pool: &SqlitePool, sql: &str) -> Result<Vec<String>> {
    Ok(
        sqlx::query(sqlx::AssertSqlSafe(format!("EXPLAIN QUERY PLAN {sql}")))
            .fetch_all(pool)
            .await?
            .iter()
            .map(|row| row.get::<String, _>("detail"))
            .collect(),
    )
}

/// The first column of every `CREATE INDEX` on `table`.
async fn leading_index_columns(pool: &SqlitePool, table: &str) -> Result<Vec<String>> {
    let names: Vec<String> =
        sqlx::query_scalar("SELECT name FROM pragma_index_list(?) WHERE origin='c'")
            .bind(table)
            .fetch_all(pool)
            .await?;
    let mut leading = Vec::new();
    for name in names {
        if let Some(first) =
            sqlx::query_scalar::<_, String>("SELECT name FROM pragma_index_info(?) WHERE seqno=0")
                .bind(&name)
                .fetch_optional(pool)
                .await?
        {
            leading.push(first);
        }
    }
    Ok(leading)
}

#[tokio::test]
async fn the_new_tables_are_strict_with_the_contract_columns() -> Result<()> {
    let db = Database::at(None).await?;
    for table in [
        "job_category_rules",
        "fleet_health_hourly",
        "fleet_host_hourly",
        "fleet_health_recompute",
        "fleet_health_snapshots",
    ] {
        assert!(is_strict(&db.pool, table).await?, "{table} must be STRICT");
    }
    assert_eq!(
        columns(&db.pool, "job_category_rules").await?,
        expected(&[
            ("id", "INTEGER", false, 1),
            ("position", "INTEGER", true, 0),
            ("category", "TEXT", true, 0),
            ("field", "TEXT", true, 0),
            ("pattern", "TEXT", true, 0),
            ("created_at", "INTEGER", true, 0),
            ("updated_at", "INTEGER", true, 0),
        ])
    );
    assert_eq!(
        columns(&db.pool, "fleet_health_hourly").await?,
        expected(&[
            ("hour", "INTEGER", true, 1),
            ("host_id", "TEXT", true, 2),
            ("pool_id", "TEXT", true, 3),
            ("category", "TEXT", true, 4),
            ("job_count", "INTEGER", true, 0),
            ("completed_count", "INTEGER", true, 0),
            ("infra_failure_count", "INTEGER", true, 0),
            ("duration_ms_sum", "INTEGER", true, 0),
            ("cpu_seconds_sum", "REAL", true, 0),
            ("queue_wait_histogram", "TEXT", true, 0),
            ("duration_histogram", "TEXT", true, 0),
            ("computed_at", "INTEGER", true, 0),
        ])
    );
    assert_eq!(
        columns(&db.pool, "fleet_host_hourly").await?,
        expected(&[
            ("hour", "INTEGER", true, 1),
            ("host_id", "TEXT", true, 2),
            ("online_minutes", "INTEGER", true, 0),
            ("cpu_used_p50_millis", "INTEGER", false, 0),
            ("cpu_used_p95_millis", "INTEGER", false, 0),
            ("memory_used_p95_mib", "INTEGER", false, 0),
            ("min_disk_free_bytes", "INTEGER", false, 0),
            ("start_attempts", "INTEGER", true, 0),
            ("start_failures_json", "TEXT", true, 0),
            ("uncertain_placements", "INTEGER", true, 0),
            ("computed_at", "INTEGER", true, 0),
        ])
    );
    assert_eq!(
        columns(&db.pool, "fleet_health_recompute").await?,
        expected(&[
            ("hour", "INTEGER", true, 1),
            ("requested_at", "INTEGER", true, 0),
        ])
    );
    assert_eq!(
        columns(&db.pool, "fleet_health_snapshots").await?,
        expected(&[
            ("recorded_at", "INTEGER", true, 1),
            ("window", "TEXT", true, 2),
            ("fleet_score", "INTEGER", false, 0),
            ("host_scores_json", "TEXT", true, 0),
        ])
    );
    db.close().await
}

#[tokio::test]
async fn the_default_category_rules_are_seeded_in_order() -> Result<()> {
    let db = Database::at(None).await?;
    let rules: Vec<(i64, String, String, String)> = sqlx::query_as(
        "SELECT position,category,field,pattern FROM job_category_rules ORDER BY position",
    )
    .fetch_all(&db.pool)
    .await?;
    let owned = |position: i64, category: &str, pattern: &str| {
        (
            position,
            category.to_owned(),
            "job_name".to_owned(),
            pattern.to_owned(),
        )
    };
    assert_eq!(
        rules,
        vec![
            owned(
                0,
                "test",
                "test|spec|e2e|playwright|vitest|jest|pytest|cargo test|shard"
            ),
            owned(1, "build", "build|compile|bundle|image|docker|package"),
            owned(
                2,
                "check",
                "lint|clippy|fmt|format|typecheck|tsc|check|audit"
            ),
            owned(3, "deploy", "deploy|release|publish|pages"),
        ],
        "#197 section 2's defaults; `other` is the fallback and has no rule"
    );
    db.close().await
}

#[tokio::test]
async fn the_checks_refuse_values_outside_the_contract() -> Result<()> {
    let db = Database::at(None).await?;
    let pool = &db.pool;
    // Rules: the four rule categories and `other`; fix-agent is never a
    // job category (#201 counts it from fix placements).
    accepted(pool, "INSERT INTO job_category_rules (position,category,field,pattern,created_at,updated_at) VALUES (10,'other','labels','gpu',1,1)").await?;
    for sql in [
        "INSERT INTO job_category_rules (position,category,field,pattern,created_at,updated_at) VALUES (11,'fix-agent','job_name','fix',1,1)",
        "INSERT INTO job_category_rules (position,category,field,pattern,created_at,updated_at) VALUES (12,'lint','job_name','x',1,1)",
        "INSERT INTO job_category_rules (position,category,field,pattern,created_at,updated_at) VALUES (13,'test','title','x',1,1)",
        "INSERT INTO job_category_rules (position,category,field,pattern,created_at,updated_at) VALUES (14,'test','job_name','',1,1)",
        "INSERT INTO job_category_rules (position,category,field,pattern,created_at,updated_at) VALUES (0,'test','job_name','again',1,1)",
        "INSERT INTO job_category_rules (position,category,field,pattern,created_at,updated_at) VALUES (-1,'test','job_name','x',1,1)",
    ] {
        assert!(refused(pool, sql).await, "accepted: {sql}");
    }

    // Job categories: nullable, and only the five job categories.
    accepted(pool, "INSERT INTO installations (id,account_id,account_login,account_type,target_type,repository_selection,created_at,updated_at) VALUES (1,1,'acme','Organization','Organization','all',1,1)").await?;
    accepted(pool, "INSERT INTO repositories (id,installation_id,owner,name,full_name,private,default_branch,html_url,last_synced_at,created_at,updated_at) VALUES (1,1,'acme','app','acme/app',0,'main','https://example.test/acme/app',1,1,1)").await?;
    accepted(pool, "INSERT INTO workflow_runs (id,repository_id,workflow_name,run_number,event,status,head_sha,html_url,github_created_at,github_updated_at,created_at,updated_at) VALUES (1,1,'CI',1,'push','completed','sha','https://example.test/run',1,1,1,1)").await?;
    for (id, category) in [(1, "NULL"), (2, "'test'"), (3, "'other'"), (4, "'deploy'")] {
        accepted(pool, &format!("INSERT INTO workflow_jobs (id,run_id,name,status,labels,html_url,created_at,updated_at,category) VALUES ({id},1,'job','completed','[]','https://example.test/job',1,1,{category})")).await?;
    }
    for category in ["'fix-agent'", "'bogus'", "''"] {
        let sql = format!(
            "INSERT INTO workflow_jobs (id,run_id,name,status,labels,html_url,created_at,updated_at,category) VALUES (9,1,'job','completed','[]','https://example.test/job',1,1,{category})"
        );
        assert!(refused(pool, &sql).await, "accepted: {sql}");
    }
    accepted(pool, "INSERT INTO gitlab_connections (id,name,base_url,scope_kind,scope_id,scope_path,access_token_key,webhook_secret_key,created_at,updated_at) VALUES ('gl','gl','https://gitlab.example.test','project',7,'team/app','gl.token','gl.webhook',1,1)").await?;
    accepted(pool, "INSERT INTO gitlab_jobs (id,connection_id,project_id,job_id,pipeline_id,status,status_changed_at,created_at,updated_at,category,job_name,project_path,started_at,completed_at) VALUES ('g1','gl',7,1,1,'success',1,1,1,'build','build image','team/app',10,20)").await?;
    assert!(
        refused(pool, "INSERT INTO gitlab_jobs (id,connection_id,project_id,job_id,pipeline_id,status,status_changed_at,created_at,updated_at,category) VALUES ('g2','gl',7,2,1,'success',1,1,1,'bogus')").await,
        "gitlab_jobs.category accepts only the job categories"
    );

    // The hourly job rollup: whole hours, `unmanaged` allowed as a host and
    // a pool (no foreign key), nine-bucket histograms.
    let nine = "'[0,0,0,0,0,0,0,0,0]'";
    accepted(pool, &format!("INSERT INTO fleet_health_hourly VALUES (3600000,'unmanaged','unmanaged','other',1,1,0,10,0.5,{nine},{nine},1)")).await?;
    for sql in [
        format!(
            "INSERT INTO fleet_health_hourly VALUES (3600001,'h','p','test',1,1,0,10,0.5,{nine},{nine},1)"
        ),
        format!(
            "INSERT INTO fleet_health_hourly VALUES (7200000,'h','p','fix-agent',1,1,0,10,0.5,{nine},{nine},1)"
        ),
        format!(
            "INSERT INTO fleet_health_hourly VALUES (7200000,'h','p','test',-1,1,0,10,0.5,{nine},{nine},1)"
        ),
        format!(
            "INSERT INTO fleet_health_hourly VALUES (7200000,'h','p','test',1,1,0,10,0.5,'[0,0,0,0,0,0,0,0]',{nine},1)"
        ),
        format!(
            "INSERT INTO fleet_health_hourly VALUES (7200000,'h','p','test',1,1,0,10,0.5,{nine},'not json',1)"
        ),
        format!(
            "INSERT INTO fleet_health_hourly VALUES (7200000,'h','p','test',1,1,0,10,'x',{nine},{nine},1)"
        ),
        format!(
            "INSERT INTO fleet_health_hourly VALUES (3600000,'unmanaged','unmanaged','other',2,2,0,10,0.5,{nine},{nine},1)"
        ),
    ] {
        assert!(refused(pool, &sql).await, "accepted: {sql}");
    }

    // The hourly machine rollup.
    accepted(pool, "INSERT INTO fleet_host_hourly VALUES (3600000,'h',60,100,200,300,400,2,'{\"agent_reported\":1}',0,1)").await?;
    accepted(
        pool,
        "INSERT INTO fleet_host_hourly VALUES (7200000,'h',0,NULL,NULL,NULL,NULL,0,'{}',0,1)",
    )
    .await?;
    for sql in [
        "INSERT INTO fleet_host_hourly VALUES (10800000,'h',61,1,1,1,1,0,'{}',0,1)",
        "INSERT INTO fleet_host_hourly VALUES (10800000,'h',-1,1,1,1,1,0,'{}',0,1)",
        "INSERT INTO fleet_host_hourly VALUES (10800001,'h',1,1,1,1,1,0,'{}',0,1)",
        "INSERT INTO fleet_host_hourly VALUES (10800000,'h',1,1,1,1,1,0,'[]',0,1)",
        "INSERT INTO fleet_host_hourly VALUES (10800000,'h',1,1,1,1,1,-1,'{}',0,1)",
    ] {
        assert!(refused(pool, sql).await, "accepted: {sql}");
    }

    // Recompute marks and snapshots.
    accepted(
        pool,
        "INSERT INTO fleet_health_recompute (hour,requested_at) VALUES (3600000,1)",
    )
    .await?;
    assert!(
        refused(
            pool,
            "INSERT INTO fleet_health_recompute (hour,requested_at) VALUES (3600123,1)"
        )
        .await
    );
    accepted(pool, "INSERT INTO fleet_health_snapshots (recorded_at,window,fleet_score,host_scores_json) VALUES (1,'24h',87,'{}')").await?;
    accepted(pool, "INSERT INTO fleet_health_snapshots (recorded_at,window,fleet_score,host_scores_json) VALUES (1,'7d',NULL,'{}')").await?;
    for sql in [
        "INSERT INTO fleet_health_snapshots (recorded_at,window,fleet_score,host_scores_json) VALUES (2,'30d',1,'{}')",
        "INSERT INTO fleet_health_snapshots (recorded_at,window,fleet_score,host_scores_json) VALUES (2,'24h',101,'{}')",
        "INSERT INTO fleet_health_snapshots (recorded_at,window,fleet_score,host_scores_json) VALUES (2,'24h',-1,'{}')",
        "INSERT INTO fleet_health_snapshots (recorded_at,window,fleet_score,host_scores_json) VALUES (2,'24h',1,'nope')",
    ] {
        assert!(refused(pool, sql).await, "accepted: {sql}");
    }
    db.close().await
}

#[tokio::test]
async fn window_reads_go_through_an_index() -> Result<()> {
    let db = Database::at(None).await?;
    assert!(
        leading_index_columns(&db.pool, "workflow_jobs")
            .await?
            .contains(&"completed_at".to_owned()),
        "workflow_jobs needs an index led by completed_at"
    );
    assert!(
        leading_index_columns(&db.pool, "gitlab_jobs")
            .await?
            .contains(&"completed_at".to_owned()),
        "gitlab_jobs needs an index led by completed_at"
    );
    for sql in [
        "SELECT id,name,run_id,labels FROM workflow_jobs WHERE completed_at>=1 AND completed_at<2",
        "SELECT id,job_name FROM gitlab_jobs WHERE completed_at>=1 AND completed_at<2",
    ] {
        let details = plan(&db.pool, sql).await?;
        assert!(
            details.iter().any(|detail| detail.contains("USING")),
            "{sql} must use an index: {details:?}"
        );
        assert!(
            !details
                .iter()
                .any(|detail| detail.starts_with("SCAN") && !detail.contains("USING")),
            "{sql} scans its table: {details:?}"
        );
    }
    db.close().await
}

/// 0050 on a database that already holds jobs: every existing row keeps
/// every value, the added columns are NULL, and the rules are seeded once.
#[tokio::test]
async fn the_upgrade_keeps_existing_jobs_and_leaves_them_unclassified() -> Result<()> {
    let db = Database::at(Some(FLEET_HEALTH_MIGRATION - 1)).await?;
    sqlx::raw_sql(
        "INSERT INTO installations (id,account_id,account_login,account_type,target_type,repository_selection,created_at,updated_at) VALUES (1,1,'acme','Organization','Organization','all',1,1);
         INSERT INTO repositories (id,installation_id,owner,name,full_name,private,default_branch,html_url,last_synced_at,created_at,updated_at) VALUES (1,1,'acme','app','acme/app',0,'main','https://example.test/acme/app',1,1,1);
         INSERT INTO workflow_runs (id,repository_id,workflow_name,run_number,event,status,head_sha,html_url,github_created_at,github_updated_at,created_at,updated_at) VALUES (1,1,'CI',1,'push','completed','sha','https://example.test/run',1,1,1,1);
         INSERT INTO workflow_jobs (id,run_id,name,status,conclusion,labels,html_url,started_at,completed_at,created_at,updated_at) VALUES (1,1,'unit tests','completed','success','[\"self-hosted\"]','https://example.test/job/1',5,9,1,9),(2,1,'build','queued',NULL,'[]','https://example.test/job/2',NULL,NULL,2,2);
         INSERT INTO gitlab_connections (id,name,base_url,scope_kind,scope_id,scope_path,access_token_key,webhook_secret_key,created_at,updated_at) VALUES ('gl','gl','https://gitlab.example.test','project',7,'team/app','gl.token','gl.webhook',1,1);
         INSERT INTO gitlab_jobs (id,connection_id,project_id,job_id,pipeline_id,status,tag_list,status_changed_at,created_at,updated_at) VALUES ('g1','gl',7,1,1,'success','[\"docker\"]',3,1,3);",
    )
    .execute(&db.pool)
    .await?;
    let jobs_before: Vec<String> = sqlx::query_scalar(
        "SELECT json_array(id,run_id,name,status,conclusion,labels,html_url,started_at,completed_at,created_at,updated_at) FROM workflow_jobs ORDER BY id",
    )
    .fetch_all(&db.pool)
    .await?;
    let gitlab_before: Vec<String> = sqlx::query_scalar(
        "SELECT json_array(id,connection_id,project_id,job_id,pipeline_id,status,tag_list,status_changed_at,retries_count,gitlab_runner_id,pool_id,created_at,updated_at) FROM gitlab_jobs ORDER BY id",
    )
    .fetch_all(&db.pool)
    .await?;

    MIGRATOR.run(&db.pool).await?;

    let jobs_after: Vec<String> = sqlx::query_scalar(
        "SELECT json_array(id,run_id,name,status,conclusion,labels,html_url,started_at,completed_at,created_at,updated_at) FROM workflow_jobs ORDER BY id",
    )
    .fetch_all(&db.pool)
    .await?;
    let gitlab_after: Vec<String> = sqlx::query_scalar(
        "SELECT json_array(id,connection_id,project_id,job_id,pipeline_id,status,tag_list,status_changed_at,retries_count,gitlab_runner_id,pool_id,created_at,updated_at) FROM gitlab_jobs ORDER BY id",
    )
    .fetch_all(&db.pool)
    .await?;
    assert_eq!(jobs_after, jobs_before);
    assert_eq!(gitlab_after, gitlab_before);
    let unclassified: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM workflow_jobs WHERE category IS NULL")
            .fetch_one(&db.pool)
            .await?;
    assert_eq!(unclassified, 2, "existing jobs wait for #190's backfill");
    let gitlab_new: Vec<Option<String>> = sqlx::query_scalar(
        "SELECT value FROM gitlab_jobs, json_each(json_array(category,job_name,project_path,started_at,completed_at)) WHERE gitlab_jobs.id='g1'",
    )
    .fetch_all(&db.pool)
    .await?;
    assert_eq!(gitlab_new, vec![None; 5]);

    // The gitlab_jobs additions, appended, nullable, in this order.
    let gitlab_columns = columns(&db.pool, "gitlab_jobs").await?;
    let tail: Vec<(String, String, bool)> = gitlab_columns[gitlab_columns.len() - 5..]
        .iter()
        .map(|(name, kind, notnull, _)| (name.clone(), kind.clone(), *notnull))
        .collect();
    assert_eq!(
        tail,
        vec![
            ("category".to_owned(), "TEXT".to_owned(), false),
            ("job_name".to_owned(), "TEXT".to_owned(), false),
            ("project_path".to_owned(), "TEXT".to_owned(), false),
            ("started_at".to_owned(), "INTEGER".to_owned(), false),
            ("completed_at".to_owned(), "INTEGER".to_owned(), false),
        ]
    );
    let rules: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM job_category_rules")
        .fetch_one(&db.pool)
        .await?;
    assert_eq!(rules, 4);
    MIGRATOR.run(&db.pool).await?;
    let rules: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM job_category_rules")
        .fetch_one(&db.pool)
        .await?;
    assert_eq!(rules, 4, "rerunning the migrator seeds nothing again");
    assert_eq!(
        sqlx::query_scalar::<_, String>("PRAGMA integrity_check")
            .fetch_one(&db.pool)
            .await?,
        "ok"
    );
    db.close().await
}
