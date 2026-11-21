//! GitLab demand matching (#148): which pools count a GitLab job, through the
//! public queue views. Only `assigned_queued_jobs` (room and scale) sees
//! GitLab jobs; every GitHub-shaped output stays GitHub-only, but a GitLab job
//! still uses up room in a pool it shares with GitHub jobs (brief A-B2, A-B3,
//! A-B4, A-W4, A2-W5).

use super::*;
use crate::connect_database_path;
use std::fs;

/// A database with installation 1, repository 10 and its run 20 (for GitHub
/// jobs), and two GitLab connections, `conn-a` and `conn-b`, then `extra`.
async fn gitlab_fixture(extra: &str) -> anyhow::Result<(std::path::PathBuf, SqlitePool)> {
    let directory = std::env::temp_dir().join(format!(
        "gridops-gitlab-demand-test-{}",
        uuid::Uuid::new_v4()
    ));
    let database = connect_database_path(&directory.join("gridops.sqlite")).await?;
    sqlx::raw_sql(
        r#"
        INSERT INTO installations (id,account_id,account_login,account_type,target_type,repository_selection,created_at,updated_at)
          VALUES (1,1,'octo-org','Organization','Organization','all',1,1);
        INSERT INTO repositories (id,installation_id,owner,name,full_name,private,default_branch,html_url,last_synced_at,created_at,updated_at)
          VALUES (10,1,'octo-org','gridops','octo-org/gridops',0,'master','https://github.com/octo-org/gridops',1,1,1);
        INSERT INTO workflow_runs (id,repository_id,workflow_name,run_number,event,status,head_sha,html_url,github_created_at,github_updated_at,created_at,updated_at)
          VALUES (20,10,'CI',1,'push','queued','abc123','https://github.com/octo-org/gridops/actions/runs/20',1,1,1,1);
        INSERT INTO gitlab_connections (id,name,base_url,scope_kind,scope_id,scope_path,access_token_key,webhook_secret_key,created_at,updated_at)
          VALUES
          ('conn-a','a','https://gitlab-a.example.test','project',11,'group/a','gitlab.conn-a.token','gitlab.conn-a.webhook',1,1),
          ('conn-b','b','https://gitlab-b.example.test','group',12,'group-b','gitlab.conn-b.token','gitlab.conn-b.webhook',1,1);
        "#,
    )
    .execute(&database)
    .await?;
    sqlx::raw_sql(sqlx::AssertSqlSafe(extra.to_owned()))
        .execute(&database)
        .await?;
    Ok((directory, database))
}

fn json_list(values: &[&str]) -> String {
    serde_json::to_string(values).unwrap_or_else(|_| "[]".into())
}

/// An autoscaling pool configuring `labels`, linked to `connections`. Its
/// scope matches no GitHub repository, so it only ever sees GitLab jobs.
fn gitlab_pool(
    id: &str,
    created_at: i64,
    max_count: i64,
    labels: &[&str],
    connections: &[&str],
) -> String {
    let labels = json_list(labels);
    let links = connections
        .iter()
        .map(|connection| {
            format!(
                "INSERT INTO runner_pool_gitlab_connections (pool_id,connection_id,created_at) VALUES ('{id}','{connection}',1);"
            )
        })
        .collect::<Vec<_>>()
        .concat();
    format!(
        "INSERT INTO runner_pools (id,installation_id,repository_id,name,scope,labels,image,max_count,autoscaling_enabled,created_at,updated_at)
          VALUES ('{id}',1,NULL,'{id}','repository','{labels}','runner:latest',{max_count},1,{created_at},{created_at});{links}"
    )
}

/// An organization pool on installation 1 carrying `shared`, linked to
/// `conn-a`: it serves GitHub jobs of repository 10 and GitLab jobs of conn-a.
fn shared_pool(id: &str, created_at: i64, max_count: i64) -> String {
    format!(
        "INSERT INTO runner_pools (id,installation_id,repository_id,name,scope,labels,image,max_count,autoscaling_enabled,created_at,updated_at)
          VALUES ('{id}',1,NULL,'{id}','organization','[\"shared\"]','runner:latest',{max_count},1,{created_at},{created_at});
        INSERT INTO runner_pool_gitlab_connections (pool_id,connection_id,created_at) VALUES ('{id}','conn-a',1);"
    )
}

