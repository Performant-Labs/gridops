//! Query retention floors remain durable across reconnects and policy changes.
//! These direct schema checks protect cursor continuity independently of reads.

use std::{path::PathBuf, time::Duration};

use anyhow::Result;
use sqlx::{
    SqlitePool,
    migrate::Migrator,
    sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions},
};

static MIGRATOR: Migrator = sqlx::migrate!("../../migrations");

async fn database() -> Result<(PathBuf, SqlitePool)> {
    let directory =
        std::env::temp_dir().join(format!("gridops-query-schema-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&directory)?;
    let pool = SqlitePoolOptions::new()
        .max_connections(2)
        .connect_with(
            SqliteConnectOptions::new()
                .filename(directory.join("query.sqlite"))
                .create_if_missing(true)
                .foreign_keys(true)
                .journal_mode(SqliteJournalMode::Wal)
                .busy_timeout(Duration::from_secs(5)),
        )
        .await?;
    MIGRATOR.run(&pool).await?;
    Ok((directory, pool))
}

#[tokio::test]
async fn retention_floors_survive_reopen_and_only_advance_atomically() -> Result<()> {
    let (directory, pool) = database().await?;
    let floors: Vec<(String, i64)> = sqlx::query_as(
        "SELECT stream,first_retained FROM fleet_query_retention_floors ORDER BY stream",
    )
    .fetch_all(&pool)
    .await?;
    assert_eq!(
        floors,
        [("fleet_events".to_owned(), 1), ("samples".to_owned(), 1)]
    );
    for statement in [
        "UPDATE fleet_query_retention_floors SET first_retained=0 WHERE stream='samples'",
        "UPDATE fleet_query_retention_floors SET stream='unknown' WHERE stream='samples'",
        "DELETE FROM fleet_query_retention_floors WHERE stream='samples'",
        "INSERT INTO fleet_query_retention_floors VALUES ('unknown',1)",
    ] {
        assert!(sqlx::query(statement).execute(&pool).await.is_err());
    }
    let mut tx = pool.begin().await?;
    sqlx::query("UPDATE fleet_query_retention_floors SET first_retained=40 WHERE stream='samples'")
        .execute(&mut *tx)
        .await?;
    tx.rollback().await?;
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT first_retained FROM fleet_query_retention_floors WHERE stream='samples'"
        )
        .fetch_one(&pool)
        .await?,
        1
    );
    sqlx::query("UPDATE fleet_query_retention_floors SET first_retained=40 WHERE stream='samples'")
        .execute(&pool)
        .await?;
    assert!(
        sqlx::query(
            "UPDATE fleet_query_retention_floors SET first_retained=39 WHERE stream='samples'"
        )
        .execute(&pool)
        .await
        .is_err()
    );
    pool.close().await;
    let reopened = SqlitePool::connect_with(
        SqliteConnectOptions::new()
            .filename(directory.join("query.sqlite"))
            .foreign_keys(true),
    )
    .await?;
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT first_retained FROM fleet_query_retention_floors WHERE stream='samples'"
        )
        .fetch_one(&reopened)
        .await?,
        40
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT first_retained FROM fleet_query_retention_floors WHERE stream='fleet_events'"
        )
        .fetch_one(&reopened)
        .await?,
        1
    );
    reopened.close().await;
    std::fs::remove_dir_all(directory)?;
    Ok(())
}
