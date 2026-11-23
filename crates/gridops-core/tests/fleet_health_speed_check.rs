//! #197 section 7: the database speed check, in its own test binary (A warn
//! 8) so its deliberately slow statements can't fail another test, and no
//! other test's statements can fail these.
//!
//! `speed_check::check(db, work)` runs `work` and fails when any statement
//! takes over 100 ms (the #179 slow-statement log, `db.rs`) or any write
//! transaction holds the lock over 50 ms. The synthetic dataset builder
//! (`testdata::build_week`) gives the wave-1 contract tests 7 days of the
//! five fixture machines at realistic volume.
//!
//! This binary needs core's `test-support` feature under a plain
//! `cargo test -p gridops-core` (for example a self dev-dependency with the
//! feature); it must not be skipped silently.

use std::time::{Duration, Instant};

use anyhow::Result;
use gridops_core::{
    connect_database_path,
    fleet_health::{
        speed_check,
        testdata::{self, FLEET_HOSTS},
    },
};
use sqlx::SqlitePool;

async fn database() -> Result<(std::path::PathBuf, SqlitePool)> {
    let directory =
        std::env::temp_dir().join(format!("gridops-speed-check-{}", uuid::Uuid::new_v4()));
    let pool = connect_database_path(&directory.join("db.sqlite")).await?;
    Ok((directory, pool))
}

async fn close(directory: std::path::PathBuf, pool: SqlitePool) -> Result<()> {
    pool.close().await;
    std::fs::remove_dir_all(directory)?;
    Ok(())
}

/// A read that takes well over 100 ms on any machine.
const SLOW_QUERY: &str = "WITH RECURSIVE c(x) AS (SELECT 1 UNION ALL SELECT x+1 FROM c WHERE x<3000000) SELECT COUNT(*) FROM c";

#[tokio::test]
async fn a_deliberately_slow_statement_fails_the_check() -> Result<()> {
    let (directory, pool) = database().await?;
    let outcome = speed_check::check(&pool, async {
        let count: i64 = sqlx::query_scalar(SLOW_QUERY).fetch_one(&pool).await?;
        Ok(count)
    })
    .await;
    let error = match outcome {
        Ok(count) => anyhow::bail!("a slow statement passed the check (returned {count})"),
        Err(error) => format!("{error:#}"),
    };
    assert!(
        error.to_lowercase().contains("slow statement"),
        "the failure names the slow statement: {error}"
    );
    close(directory, pool).await
}

#[tokio::test]
async fn a_long_write_transaction_fails_the_check() -> Result<()> {
    let (directory, pool) = database().await?;
    let outcome = speed_check::check(&pool, async {
        let mut tx = pool.begin().await?;
        sqlx::query("INSERT INTO settings (key,value,updated_at) VALUES ('speed-check','1',1)")
            .execute(&mut *tx)
            .await?;
        tokio::time::sleep(Duration::from_millis(150)).await;
        tx.commit().await?;
        Ok(())
    })
    .await;
    let error = match outcome {
        Ok(()) => anyhow::bail!("a 150 ms write transaction passed the check"),
        Err(error) => format!("{error:#}"),
    };
    assert!(
        error.to_lowercase().contains("write transaction"),
        "the failure names the write transaction: {error}"
    );
    close(directory, pool).await
}

#[tokio::test]
async fn fast_work_passes_and_its_value_and_errors_come_through() -> Result<()> {
    let (directory, pool) = database().await?;
    let value = speed_check::check(&pool, async {
        for index in 0..20 {
            let mut tx = pool.begin().await?;
            sqlx::query("INSERT INTO settings (key,value,updated_at) VALUES (?,'1',1)")
                .bind(format!("fast-{index}"))
                .execute(&mut *tx)
                .await?;
            tx.commit().await?;
        }
        Ok(41 + 1)
    })
    .await?;
    assert_eq!(value, 42);
    let fail = value == 42;
    let failed = speed_check::check(&pool, async move {
        if fail {
            anyhow::bail!("work failed");
        }
        Ok(())
    })
    .await;
    assert!(
        failed.is_err_and(|error| format!("{error:#}").contains("work failed")),
        "the work's own error is returned"
    );
    close(directory, pool).await
}

/// The synthetic week (#197 section 7): the five fixture machines, about
/// 50,000 jobs and a sample every 30 s per machine, written in batches fast
/// enough that the whole speed-check set stays under 60 s in CI, and itself
/// within the lock rules it is used to enforce.
#[tokio::test]
async fn the_synthetic_week_has_the_fixture_machines_at_realistic_volume() -> Result<()> {
    let (directory, pool) = database().await?;
    let end = 1_800_000_000_000_i64; // a whole hour: 500,000 hours since the epoch
    let started = Instant::now();
    let week = speed_check::check(&pool, testdata::build_week(&pool, end)).await?;
    let elapsed = started.elapsed();
    assert!(
        elapsed < Duration::from_secs(20),
        "building the week took {elapsed:?}"
    );

    let hosts: Vec<(String, i64)> = sqlx::query_as(
        "SELECT h.name,d.cpu_millis FROM fleet_hosts h JOIN host_resource_domains d ON d.host_id=h.id AND d.parent_domain_id IS NULL WHERE h.deleted_at IS NULL ORDER BY h.name",
    )
    .fetch_all(&pool)
    .await?;
    let mut expected: Vec<(String, i64)> = FLEET_HOSTS
        .iter()
        .map(|(name, cpu)| ((*name).to_owned(), i64::from(*cpu)))
        .collect();
    expected.sort();
    assert_eq!(hosts, expected);

    let week_start = end - 7 * 86_400_000;
    let jobs: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM workflow_jobs WHERE completed_at>=? AND completed_at<?",
    )
    .bind(week_start)
    .bind(end)
    .fetch_one(&pool)
    .await?;
    assert!(
        (45_000..=55_000).contains(&jobs),
        "about 50,000 jobs in the week, got {jobs}"
    );
    assert_eq!(usize::try_from(jobs)?, week.jobs);
    let samples: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM host_samples WHERE observed_at>=? AND observed_at<?",
    )
    .bind(week_start)
    .bind(end)
    .fetch_one(&pool)
    .await?;
    // One every 30 s for 7 days, for each machine (MacBookPro included).
    assert_eq!(samples, 5 * 7 * 24 * 120);
    assert_eq!(usize::try_from(samples)?, week.samples);
    let placed: i64 = sqlx::query_scalar(
        "SELECT COUNT(DISTINCT p.host_id) FROM workflow_jobs j JOIN runners r ON r.github_runner_id=j.runner_id JOIN workload_placements p ON p.runner_id=r.id WHERE j.completed_at>=?",
    )
    .bind(week_start)
    .fetch_one(&pool)
    .await?;
    assert_eq!(placed, 4, "jobs run on every machine with a CPU budget");
    close(directory, pool).await
}