/// A `gitlab_jobs` row as #140 or the poll records it: project 11, a fresh
/// UUID id and the contract `tag_list` encoding, created at `created_at`.
fn gitlab_job(
    connection: &str,
    job_id: i64,
    status: &str,
    tags: &[&str],
    created_at: i64,
) -> String {
    format!(
        "INSERT INTO gitlab_jobs (id,connection_id,project_id,job_id,pipeline_id,status,tag_list,status_changed_at,created_at,updated_at)
          VALUES ('{}','{connection}',11,{job_id},1,'{status}','{}',{created_at},{created_at},{created_at});",
        uuid::Uuid::new_v4(),
        json_list(tags),
    )
}

/// A GitHub job of run 20 with `labels`.
fn github_job(id: i64, status: &str, labels: &[&str], created_at: i64) -> String {
    format!(
        "INSERT INTO workflow_jobs (id,run_id,name,status,labels,html_url,created_at,updated_at)
          VALUES ({id},20,'job-{id}','{status}','{}','https://github.com/octo-org/gridops/actions/runs/20/job/{id}',{created_at},{created_at});",
        json_list(labels),
    )
}

/// `count` busy GitLab runners in the pool.
fn busy_gitlab_runners(pool: &str, count: usize) -> String {
    (0..count)
        .map(|index| {
            format!(
                "INSERT INTO runners (id,pool_id,name,status,busy,ci_platform,gitlab_connection_id,gitlab_runner_id,created_at,updated_at)
                  VALUES ('{pool}-busy-{index}','{pool}','{pool}-busy-{index}','busy',1,'gitlab','conn-a',{},1,1);",
                900 + index
            )
        })
        .collect::<Vec<_>>()
        .concat()
}

async fn demand(database: &SqlitePool, pool: &str) -> anyhow::Result<i64> {
    Ok(assigned_queued_jobs(database, pool).await?)
}

async fn finish(directory: std::path::PathBuf, database: SqlitePool) -> anyhow::Result<()> {
    database.close().await;
    fs::remove_dir_all(directory)?;
    Ok(())
}

#[tokio::test]
async fn a_pending_gitlab_job_matches_a_pool_only_when_the_pool_configures_every_tag()
-> anyhow::Result<()> {
    // All tags present (in any case) match; one tag missing does not.
    let (directory, database) = gitlab_fixture(&format!(
        "{}{}{}{}",
        gitlab_pool("gl", 1, 10, &["docker", "gpu"], &["conn-a"]),
        gitlab_job("conn-a", 501, "pending", &["docker"], 1),
        gitlab_job("conn-a", 502, "pending", &["DOCKER", "Gpu"], 2),
        gitlab_job("conn-a", 503, "pending", &["docker", "gpu", "arm-board"], 3),
    ))
    .await?;

    assert_eq!(demand(&database, "gl").await?, 2);

    finish(directory, database).await
}

#[tokio::test]
async fn a_gitlab_job_is_counted_only_by_pools_linked_to_its_connection() -> anyhow::Result<()> {
    // 'other' is older and configures the tag, but serves conn-b only. The
    // job's project plays no part: both of conn-a's jobs go to 'gl'.
    let (directory, database) = gitlab_fixture(&format!(
        "{}{}{}{}",
        gitlab_pool("other", 1, 10, &["docker"], &["conn-b"]),
        gitlab_pool("gl", 2, 10, &["docker"], &["conn-a"]),
        gitlab_job("conn-a", 501, "pending", &["docker"], 1),
        "INSERT INTO gitlab_jobs (id,connection_id,project_id,job_id,pipeline_id,status,tag_list,status_changed_at,created_at,updated_at)
          VALUES ('6a1b2c3d-0000-4000-8000-000000000502','conn-a',99,502,1,'pending','[\"docker\"]',2,2,2);",
    ))
    .await?;

    assert_eq!(demand(&database, "gl").await?, 2);
    assert_eq!(demand(&database, "other").await?, 0);

    finish(directory, database).await
}

