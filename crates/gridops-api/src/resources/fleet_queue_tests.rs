//! #197 (D-3(b), A warns 17 and 24): the fleet-wide queue breakdown #201's
//! `collect_live` reads. It is the Overview's own split (`split_queue` over
//! `waiting_job_pools`) with the job query not filtered by user, so for a
//! user who sees every installation it gives exactly the Overview's numbers,
//! and with no user at all it still counts every queued job. It also counts
//! the queued jobs by category, NULL as `other`. The per-user path is
//! unchanged (its tests in `overview_queue.rs` stay as they are).
//!
//! Mounted from `resources.rs`.

use anyhow::Result;
use gridops_core::{BitbucketClient, Config, GitHubClient, Vault, fleet_health::JobCategory};
use std::collections::BTreeMap;

use super::{fleet_queue_breakdown, overview_queue::queue_breakdown};
use crate::{auth::AuthUser, state::AppState};

#[tokio::test]
async fn the_fleet_wide_queue_is_the_overviews_split_over_every_installation() -> Result<()> {
    let path = std::env::temp_dir().join(format!(
        "gridops-fleet-queue-{}.sqlite",
        uuid::Uuid::new_v4()
    ));
    let database = gridops_core::connect_database_path(&path).await?;
    let now = 1_800_000_000_000_i64;
    sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
        r#"
        INSERT INTO installations (id,account_id,account_login,account_type,target_type,repository_selection,created_at,updated_at)
          VALUES (1,1,'one','Organization','Organization','all',1,1),(2,2,'two','Organization','Organization','all',1,1);
        INSERT INTO repositories (id,installation_id,owner,name,full_name,private,default_branch,html_url,last_synced_at,created_at,updated_at)
          VALUES (10,1,'one','app','one/app',0,'main','https://example.test/one/app',1,1,1),
                 (20,2,'two','web','two/web',0,'main','https://example.test/two/web',1,1,1);
        INSERT INTO runner_pools (id,installation_id,repository_id,name,scope,labels,image,desired_count,max_count,queue_scale_factor,created_at,updated_at)
          VALUES ('pool-a',1,10,'pool-a','repository','["pool-a"]','runner:latest',0,10,1,1,1);
        INSERT OR IGNORE INTO runner_pool_installations (pool_id,installation_id) VALUES ('pool-a',1);
        INSERT INTO workflow_runs (id,repository_id,workflow_name,run_number,event,status,head_sha,html_url,github_created_at,github_updated_at,created_at,updated_at)
          VALUES (100,10,'CI',1,'push','in_progress','a','https://example.test/r/100',1,1,1,1),
                 (200,20,'CI',1,'push','queued','b','https://example.test/r/200',1,1,1,1),
                 (300,20,'CI',2,'push','completed','c','https://example.test/r/300',1,1,1,1);
        INSERT INTO workflow_jobs (id,run_id,name,status,labels,html_url,created_at,updated_at,category) VALUES
          (1,100,'unit tests','queued','["self-hosted","pool-a"]','https://example.test/j/1',{a},{a},'test'),
          (2,100,'e2e','queued','["self-hosted","pool-a"]','https://example.test/j/2',{b},{b},'test'),
          (3,100,'misc','queued','["self-hosted","pool-a"]','https://example.test/j/3',{c},{c},NULL),
          (4,200,'image','waiting','["self-hosted","gpu"]','https://example.test/j/4',{d},{d},'build'),
          (5,200,'done','completed','["self-hosted","pool-a"]','https://example.test/j/5',{d},{d},'test'),
          (6,300,'orphan','queued','["self-hosted","pool-a"]','https://example.test/j/6',{d},{d},'deploy');
        "#,
        a = now - 600_000,
        b = now - 300_000,
        c = now - 60_000,
        d = now - 30_000,
    )))
    .execute(&database)
    .await?;

    // Nobody is signed in: the fleet-wide breakdown still sees every job.
    let fleet = fleet_queue_breakdown(&database, now).await?;
    assert_eq!(
        fleet.queue.total(),
        4,
        "four open jobs in unfinished runs (a completed job and a finished run's job don't count)"
    );
    assert_eq!(
        fleet.by_category,
        BTreeMap::from([
            (JobCategory::Test, 2),
            (JobCategory::Build, 1),
            (JobCategory::Other, 1),
        ]),
        "by category, NULL counted as other"
    );
    assert_eq!(
        fleet.by_category.values().sum::<i64>(),
        fleet.queue.total(),
        "the categories add up to the queue"
    );

    // A user who sees both installations gets the same split.
    let config = Config::from_env()?;
    let state = AppState::new(
        config.clone(),
        database.clone(),
        Vault::new([3_u8; 32], b"fleet-queue-test".to_vec()),
        GitHubClient::new(config)?,
        BitbucketClient::new()?,
    )?;
    sqlx::raw_sql(
        "INSERT INTO users (id,github_id,login,role,access_token,last_login_at,created_at,updated_at) VALUES ('viewer',7,'viewer','member','',1,1,1);
         INSERT INTO user_installations (user_id,installation_id,permission,created_at) VALUES ('viewer',1,'read',1),('viewer',2,'read',1);",
    )
    .execute(&database)
    .await?;
    let viewer = AuthUser {
        id: "viewer".to_owned(),
        login: "viewer".to_owned(),
        github_id: 7,
        role: "member".to_owned(),
        via_token: None,
    };
    let overview = queue_breakdown(&state, &viewer, now).await?;
    assert_eq!(fleet.queue.waiting, overview.waiting);
    assert_eq!(fleet.queue.manual, overview.manual);
    assert_eq!(fleet.queue.unmatched, overview.unmatched);
    assert_eq!(fleet.queue.unavailable, overview.unavailable);
    assert_eq!(fleet.queue.by_pool, overview.by_pool);
    assert_eq!(
        fleet.queue.oldest_waiting_millis,
        overview.oldest_waiting_millis
    );
    assert_eq!(fleet.queue.oldest_waiting_job, overview.oldest_waiting_job);
    assert_eq!(fleet.queue.missing_labels, overview.missing_labels);
    assert!(fleet.queue.unmatched >= 1, "the gpu job has no pool");

    database.close().await;
    for suffix in ["", "-shm", "-wal"] {
        let _ = std::fs::remove_file(format!("{}{suffix}", path.display()));
    }
    Ok(())
}
