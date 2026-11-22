//! Contract tests for #190 (job categories) in `gridops-core`, written by
//! #197: the classifier cache and GitLab ingestion. #190's GitHub webhook
//! test is in `gridops-api` and its poll, backfill and reclassify tests are
//! in `gridops-reconciler` (one contract-test file per crate, A warn 13).
//!
//! #190 removes the `#[ignore = "contract: #190"]` markers and makes these
//! pass; it changes nothing else here (`Fleet health guard`).
//!
//! The cache (#190 step 1): one compiled `Classifier` per process, rebuilt
//! when `MAX(updated_at)` or `COUNT(*)` of `job_category_rules` changes,
//! checked at most once a minute of the `now` it is given (and again
//! whenever `now` is earlier than the last check).
//!
//! Tests that classify through the process-wide cache hold
//! `testdata::CLASSIFIER_TEST_LOCK`, so no other test's database moves the
//! cache under them.
//!
//! Mounted from `fleet_health/mod.rs` (#197-owned).

use std::sync::Arc;

use anyhow::Result;
use sqlx::SqlitePool;

use super::{JobCategory, classifier_cache::ClassifierCache, testdata};
use crate::{
    fleet::ExternalId,
    gitlab::{
        JobStatus,
        jobs::{JobObservation, record_gitlab_job},
    },
};

const MINUTE: i64 = 60_000;

async fn database() -> Result<(std::path::PathBuf, SqlitePool)> {
    let directory =
        std::env::temp_dir().join(format!("gridops-classifier-{}", uuid::Uuid::new_v4()));
    let db = crate::connect_database_path(&directory.join("db.sqlite")).await?;
    Ok((directory, db))
}

async fn close(directory: std::path::PathBuf, db: SqlitePool) -> Result<()> {
    db.close().await;
    std::fs::remove_dir_all(directory)?;
    Ok(())
}

fn ext(value: u64) -> Result<ExternalId> {
    ExternalId::new(value).map_err(|error| anyhow::anyhow!("{error}"))
}

#[tokio::test]
#[ignore = "contract: #190"]
async fn a_rule_change_is_picked_up_within_a_minute_and_nothing_recompiles_otherwise() -> Result<()>
{
    let (directory, db) = database().await?;
    let mut conn = db.acquire().await?;
    let cache = ClassifierCache::new();
    let start = 10_000 * MINUTE;

    let first = cache.current(&mut conn, start).await?;
    assert_eq!(cache.compilations(), 1);
    assert_eq!(
        first.classify("smoke suite", None, None, &[]),
        JobCategory::Other
    );

    // Nothing changed: the same classifier, nothing compiled, however often
    // it is asked and whether or not the minute has passed.
    for offset in [1, 30_000, MINUTE, 3 * MINUTE] {
        let again = cache.current(&mut conn, start + offset).await?;
        assert!(Arc::ptr_eq(&first, &again), "recompiled at +{offset} ms");
    }
    assert_eq!(
        cache.compilations(),
        1,
        "nothing changed, nothing recompiled"
    );

    // A rule is added. It shows within a minute of the last check, and the
    // cache compiles exactly once for it.
    let last_check = start + 3 * MINUTE;
    sqlx::query("INSERT INTO job_category_rules (position,category,field,pattern,created_at,updated_at) VALUES (9,'test','job_name','smoke',?,?)")
        .bind(last_check + 1)
        .bind(last_check + 1)
        .execute(&mut *conn)
        .await?;
    let picked_up = cache.current(&mut conn, last_check + MINUTE).await?;
    assert_eq!(
        picked_up.classify("smoke suite", None, None, &[]),
        JobCategory::Test,
        "a rule change shows within a minute"
    );
    assert_eq!(cache.compilations(), 2);
    let steady = cache.current(&mut conn, last_check + 5 * MINUTE).await?;
    assert!(Arc::ptr_eq(&picked_up, &steady));
    assert_eq!(cache.compilations(), 2);

    // An edit that keeps the count but moves MAX(updated_at) is a change too.
    sqlx::query(
        "UPDATE job_category_rules SET pattern='nothing-matches',updated_at=? WHERE position=9",
    )
    .bind(last_check + 6 * MINUTE)
    .execute(&mut *conn)
    .await?;
    let edited = cache.current(&mut conn, last_check + 7 * MINUTE).await?;
    assert_eq!(
        edited.classify("smoke suite", None, None, &[]),
        JobCategory::Other
    );
    assert_eq!(cache.compilations(), 3);

    // A deletion that leaves MAX(updated_at) alone changes the count.
    sqlx::query("DELETE FROM job_category_rules WHERE position=0")
        .execute(&mut *conn)
        .await?;
    let deleted = cache.current(&mut conn, last_check + 9 * MINUTE).await?;
    assert_eq!(
        deleted.classify("unit tests", None, None, &[]),
        JobCategory::Other,
        "the default `test` rule is gone"
    );
    assert_eq!(cache.compilations(), 4);
    drop(conn);
    close(directory, db).await
}

#[tokio::test]
#[ignore = "contract: #190"]
async fn a_gitlab_job_event_stores_the_expected_category() -> Result<()> {
    let _serial = testdata::CLASSIFIER_TEST_LOCK.lock().await;
    let (directory, db) = database().await?;
    sqlx::query("INSERT INTO gitlab_connections (id,name,base_url,scope_kind,scope_id,scope_path,access_token_key,webhook_secret_key,created_at,updated_at) VALUES ('gl','gl','https://gitlab.example.test','project',7,'team/app','gl.token','gl.webhook',1,1)")
        .execute(&db)
        .await?;
    let now = crate::now_millis();
    let docker = vec!["docker".to_owned()];
    let mut conn = db.acquire().await?;
    for (job_id, name, expected) in [
        (1, "rspec 1/3", "test"),
        (2, "docker-image", "build"),
        (3, "rubocop lint", "check"),
        (4, "pages", "deploy"),
        (5, "notify", "other"),
    ] {
        record_gitlab_job(
            &mut conn,
            &JobObservation {
                connection_id: "gl",
                project_id: ext(7)?,
                job_id: ext(job_id)?,
                pipeline_id: ext(1)?,
                status: JobStatus::Pending,
                status_changed_at: now,
                retries_count: None,
                tag_list: Some(&docker),
                gitlab_runner_id: None,
                job_name: Some(name),
                project_path: Some("team/app"),
            },
            now,
        )
        .await?;
        let stored: Option<String> = sqlx::query_scalar(
            "SELECT category FROM gitlab_jobs WHERE connection_id='gl' AND job_id=?",
        )
        .bind(i64::try_from(job_id)?)
        .fetch_one(&mut *conn)
        .await?;
        assert_eq!(stored.as_deref(), Some(expected), "{name}");
    }
    drop(conn);
    close(directory, db).await
}