#[tokio::test]
async fn an_untagged_gitlab_job_matches_only_pools_that_run_untagged() -> anyhow::Result<()> {
    // 'tagged' is older and has room but does not run untagged jobs.
    let (directory, database) = gitlab_fixture(&format!(
        "{}{}{}{}",
        gitlab_pool("tagged", 1, 10, &["docker"], &["conn-a"]),
        gitlab_pool("untagged", 2, 10, &["anything"], &["conn-a"]),
        "UPDATE runner_pools SET run_untagged=1 WHERE id='untagged';",
        gitlab_job("conn-a", 501, "pending", &[], 1),
    ))
    .await?;

    assert_eq!(demand(&database, "untagged").await?, 1);
    assert_eq!(demand(&database, "tagged").await?, 0);

    // With no pool running untagged jobs, nobody counts it.
    sqlx::raw_sql("UPDATE runner_pools SET run_untagged=0 WHERE id='untagged';")
        .execute(&database)
        .await?;
    assert_eq!(demand(&database, "untagged").await?, 0);
    assert_eq!(demand(&database, "tagged").await?, 0);

    finish(directory, database).await
}

#[tokio::test]
async fn gitlab_tags_match_configured_labels_only_never_implied_system_or_catch_all_labels()
-> anyhow::Result<()> {
    // A-B4: #139 registers a GitLab runner with the pool's configured labels
    // and nothing else, so no GitHub platform, OS, arch or catch-all label is
    // implied. An exclusive pool is matched like any other.
    let host_arch = runner_arch_label();
    for label in ["self-hosted", "self-hosted-linux", "linux", host_arch] {
        let (directory, database) = gitlab_fixture(&format!(
            "{}{}{}{}",
            gitlab_pool("plain", 1, 10, &["docker"], &["conn-a"]),
            gitlab_pool("exclusive", 2, 10, &["docker"], &["conn-a"]),
            "INSERT INTO runner_pool_exclusive_labels (pool_id) VALUES ('exclusive');",
            gitlab_job("conn-a", 501, "pending", &[label], 1),
        ))
        .await?;
        assert_eq!(demand(&database, "plain").await?, 0, "{label}");
        assert_eq!(demand(&database, "exclusive").await?, 0, "{label}");

        // A non-exclusive pool that configures the label does match it.
        sqlx::raw_sql(sqlx::AssertSqlSafe(gitlab_pool(
            "configures",
            3,
            10,
            &[label],
            &["conn-a"],
        )))
        .execute(&database)
        .await?;
        assert_eq!(demand(&database, "configures").await?, 1, "{label}");
        finish(directory, database).await?;
    }

    // An exclusive pool matches a job whose tags it configures.
    let (directory, database) = gitlab_fixture(&format!(
        "{}{}{}",
        gitlab_pool("exclusive", 1, 10, &["docker"], &["conn-a"]),
        "INSERT INTO runner_pool_exclusive_labels (pool_id) VALUES ('exclusive');",
        gitlab_job("conn-a", 501, "pending", &["docker"], 1),
    ))
    .await?;
    assert_eq!(demand(&database, "exclusive").await?, 1);

    finish(directory, database).await
}

#[tokio::test]
async fn a_pool_with_no_runnable_platform_never_owns_a_gitlab_job() -> anyhow::Result<()> {
    // A2-W5: 'nowhere' is older, has room and configures the tag, but has no
    // provider to run on, so the job goes to 'docker'.
    let (directory, database) = gitlab_fixture(&format!(
        "{}{}{}{}",
        gitlab_pool("nowhere", 1, 10, &["docker"], &["conn-a"]),
        "UPDATE runner_pools SET providers='[]' WHERE id='nowhere';",
        gitlab_pool("docker", 2, 10, &["docker"], &["conn-a"]),
        gitlab_job("conn-a", 501, "pending", &["docker"], 1),
    ))
    .await?;

    assert_eq!(demand(&database, "nowhere").await?, 0);
    assert_eq!(demand(&database, "docker").await?, 1);

    finish(directory, database).await
}

