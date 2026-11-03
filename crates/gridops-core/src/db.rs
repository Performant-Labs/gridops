use std::{
    fs,
    path::Path,
    str::FromStr,
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result};
use sqlx::{
    Row as _, SqlitePool,
    migrate::Migrator,
    sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous},
};

use crate::Config;

static MIGRATOR: Migrator = sqlx::migrate!("../../migrations");

pub async fn connect_database(config: &Config) -> Result<SqlitePool> {
    connect_database_path(config.database_path()).await
}

pub async fn connect_database_path(path: &Path) -> Result<SqlitePool> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).context("failed to create database directory")?;
    }
    let options = SqliteConnectOptions::from_str(&format!("sqlite://{}", path.display()))?
        .create_if_missing(true)
        .foreign_keys(true)
        .journal_mode(SqliteJournalMode::Wal)
        .synchronous(SqliteSynchronous::Normal)
        .busy_timeout(std::time::Duration::from_secs(5));
    let pool = SqlitePoolOptions::new()
        .max_connections(8)
        .connect_with(options)
        .await?;
    adopt_legacy_drizzle_schema(&pool).await?;
    MIGRATOR
        .run(&pool)
        .await
        .context("failed to migrate GridOps database")?;
    Ok(pool)
}

async fn adopt_legacy_drizzle_schema(pool: &SqlitePool) -> Result<()> {
    let has_drizzle_history = table_exists(pool, "__drizzle_migrations").await?;
    if !has_drizzle_history {
        return Ok(());
    }

    let applied_sqlx_migrations = if table_exists(pool, "_sqlx_migrations").await? {
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM _sqlx_migrations")
            .fetch_one(pool)
            .await?
    } else {
        0
    };
    if applied_sqlx_migrations > 0 {
        return Ok(());
    }

    for table in ["users", "runner_pools", "runners", "workflow_runs"] {
        anyhow::ensure!(
            table_exists(pool, table).await?,
            "legacy GridOps database is incomplete: missing {table} table"
        );
    }

    tracing::info!("adopting legacy Drizzle migration history into SQLx");
    MIGRATOR
        .skip(pool, Some(1))
        .await
        .context("failed to adopt the legacy base schema")?;

    if column_exists(pool, "runner_pools", "runner_group_id").await? {
        MIGRATOR
            .skip(pool, Some(2))
            .await
            .context("failed to adopt the legacy runner-group migration")?;
    } else {
        MIGRATOR
            .run_to(2, pool)
            .await
            .context("failed to apply the runner-group migration")?;
    }

    let autoscaling_columns = [
        "autoscaling_enabled",
        "queue_scale_factor",
        "idle_timeout_minutes",
    ];
    let mut present = 0;
    for column in autoscaling_columns {
        if column_exists(pool, "runner_pools", column).await? {
            present += 1;
        }
    }

    match present {
        0 => MIGRATOR
            .run_to(3, pool)
            .await
            .context("failed to apply the autoscaling migration")?,
        3 => MIGRATOR
            .skip(pool, Some(3))
            .await
            .context("failed to adopt the legacy autoscaling migration")?,
        _ => anyhow::bail!("legacy GridOps database has a partially applied autoscaling migration"),
    }

    Ok(())
}

async fn table_exists(pool: &SqlitePool, table: &str) -> Result<bool> {
    Ok(sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = ?",
    )
    .bind(table)
    .fetch_one(pool)
    .await?
        > 0)
}

async fn column_exists(pool: &SqlitePool, table: &str, column: &str) -> Result<bool> {
    Ok(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM pragma_table_info(?) WHERE name = ?")
            .bind(table)
            .bind(column)
            .fetch_one(pool)
            .await?
            > 0,
    )
}

pub fn now_millis() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| {
            i64::try_from(duration.as_millis()).unwrap_or(i64::MAX)
        })
}

pub async fn associate_runner_with_job(
    database: &SqlitePool,
    job_id: i64,
    status: &str,
    github_runner_id: Option<i64>,
    runner_name: Option<&str>,
    now: i64,
) -> std::result::Result<Option<(String, String)>, sqlx::Error> {
    let Some(runner_name) = runner_name else {
        return Ok(None);
    };
    match status {
        "in_progress" => {
            sqlx::query(
                r#"UPDATE runners SET busy=1,status='busy',current_job_id=?,last_job_id=?,
                   github_runner_id=COALESCE(github_runner_id,?),
                   last_heartbeat_at=?,updated_at=? WHERE name=? AND deleted_at IS NULL"#,
            )
            .bind(job_id)
            .bind(job_id)
            .bind(github_runner_id)
            .bind(now)
            .bind(now)
            .bind(runner_name)
            .execute(database)
            .await?;
        }
        "completed" => {
            sqlx::query(
                r#"UPDATE runners SET busy=0,status='online',current_job_id=NULL,last_job_id=?,
                   github_runner_id=COALESCE(github_runner_id,?),
                   last_heartbeat_at=?,updated_at=? WHERE name=? AND deleted_at IS NULL"#,
            )
            .bind(job_id)
            .bind(github_runner_id)
            .bind(now)
            .bind(now)
            .bind(runner_name)
            .execute(database)
            .await?;
        }
        _ => {}
    }
    Ok(
        sqlx::query("SELECT id,pool_id FROM runners WHERE name=? AND deleted_at IS NULL")
            .bind(runner_name)
            .fetch_optional(database)
            .await?
            .map(|row| (row.get("id"), row.get("pool_id"))),
    )
}

