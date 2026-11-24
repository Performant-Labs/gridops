//! Contract test for #190 (job categories) in `gridops-api`, written by
//! #197: a GitHub `workflow_job` webhook stores the job's category, from
//! the seeded default rules (`docs/design/fleet-health.md`, "Job
//! categories").
//!
//! #190 removes the `#[ignore = "contract: #190"]` marker and makes this
//! pass; it changes nothing else here (`Fleet health guard`). #197 already
//! binds `classifier_cache::category_for_insert` in `process_workflow_job`;
//! its stub answers `None`, so the column stays NULL until #190 fills it.
//!
//! It classifies through the process-wide cache, so it holds
//! `testdata::CLASSIFIER_TEST_LOCK`. Mounted from `webhooks.rs`.

use anyhow::Result;
use gridops_core::{connect_database_path, fleet_health::testdata};
use serde_json::json;

use super::process_workflow_job;

#[tokio::test]
async fn a_github_job_webhook_stores_the_expected_category() -> Result<()> {
    let _serial = testdata::CLASSIFIER_TEST_LOCK.lock().await;
    let directory =
        std::env::temp_dir().join(format!("gridops-webhook-category-{}", uuid::Uuid::new_v4()));
    let database = connect_database_path(&directory.join("gridops.sqlite")).await?;
    sqlx::raw_sql(
        r#"
        INSERT INTO installations (id,account_id,account_login,account_type,target_type,repository_selection,created_at,updated_at)
          VALUES (1,1,'acme','Organization','Organization','all',1,1);
        INSERT INTO repositories (id,installation_id,owner,name,full_name,private,default_branch,html_url,last_synced_at,created_at,updated_at)
          VALUES (10,1,'acme','app','acme/app',0,'main','https://github.com/acme/app',1,1,1);
        INSERT INTO workflow_runs (id,repository_id,workflow_name,run_number,event,status,head_sha,html_url,github_created_at,github_updated_at,created_at,updated_at)
          VALUES (20,10,'CI',1,'push','in_progress','abc','https://github.com/acme/app/actions/runs/20',1,1,1,1);
        "#,
    )
    .execute(&database)
    .await?;

    for (id, name, expected) in [
        (30, "unit tests (1/3)", Some("test")),
        (31, "docker image", Some("build")),
        (32, "clippy", Some("check")),
        (33, "publish pages", Some("deploy")),
        (34, "notify slack", Some("other")),
    ] {
        let payload = json!({
            "repository": { "id": 10, "full_name": "acme/app" },
            "workflow_job": {
                "id": id, "run_id": 20, "name": name, "status": "queued",
                "workflow_name": "CI", "labels": ["self-hosted", "gridops"],
                "html_url": format!("https://github.com/acme/app/actions/runs/20/job/{id}")
            }
        });
        let payload = payload
            .as_object()
            .ok_or_else(|| anyhow::anyhow!("test payload is not an object"))?;
        process_workflow_job(&database, Some("queued"), payload).await?;
        let stored: Option<String> =
            sqlx::query_scalar("SELECT category FROM workflow_jobs WHERE id=?")
                .bind(id)
                .fetch_one(&database)
                .await?;
        assert_eq!(stored.as_deref(), expected, "{name}");
    }

    database.close().await;
    std::fs::remove_dir_all(directory)?;
    Ok(())
}