#[tokio::test]
async fn only_pending_gitlab_jobs_are_scaler_demand() -> anyhow::Result<()> {
    // A-B3 and D8: `running` is counted through the busy runner, the waiting
    // statuses are display only, and manual, scheduled, unknown and terminal
    // jobs are never read.
    let others = [
        "created",
        "waiting_for_resource",
        "preparing",
        "running",
        "manual",
        "scheduled",
        "unknown",
        "success",
        "failed",
        "canceled",
        "skipped",
    ];
    let jobs = others
        .iter()
        .zip(600..)
        .map(|(status, job_id)| gitlab_job("conn-a", job_id, status, &["docker"], job_id))
        .collect::<Vec<_>>()
        .concat();
    let (directory, database) = gitlab_fixture(&format!(
        "{}{jobs}",
        gitlab_pool("gl", 1, 10, &["docker"], &["conn-a"]),
    ))
    .await?;
    assert_eq!(demand(&database, "gl").await?, 0);
    assert!(queued_job_ownership(&database).await?.is_empty());

    sqlx::raw_sql(sqlx::AssertSqlSafe(gitlab_job(
        "conn-a",
        700,
        "pending",
        &["docker"],
        700,
    )))
    .execute(&database)
    .await?;
    assert_eq!(demand(&database, "gl").await?, 1);

    finish(directory, database).await
}

#[tokio::test]
async fn waiting_gitlab_jobs_take_display_room_but_other_unready_statuses_take_none()
-> anyhow::Result<()> {
    // 'shared' has room for one job. A GitLab job in the display tier
    // (created, waiting_for_resource, preparing) that is older than GitHub's
    // waiting job 100 takes that room for display, so 100 is shown under
    // 'overflow'; any other non-pending GitLab job takes none. No such job is
    // scaler demand.
    let (directory, database) = gitlab_fixture(&format!(
        "{}{}{}",
        shared_pool("shared", 1, 1),
        shared_pool("overflow", 2, 10),
        github_job(100, "waiting", &["self-hosted", "shared"], 2),
    ))
    .await?;
    let shown_under = |status: &'static str| {
        let database = database.clone();
        async move {
            sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
                "DELETE FROM gitlab_jobs;{}",
                gitlab_job("conn-a", 501, status, &["shared"], 1)
            )))
            .execute(&database)
            .await?;
            assert_eq!(demand(&database, "shared").await?, 0, "{status}");
            assert_eq!(demand(&database, "overflow").await?, 0, "{status}");
            anyhow::Ok(
                waiting_job_ownership(&database)
                    .await?
                    .into_iter()
                    .map(|owner| (owner.job_id, owner.pool_id))
                    .collect::<Vec<_>>(),
            )
        }
    };

    for status in ["created", "waiting_for_resource", "preparing"] {
        assert_eq!(
            shown_under(status).await?,
            vec![(100, "overflow".to_owned())],
            "{status}"
        );
    }
    for status in ["running", "manual", "scheduled", "unknown", "success"] {
        assert_eq!(
            shown_under(status).await?,
            vec![(100, "shared".to_owned())],
            "{status}"
        );
    }

    finish(directory, database).await
}

#[tokio::test]
async fn github_and_gitlab_jobs_share_one_ordered_queue_and_only_github_jobs_are_listed()
-> anyhow::Result<()> {
    // A-W4: one list ordered by queue time, then GitHub before GitLab.
    // 'shared' has room for one job; the other goes to 'overflow'.
    let (directory, database) = gitlab_fixture(&format!(
        "{}{}{}{}",
        shared_pool("shared", 1, 1),
        shared_pool("overflow", 2, 10),
        gitlab_job("conn-a", 501, "pending", &["shared"], 1),
        github_job(100, "queued", &["self-hosted", "shared"], 2),
    ))
    .await?;

    // The older GitLab job takes 'shared''s room, so the GitHub job overflows.
    assert_eq!(demand(&database, "shared").await?, 1);
    assert_eq!(demand(&database, "overflow").await?, 1);
    let owners = queued_job_ownership(&database).await?;
    assert_eq!(
        owners
            .iter()
            .map(|owner| (owner.job_id, owner.pool_id.as_str()))
            .collect::<Vec<_>>(),
        vec![(100, "overflow")]
    );
    // A-B2: the GitHub-shaped views list GitHub jobs only.
    assert!(
        queued_job_assignments(&database, "shared")
            .await?
            .is_empty()
    );
    let overflow = queued_job_assignments(&database, "overflow").await?;
    assert_eq!(
        overflow
            .iter()
            .map(|job| (job.repository_id, job.queued_at))
            .collect::<Vec<_>>(),
        vec![(10, 2)]
    );

    // Queued at the same time, the GitHub job goes first and takes the room.
    sqlx::raw_sql("UPDATE gitlab_jobs SET created_at=2;")
        .execute(&database)
        .await?;
    assert_eq!(demand(&database, "shared").await?, 1);
    assert_eq!(demand(&database, "overflow").await?, 1);
    let owners = queued_job_ownership(&database).await?;
    assert_eq!(
        owners
            .iter()
            .map(|owner| (owner.job_id, owner.pool_id.as_str()))
            .collect::<Vec<_>>(),
        vec![(100, "shared")]
    );

    finish(directory, database).await
}