/// Records that GitHub reported job `job_id` at `now` (`workflow_job_sync`),
/// with its run attempt when GitHub gave one; an attempt already known is kept
/// otherwise. Called wherever a job's status is written from GitHub's word.
pub async fn record_job_report<'e, E>(
    executor: E,
    job_id: i64,
    run_attempt: Option<i64>,
    now: i64,
) -> std::result::Result<(), sqlx::Error>
where
    E: sqlx::SqliteExecutor<'e>,
{
    sqlx::query(
        r#"INSERT INTO workflow_job_sync (job_id,run_attempt,confirmed_at) VALUES (?,?,?)
           ON CONFLICT(job_id) DO UPDATE SET
             run_attempt=COALESCE(excluded.run_attempt,workflow_job_sync.run_attempt),
             confirmed_at=excluded.confirmed_at"#,
    )
    .bind(job_id)
    .bind(run_attempt)
    .bind(now)
    .execute(executor)
    .await?;
    Ok(())
}

/// Finishes the open jobs of `run_id` that belong to an earlier attempt than
/// the newest one known here (the run's own `run_attempt`, or a job of a
/// later attempt, whichever is higher). Returns how many it finished.
///
/// GitHub re-runs a run only once it has completed, and the new attempt keeps
/// the run id, so an earlier attempt's job that is still `queued`,
/// `waiting` or `in_progress` here is one whose completion was missed. It
/// can never run, yet it would count as queued for as long as the new attempt
/// keeps the run unfinished. Its outcome is not known here, so a conclusion
/// already recorded is kept and none is made up: the next poll lists every
/// attempt's jobs and records the real one, and if the run is not polled
/// again the reconciler's job check asks GitHub for it. When GitHub last
/// reported the job is left alone, since GitHub did not say so. Jobs whose attempt is unknown
/// (recorded before attempts were) are left to the reconciler's job check.
pub async fn finish_earlier_attempts<'e, E>(
    executor: E,
    run_id: i64,
    now: i64,
) -> std::result::Result<u64, sqlx::Error>
where
    E: sqlx::SqliteExecutor<'e>,
{
    Ok(sqlx::query(
        r#"UPDATE workflow_jobs
              SET status='completed',completed_at=COALESCE(completed_at,?),updated_at=?
            WHERE run_id=? AND status<>'completed' AND id IN (
              SELECT sync.job_id FROM workflow_job_sync sync
                JOIN workflow_jobs job ON job.id=sync.job_id
               WHERE job.run_id=? AND sync.run_attempt < MAX(
                 COALESCE((SELECT run.run_attempt FROM workflow_runs run WHERE run.id=?),1),
                 COALESCE((SELECT MAX(later.run_attempt) FROM workflow_job_sync later
                   JOIN workflow_jobs later_job ON later_job.id=later.job_id
                  WHERE later_job.run_id=?),1)))"#,
    )
    .bind(now)
    .bind(now)
    .bind(run_id)
    .bind(run_id)
    .bind(run_id)
    .bind(run_id)
    .execute(executor)
    .await?
    .rows_affected())
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::{Connection as _, Executor as _, SqliteConnection};

    #[tokio::test]
    async fn fresh_database_applies_every_migration() -> Result<()> {
        let directory =
            std::env::temp_dir().join(format!("gridops-db-test-{}", uuid::Uuid::new_v4()));
        let path = directory.join("gridops.sqlite");
        let pool = connect_database_path(&path).await?;
        let tables = sqlx::query(
            "SELECT COUNT(*) AS count FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%' AND name != '_sqlx_migrations'",
        )
        .fetch_one(&pool)
        .await?
        .get::<i64, _>("count");
        assert_eq!(tables, 70);
        assert!(table_exists(&pool, "workflow_job_sync").await?);
        assert!(table_exists(&pool, "runner_pool_waits").await?);
        assert!(table_exists(&pool, "fleet_hosts").await?);
        assert!(table_exists(&pool, "api_tokens").await?);
        assert!(table_exists(&pool, "fleet_operations").await?);
        assert!(table_exists(&pool, "capacity_allocations").await?);
        assert!(table_exists(&pool, "fleet_output_streams").await?);
        assert!(table_exists(&pool, "runner_pool_installations").await?);
        assert!(table_exists(&pool, "ai_connections").await?);
        assert!(table_exists(&pool, "agent_runs").await?);
        assert!(table_exists(&pool, "agent_run_events").await?);
        assert!(table_exists(&pool, "bitbucket_connections").await?);
        assert!(table_exists(&pool, "runner_pool_bitbucket_connections").await?);
        assert!(table_exists(&pool, "runner_pool_priority_repositories").await?);
        assert!(table_exists(&pool, "reconciler_passes").await?);
        let columns = sqlx::query("PRAGMA table_info(runner_pools)")
            .fetch_all(&pool)
            .await?;
        assert!(
            columns
                .iter()
                .any(|row| { row.get::<String, _>("name") == "autoscaling_enabled" })
        );
        assert!(
            columns
                .iter()
                .any(|row| row.get::<String, _>("name") == "provision_circuit_open")
        );
        let runner_columns = sqlx::query("PRAGMA table_info(runners)")
            .fetch_all(&pool)
            .await?;
        assert!(
            runner_columns
                .iter()
                .any(|row| row.get::<String, _>("name") == "last_job_id")
        );
        assert!(
            runner_columns
                .iter()
                .any(|row| row.get::<String, _>("name") == "configuration_version")
        );
        assert!(
            runner_columns
                .iter()
                .any(|row| row.get::<String, _>("name") == "target_repository_id")
        );
        assert!(
            columns
                .iter()
                .any(|row| row.get::<String, _>("name") == "configuration_version")
        );
        let user_columns = sqlx::query("PRAGMA table_info(users)")
            .fetch_all(&pool)
            .await?;
        assert!(
            user_columns
                .iter()
                .any(|row| row.get::<String, _>("name") == "role")
        );
        sqlx::raw_sql(
            r#"
            INSERT INTO installations (id,account_id,account_login,account_type,target_type,repository_selection,created_at,updated_at)
              VALUES (1,1,'iamngoni','User','User','selected',1,1);
            INSERT INTO repositories (id,installation_id,owner,name,full_name,private,default_branch,html_url,last_synced_at,created_at,updated_at)
              VALUES
                (10,1,'iamngoni','gridops','iamngoni/gridops',0,'master','https://github.com/iamngoni/gridops',1,1,1),
                (11,1,'iamngoni','tgpatcher','iamngoni/tgpatcher',1,'main','https://github.com/iamngoni/tgpatcher',1,1,1);
            INSERT INTO runner_pools (id,installation_id,name,scope,labels,image,created_at,updated_at)
              VALUES ('pool-1',1,'homelab','repository','["homelab"]','runner:latest',1,1);
            INSERT INTO runner_pool_repositories (pool_id,repository_id,created_at)
              VALUES ('pool-1',10,1),('pool-1',11,1);
            INSERT INTO runners (id,pool_id,target_repository_id,github_runner_id,name,created_at,updated_at)
              VALUES
                ('runner-1','pool-1',10,61,'homelab-gridops',1,1),
                ('runner-2','pool-1',11,61,'homelab-tgpatcher',1,1);
            "#,
        )
        .execute(&pool)
        .await?;
        let shared_github_id =
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM runners WHERE github_runner_id=61")
                .fetch_one(&pool)
                .await?;
        assert_eq!(shared_github_id, 2);
        pool.close().await;
        fs::remove_dir_all(directory)?;
        Ok(())
    }

    #[tokio::test]
    async fn complete_drizzle_schema_is_adopted_without_losing_data() -> Result<()> {
        let directory =
            std::env::temp_dir().join(format!("gridops-legacy-db-test-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&directory)?;
        let path = directory.join("gridops.sqlite");
        let options = SqliteConnectOptions::from_str(&format!("sqlite://{}", path.display()))?
            .create_if_missing(true);
        let mut connection = SqliteConnection::connect_with(&options).await?;
        connection
            .execute(sqlx::raw_sql(include_str!(
                "../../../migrations/0001_initial.sql"
            )))
            .await?;
        connection
            .execute(sqlx::raw_sql(include_str!(
                "../../../migrations/0002_runner_group.sql"
            )))
            .await?;
        connection
            .execute(sqlx::raw_sql(include_str!(
                "../../../migrations/0003_autoscaling.sql"
            )))
            .await?;
        connection
            .execute(
                "CREATE TABLE __drizzle_migrations (id INTEGER PRIMARY KEY, hash TEXT NOT NULL, created_at INTEGER)",
            )
            .await?;
        connection
            .execute(
                "INSERT INTO settings (key, value, updated_at) VALUES ('legacy', 'preserved', 1)",
            )
            .await?;
        connection
            .execute(
                "INSERT INTO users (id,github_id,login,access_token,last_login_at,created_at,updated_at) VALUES ('legacy-user',1,'octocat','sealed',1,1,1)",
            )
            .await?;
        connection.close().await?;

        let pool = connect_database_path(&path).await?;
        let migration_count = sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM _sqlx_migrations")
            .fetch_one(&pool)
            .await?;
        let preserved =
            sqlx::query_scalar::<_, String>("SELECT value FROM settings WHERE key = 'legacy'")
                .fetch_one(&pool)
                .await?;
        let migrated_role =
            sqlx::query_scalar::<_, String>("SELECT role FROM users WHERE id='legacy-user'")
                .fetch_one(&pool)
                .await?;
        assert_eq!(migration_count, i64::try_from(MIGRATOR.migrations.len())?);
        assert_eq!(preserved, "preserved");
        assert_eq!(migrated_role, "admin");
        pool.close().await;
        fs::remove_dir_all(directory)?;
        Ok(())
    }
}