#[tokio::test]
async fn a_github_job_and_gitlab_jobs_with_the_same_numeric_id_never_collide() -> anyhow::Result<()>
{
    // GitHub job 100 and GitLab job 100 on each of two connections, all
    // queued at the same moment, are three jobs.
    let (directory, database) = gitlab_fixture(&format!(
        "{}{}{}{}{}",
        shared_pool("both", 1, 10),
        "INSERT INTO runner_pool_gitlab_connections (pool_id,connection_id,created_at) VALUES ('both','conn-b',1);",
        github_job(100, "queued", &["self-hosted", "shared"], 5),
        gitlab_job("conn-a", 100, "pending", &["shared"], 5),
        gitlab_job("conn-b", 100, "pending", &["shared"], 5),
    ))
    .await?;

    assert_eq!(demand(&database, "both").await?, 3);
    let owners = queued_job_ownership(&database).await?;
    assert_eq!(
        owners
            .iter()
            .map(|owner| (owner.job_id, owner.pool_id.as_str(), owner.over_capacity))
            .collect::<Vec<_>>(),
        vec![(100, "both", false)]
    );
    let listed = queued_job_assignments(&database, "both").await?;
    assert_eq!(
        listed
            .iter()
            .map(|job| job.repository_id)
            .collect::<Vec<_>>(),
        vec![10]
    );

    finish(directory, database).await
}

#[tokio::test]
async fn a_saturated_gitlab_pool_hands_pending_jobs_to_the_next_compatible_gitlab_pool()
-> anyhow::Result<()> {
    // 'older' is full (2 busy of max 2); 'paused' and 'manual' never take
    // jobs; 'newer' takes them all, until 'older' frees a runner.
    let (directory, database) = gitlab_fixture(&format!(
        "{}{}{}{}{}{}{}{}{}",
        gitlab_pool("older", 1, 2, &["docker"], &["conn-a"]),
        gitlab_pool("paused", 2, 10, &["docker"], &["conn-a"]),
        gitlab_pool("manual", 3, 10, &["docker"], &["conn-a"]),
        gitlab_pool("newer", 4, 10, &["docker"], &["conn-a"]),
        "UPDATE runner_pools SET paused=1 WHERE id='paused';
         UPDATE runner_pools SET autoscaling_enabled=0 WHERE id='manual';",
        busy_gitlab_runners("older", 2),
        gitlab_job("conn-a", 501, "pending", &["docker"], 1),
        gitlab_job("conn-a", 502, "pending", &["docker"], 2),
        gitlab_job("conn-a", 503, "pending", &["docker"], 3),
    ))
    .await?;

    assert_eq!(demand(&database, "older").await?, 0);
    assert_eq!(demand(&database, "paused").await?, 0);
    assert_eq!(demand(&database, "manual").await?, 0);
    assert_eq!(demand(&database, "newer").await?, 3);

    // One of 'older's runners finishes: it takes back the oldest job.
    sqlx::raw_sql("UPDATE runners SET status='online',busy=0 WHERE id='older-busy-0';")
        .execute(&database)
        .await?;
    assert_eq!(demand(&database, "older").await?, 1);
    assert_eq!(demand(&database, "newer").await?, 2);

    finish(directory, database).await
}

#[tokio::test]
async fn one_gitlab_job_recorded_under_a_project_and_its_group_connection_is_one_job()
-> anyhow::Result<()> {
    // D18: `conn-a` (project 11) and `conn-a-group` (its group, on the same
    // GitLab instance) each record job 501, so it has two `gitlab_jobs` rows.
    // It is one GitLab job: deduplicated on (base_url, job_id), it is demand
    // once, whichever connection's pools take it. (Job 100 on `conn-a` and on
    // `conn-b`, which are different instances, stays two jobs: see
    // `a_github_job_and_gitlab_jobs_with_the_same_numeric_id_never_collide`.)
    let group_connection = "INSERT INTO gitlab_connections (id,name,base_url,scope_kind,scope_id,scope_path,access_token_key,webhook_secret_key,created_at,updated_at)
          VALUES ('conn-a-group','a group','https://gitlab-a.example.test','group',3,'group','gitlab.conn-a-group.token','gitlab.conn-a-group.webhook',1,1);";

    // One pool linked to both connections counts it once.
    let (directory, database) = gitlab_fixture(&format!(
        "{group_connection}{}{}{}",
        gitlab_pool("both", 1, 10, &["docker"], &["conn-a", "conn-a-group"]),
        gitlab_job("conn-a", 501, "pending", &["docker"], 1),
        gitlab_job("conn-a-group", 501, "pending", &["docker"], 2),
    ))
    .await?;
    assert_eq!(demand(&database, "both").await?, 1);
    finish(directory, database).await?;

    // A pool on each connection: the candidate pools are the pools of either
    // connection, and the one job goes to one of them, the older pool first.
    let (directory, database) = gitlab_fixture(&format!(
        "{group_connection}{}{}{}{}",
        gitlab_pool("group-only", 1, 10, &["docker"], &["conn-a-group"]),
        gitlab_pool("project-only", 2, 10, &["docker"], &["conn-a"]),
        gitlab_job("conn-a", 501, "pending", &["docker"], 1),
        gitlab_job("conn-a-group", 501, "pending", &["docker"], 2),
    ))
    .await?;
    assert_eq!(
        (
            demand(&database, "group-only").await?,
            demand(&database, "project-only").await?
        ),
        (1, 0)
    );

    finish(directory, database).await
}

#[tokio::test]
async fn a_duplicated_gitlab_job_takes_the_furthest_status_of_its_rows() -> anyhow::Result<()> {
    // Job 601 is recorded under `conn-a` and under its group connection on the
    // same instance. Its status is the furthest along of its rows: once one
    // row says `running`, a lagging `pending` duplicate is not demand.
    let group_connection = "INSERT INTO gitlab_connections (id,name,base_url,scope_kind,scope_id,scope_path,access_token_key,webhook_secret_key,created_at,updated_at)
          VALUES ('conn-a-group','a group','https://gitlab-a.example.test','group',3,'group','gitlab.conn-a-group.token','gitlab.conn-a-group.webhook',1,1);";
    let (directory, database) = gitlab_fixture(&format!(
        "{group_connection}{}{}{}",
        gitlab_pool("both", 1, 10, &["docker"], &["conn-a", "conn-a-group"]),
        gitlab_job("conn-a", 601, "running", &["docker"], 1),
        gitlab_job("conn-a-group", 601, "pending", &["docker"], 2),
    ))
    .await?;
    assert_eq!(demand(&database, "both").await?, 0);
    finish(directory, database).await?;

    // `pending` beside a lagging `created` duplicate is demand, once.
    let (directory, database) = gitlab_fixture(&format!(
        "{group_connection}{}{}{}",
        gitlab_pool("both", 1, 10, &["docker"], &["conn-a", "conn-a-group"]),
        gitlab_job("conn-a", 601, "created", &["docker"], 1),
        gitlab_job("conn-a-group", 601, "pending", &["docker"], 2),
    ))
    .await?;
    assert_eq!(demand(&database, "both").await?, 1);
    finish(directory, database).await
}
