//! A GitLab pool's fleet targets and runner image (#139: D6 and the image
//! rule of round-2 block 2), over a real migrated database.
//!
//! A GitLab pool is a `scope='gitlab'` pool with `runner_pool_gitlab_connections`
//! rows (#138's shape). Its fleet targets come from its connections: one
//! `gitlab_project` or `gitlab_group` target per connection, linked to the
//! pool's platforms. A Docker platform saved without an image gets the pool's
//! Docker image; a GitLab pool without one gets #147's pinned
//! `GITLAB_RUNNER_IMAGE` (D147-1), since the GitHub stock image cannot run
//! GitLab jobs.

use anyhow::{Context as _, Result};
use gridops_core::{
    connect_database_path,
    fleet::targets::{self, FleetSettingsInput},
};
use serde_json::json;
use sqlx::SqlitePool;

struct Database {
    pool: SqlitePool,
    directory: std::path::PathBuf,
}

impl Database {
    /// An organization installation, a GitHub pool (`gh-pool`) and a GitLab
    /// pool (`gl-pool`) with a project and a group connection, both pools
    /// with a Docker image.
    async fn new() -> Result<Self> {
        let directory =
            std::env::temp_dir().join(format!("gridops-gitlab-pool-{}", uuid::Uuid::new_v4()));
        let pool = connect_database_path(&directory.join("db.sqlite")).await?;
        sqlx::raw_sql(
            r#"
            INSERT INTO installations (id,account_id,account_login,account_type,target_type,repository_selection,created_at,updated_at)
              VALUES (1,1,'octo-org','Organization','Organization','all',1,1);
            INSERT INTO runner_pools (id,installation_id,name,scope,provider,providers,labels,image,docker_image,tart_image,created_at,updated_at)
              VALUES ('gh-pool',1,'github','organization','docker','["docker"]','[]','runner:fixture','runner:fixture','',1,1),
                     ('gl-pool',1,'gitlab','gitlab','docker','["docker"]','["docker"]','registry.example.test/gitlab-runner:v17','registry.example.test/gitlab-runner:v17','',2,2);
            INSERT INTO gitlab_connections (id,name,base_url,scope_kind,scope_id,scope_path,access_token_key,webhook_secret_key,created_at,updated_at)
              VALUES ('conn-project','Acme API','https://gitlab.example.com','project',42,'acme/api','gitlab.conn-project.token','gitlab.conn-project.webhook',1,1),
                     ('conn-group','Acme','https://gitlab.example.com','group',7,'acme','gitlab.conn-group.token','gitlab.conn-group.webhook',2,2);
            INSERT INTO runner_pool_gitlab_connections (pool_id,connection_id,created_at)
              VALUES ('gl-pool','conn-project',1),('gl-pool','conn-group',2);
            "#,
        )
        .execute(&pool)
        .await?;
        Ok(Self { pool, directory })
    }

    async fn finish(self) -> Result<()> {
        self.pool.close().await;
        std::fs::remove_dir_all(self.directory)?;
        Ok(())
    }

    /// The fleet target of a GitLab connection, if one exists:
    /// `(id, kind, scope id)`.
    async fn gitlab_target(&self, connection: &str) -> Result<Option<(String, String, i64)>> {
        Ok(sqlx::query_as(
            "SELECT id,kind,COALESCE(gitlab_project_id,gitlab_group_id) FROM fleet_ci_targets
              WHERE gitlab_connection_id=?",
        )
        .bind(connection)
        .fetch_optional(&self.pool)
        .await?)
    }

    /// Records a `pending` GitLab job of `connection`, tagged `tags`, as the
    /// poll or the webhook records it.
    async fn pending_gitlab_job(&self, connection: &str, job_id: i64, tags: &[&str]) -> Result<()> {
        sqlx::query(
            "INSERT INTO gitlab_jobs (id,connection_id,project_id,job_id,pipeline_id,status,tag_list,status_changed_at,created_at,updated_at)
             VALUES (?,?,42,?,1,'pending',?,10,10,10)",
        )
        .bind(uuid::Uuid::new_v4().to_string())
        .bind(connection)
        .bind(job_id)
        .bind(serde_json::to_string(tags)?)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// A fleet runner of the pool on `profile`, starting and not on a job: a
    /// spare runner that can take a queued job. With `repository` `None` it is
    /// registered to no GitHub repository (an organization GitHub runner or a
    /// GitLab runner); otherwise to that GitHub repository.
    async fn spare_runner(
        &self,
        pool_id: &str,
        profile: &targets::Profile,
        repository: Option<i64>,
    ) -> Result<()> {
        let target = profile
            .target_ids
            .first()
            .context("the platform has a fleet target")?;
        let id = uuid::Uuid::new_v4().to_string();
        sqlx::query(
            "INSERT INTO runners (id,pool_id,target_repository_id,name,provider,status,busy,created_at,updated_at)
             VALUES (?1,?2,?3,?1,'docker','starting',0,20,20)",
        )
        .bind(&id)
        .bind(pool_id)
        .bind(repository)
        .execute(&self.pool)
        .await?;
        sqlx::query(
            "INSERT INTO fleet_operations (id,kind,principal_kind,principal_scope,idempotency_key,request_hash,status,created_at,updated_at)
             VALUES (?1,'submit_workload','autoscaler',?2,?1,'hash','succeeded',20,20)",
        )
        .bind(&id)
        .bind(pool_id)
        .execute(&self.pool)
        .await?;
        sqlx::query(
            "INSERT INTO workload_intents (id,workload_id,generation,kind,pool_id,profile_id,target_id,agent_run_id,expected_profile_revision,status,created_at,updated_at)
             VALUES (?1,?1,1,'ci_runner',?2,?3,?4,NULL,?5,'admitted',20,20)",
        )
        .bind(&id)
        .bind(pool_id)
        .bind(&profile.id)
        .bind(target)
        .bind(profile.revision)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// The targets linked (enabled) to the pool's live platforms, by kind.
    async fn linked_kinds(&self, pool_id: &str) -> Result<Vec<(String, String)>> {
        Ok(sqlx::query_as(
            "SELECT t.id,t.kind FROM profile_ci_targets pct
               JOIN pool_execution_profiles p ON p.id=pct.profile_id
               JOIN fleet_ci_targets t ON t.id=pct.target_id
              WHERE p.pool_id=? AND p.archived_at IS NULL AND pct.enabled=1 AND pct.revoked_at IS NULL
              ORDER BY t.kind,t.id",
        )
        .bind(pool_id)
        .fetch_all(&self.pool)
        .await?)
    }
}

/// One Linux Docker platform, with `image` (empty: the pool's own image).
fn docker_platform(image: &str) -> Result<FleetSettingsInput> {
    Ok(serde_json::from_value(json!({
        "enabled": true,
        "mode": "ephemeral",
        "cpuMillis": 1000,
        "memoryMib": 1024,
        "diskBytes": 0,
        "platforms": [{
            "runtimeKind": "docker",
            "executionOs": "linux",
            "architecture": "x64",
            "image": image,
            "requiresDockerSocket": true,
        }],
    }))?)
}

/// D6 at `save_pool_fleet` (the `sync_github_targets` call site at
/// `targets.rs:540`): saving a GitLab pool's fleet settings gives each of its
/// connections a fleet target of the connection's scope kind and id, links
/// them to the pool's platform, and `pool_targets` returns them. Saving again
/// creates no duplicate, and a pool without GitLab connections gets no
/// GitLab target.
#[tokio::test]
async fn saving_a_gitlab_pool_links_one_target_per_connection_to_its_platforms() -> Result<()> {
    let database = Database::new().await?;
    let explicit = docker_platform("registry.example.test/gitlab-runner:v17")?;

    let saved = targets::save_pool_fleet(&database.pool, "gl-pool", &explicit).await?;

    let (project, project_kind, project_id) = database
        .gitlab_target("conn-project")
        .await?
        .context("the project connection's target exists")?;
    let (group, group_kind, group_id) = database
        .gitlab_target("conn-group")
        .await?
        .context("the group connection's target exists")?;
    assert_eq!((project_kind.as_str(), project_id), ("gitlab_project", 42));
    assert_eq!((group_kind.as_str(), group_id), ("gitlab_group", 7));
    let mut expected = vec![
        (group.clone(), "gitlab_group".to_owned()),
        (project.clone(), "gitlab_project".to_owned()),
    ];
    expected.sort_by(|a, b| a.1.cmp(&b.1).then(a.0.cmp(&b.0)));
    let gitlab_links = |links: Vec<(String, String)>| {
        links
            .into_iter()
            .filter(|(_, kind)| kind.starts_with("gitlab_"))
            .collect::<Vec<_>>()
    };
    assert_eq!(
        gitlab_links(database.linked_kinds("gl-pool").await?),
        expected,
        "both connection targets are linked to the GitLab pool's platform"
    );
    assert!(
        saved.target_ids.contains(&project) && saved.target_ids.contains(&group),
        "the saved settings name both targets: {:?}",
        saved.target_ids
    );
    let pool_targets = targets::pool_targets(&database.pool, "gl-pool").await?;
    assert!(
        pool_targets.contains(&project) && pool_targets.contains(&group),
        "pool_targets returns the GitLab targets: {pool_targets:?}"
    );

    // Saving again (as the settings page does on every change) duplicates
    // nothing.
    targets::save_pool_fleet(&database.pool, "gl-pool", &explicit).await?;
    let gitlab_targets: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM fleet_ci_targets WHERE kind IN ('gitlab_project','gitlab_group')",
    )
    .fetch_one(&database.pool)
    .await?;
    assert_eq!(gitlab_targets, 2, "one target per connection, once");
    assert_eq!(
        gitlab_links(database.linked_kinds("gl-pool").await?),
        expected
    );

    // A GitHub pool's platform gets no GitLab target.
    targets::save_pool_fleet(&database.pool, "gh-pool", &docker_platform("")?).await?;
    assert_eq!(
        gitlab_links(database.linked_kinds("gh-pool").await?),
        [],
        "no GitLab target on a pool without GitLab connections"
    );
    database.finish().await
}

/// D6 at `list_targets` (the `sync_github_targets` call site at
/// `targets.rs:165`, the list an administrator grants machines from): a GitLab
/// connection's target is listed before any fleet setting was saved.
#[tokio::test]
async fn the_target_list_for_granting_machines_includes_gitlab_connections() -> Result<()> {
    let database = Database::new().await?;

    let listed = targets::list_targets(&database.pool).await?;

    for connection in ["conn-project", "conn-group"] {
        let (id, kind, _) = database
            .gitlab_target(connection)
            .await?
            .with_context(|| format!("{connection} has a fleet target after list_targets"))?;
        assert!(
            listed
                .iter()
                .any(|target| target.id == id && target.kind == kind),
            "{connection}'s target is listed: {listed:?}"
        );
    }
    database.finish().await
}

/// #183 decisions 1 and 6: once a GitLab connection's target is retired
/// (`retired_at` set and its connection dropped, in one UPDATE) the key path
/// never revives it. Here the connection row is still there under the same
/// id (as a restore could bring it back), so its canonical key resolves to
/// the retired row:
/// `gitlab_connection_target` answers `None` and inserts nothing, the target
/// list for granting machines and `pool_targets` leave the retired target
/// out, and saving the pool's fleet settings does not re-link it. The
/// connection-group target beside it is unaffected.
#[tokio::test]
async fn a_retired_gitlab_target_is_never_revived_offered_or_relinked() -> Result<()> {
    let database = Database::new().await?;
    let settings = docker_platform("registry.example.test/gitlab-runner:v17")?;
    targets::save_pool_fleet(&database.pool, "gl-pool", &settings).await?;
    let (retired, _, _) = database
        .gitlab_target("conn-project")
        .await?
        .context("the project connection's target exists")?;
    let (group, _, _) = database
        .gitlab_target("conn-group")
        .await?
        .context("the group connection's target exists")?;
    // The retire and the profile-link revoke that removing the connection
    // performs (decision 1, brief item 2).
    sqlx::query("UPDATE fleet_ci_targets SET retired_at=10,gitlab_connection_id=NULL WHERE id=?")
        .bind(&retired)
        .execute(&database.pool)
        .await
        .context("0048 lets a GitLab target retire")?;
    sqlx::query("UPDATE profile_ci_targets SET enabled=0,revoked_at=10 WHERE target_id=?")
        .bind(&retired)
        .execute(&database.pool)
        .await?;
    let gitlab_rows = || {
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM fleet_ci_targets WHERE kind IN ('gitlab_project','gitlab_group')",
        )
    };
    assert_eq!(gitlab_rows().fetch_one(&database.pool).await?, 2);

    assert_eq!(
        targets::gitlab_connection_target(&database.pool, "conn-project").await?,
        None,
        "the key path does not hand back a retired target"
    );
    let listed = targets::list_targets(&database.pool).await?;
    assert!(
        listed.iter().all(|target| target.id != retired),
        "a retired target is not offered for granting machines: {listed:?}"
    );
    assert!(
        listed.iter().any(|target| target.id == group),
        "the group connection's target is still listed: {listed:?}"
    );
    let pool_targets = targets::pool_targets(&database.pool, "gl-pool").await?;
    assert_eq!(pool_targets, std::slice::from_ref(&group), "pool_targets");
    targets::save_pool_fleet(&database.pool, "gl-pool", &settings).await?;
    assert_eq!(
        database.linked_kinds("gl-pool").await?,
        [(group.clone(), "gitlab_group".to_owned())],
        "saving the pool again links only the live target"
    );
    let link: (i64, Option<i64>) =
        sqlx::query_as("SELECT enabled,revoked_at FROM profile_ci_targets WHERE target_id=?")
            .bind(&retired)
            .fetch_one(&database.pool)
            .await?;
    assert_eq!(
        link,
        (0, Some(10)),
        "the retired target's link stays revoked"
    );
    assert_eq!(
        gitlab_rows().fetch_one(&database.pool).await?,
        2,
        "nothing re-created the retired target under a new id"
    );
    let state: (Option<String>, Option<i64>) =
        sqlx::query_as("SELECT gitlab_connection_id,retired_at FROM fleet_ci_targets WHERE id=?")
            .bind(&retired)
            .fetch_one(&database.pool)
            .await?;
    assert_eq!(state, (None, Some(10)), "the target stays retired");
    database.finish().await
}

/// Round-2 block 2 and brief-gate B-2 (a), (c): a Docker platform saved
/// without an image takes the pool's Docker image. A GitLab pool with no
/// Docker image of its own (here a Tart pool, whose `docker_image` is empty)
/// gets the pinned gitlab-runner image (D147-1), where a GitHub pool in the
/// same position still gets the stock runner image. A GitLab pool's own
/// Docker image is used as given.
#[tokio::test]
async fn a_gitlab_pool_without_a_docker_image_gets_the_gitlab_runner_image() -> Result<()> {
    let database = Database::new().await?;
    sqlx::raw_sql(
        r#"
        INSERT INTO runner_pools (id,installation_id,name,scope,provider,providers,labels,image,docker_image,tart_image,created_at,updated_at)
          VALUES ('gl-mac',1,'gitlab-mac','gitlab','tart','["tart"]','[]','base','','base',3,3),
                 ('gh-mac',1,'github-mac','organization','tart','["tart"]','[]','base','','base',4,4);
        INSERT INTO gitlab_connections (id,name,base_url,scope_kind,scope_id,scope_path,access_token_key,webhook_secret_key,created_at,updated_at)
          VALUES ('conn-mac','Acme macOS','https://gitlab.example.com','project',43,'acme/mac','gitlab.conn-mac.token','gitlab.conn-mac.webhook',3,3);
        INSERT INTO runner_pool_gitlab_connections (pool_id,connection_id,created_at) VALUES ('gl-mac','conn-mac',3);
        "#,
    )
    .execute(&database.pool)
    .await?;
    let profiles = |pool_id: &'static str| {
        sqlx::query_as::<_, (String,)>(
            "SELECT image FROM pool_execution_profiles WHERE pool_id=? AND archived_at IS NULL",
        )
        .bind(pool_id)
    };

    targets::save_pool_fleet(&database.pool, "gl-mac", &docker_platform("")?).await?;
    assert_eq!(
        profiles("gl-mac").fetch_all(&database.pool).await?,
        [(gridops_core::provisioning::GITLAB_RUNNER_IMAGE.to_owned(),)]
    );
    // A GitHub pool in the same position keeps today's fallback.
    targets::save_pool_fleet(&database.pool, "gh-mac", &docker_platform("")?).await?;
    assert_eq!(
        profiles("gh-mac").fetch_all(&database.pool).await?,
        [("ghcr.io/actions/actions-runner:latest".to_owned(),)]
    );
    // A GitLab pool's own Docker image is used as given.
    targets::save_pool_fleet(&database.pool, "gl-pool", &docker_platform("")?).await?;
    assert_eq!(
        profiles("gl-pool").fetch_all(&database.pool).await?,
        [("registry.example.test/gitlab-runner:v17".to_owned(),)]
    );
    database.finish().await
}

/// #181: whether `choose_platform` says the pool's chosen platform is behind
/// its queued jobs, with the default queue scale factor (1).
async fn behind(database: &Database, pool_id: &str, configured: &[String]) -> Result<bool> {
    let profiles = targets::pool_profiles(&database.pool, pool_id).await?;
    let (_, behind) = targets::choose_platform(&database.pool, pool_id, configured, 1, &profiles)
        .await?
        .context("the pool has a fleet platform")?;
    Ok(behind)
}

/// #181 (core): a fleet GitLab pool at `min_count` 0 with no runners and one
/// pending job it owns has its platform behind, so the reconciler places a
/// runner above the minimum; once a spare runner registered to no repository
/// (a GitLab runner) covers the job, it is not behind. The job's tags match
/// the pool's labels only: a tag that reads as an architecture (`arm64`, on
/// an x64 platform) does not narrow which platform counts a GitLab job.
#[tokio::test]
async fn a_pending_gitlab_job_puts_a_fleet_platform_behind_until_a_spare_runner_covers_it()
-> Result<()> {
    let database = Database::new().await?;
    sqlx::query(
        "UPDATE runner_pools SET min_count=0,max_count=2,autoscaling_enabled=1,labels='[\"docker\",\"arm64\"]'
          WHERE id='gl-pool'",
    )
    .execute(&database.pool)
    .await?;
    targets::save_pool_fleet(
        &database.pool,
        "gl-pool",
        &docker_platform("registry.example.test/gitlab-runner:v17")?,
    )
    .await?;
    database
        .pending_gitlab_job("conn-project", 501, &["docker", "arm64"])
        .await?;
    let configured = ["docker".to_owned(), "arm64".to_owned()];
    assert_eq!(
        gridops_core::assigned_queued_jobs(&database.pool, "gl-pool").await?,
        1,
        "the pool owns the pending job (the scaler's demand)"
    );

    assert!(
        behind(&database, "gl-pool", &configured).await?,
        "one pending GitLab job and no runner: the platform is behind"
    );

    let profiles = targets::pool_profiles(&database.pool, "gl-pool").await?;
    database
        .spare_runner("gl-pool", profiles.first().context("a platform")?, None)
        .await?;
    assert!(
        !behind(&database, "gl-pool", &configured).await?,
        "a starting runner with no repository covers the one job"
    );
    database.finish().await
}

/// #181 (mixed pool): an organization pool linked to a GitLab connection
/// counts both its queued GitHub job and its pending GitLab job, so one spare
/// runner leaves its platform behind; `platform_jobs` (which picks a fleet
/// runner's GitHub repository) still returns only the GitHub job.
#[tokio::test]
async fn a_pool_with_github_and_gitlab_jobs_counts_both_and_lists_only_github_jobs() -> Result<()> {
    let database = Database::new().await?;
    sqlx::raw_sql(
        r#"
        UPDATE runner_pools SET min_count=0,max_count=4,autoscaling_enabled=1,labels='["docker"]' WHERE id='gh-pool';
        INSERT INTO runner_pool_gitlab_connections (pool_id,connection_id,created_at) VALUES ('gh-pool','conn-project',3);
        INSERT INTO repositories (id,installation_id,owner,name,full_name,private,default_branch,html_url,last_synced_at,created_at,updated_at)
          VALUES (10,1,'octo-org','gridops','octo-org/gridops',0,'master','https://github.com/octo-org/gridops',1,1,1);
        INSERT INTO workflow_runs (id,repository_id,workflow_name,run_number,event,status,head_sha,html_url,github_created_at,github_updated_at,created_at,updated_at)
          VALUES (20,10,'CI',1,'push','queued','abc123','https://github.com/octo-org/gridops/actions/runs/20',1,1,1,1);
        "#,
    )
    .execute(&database.pool)
    .await?;
    // Queued just now, so not yet past the pool's priority window: a
    // prioritised job would make the platform behind on its own.
    sqlx::query(
        r#"INSERT INTO workflow_jobs (id,run_id,name,status,labels,html_url,created_at,updated_at)
             VALUES (200,20,'build','queued','["self-hosted","linux"]','https://github.com/octo-org/gridops/actions/runs/20/job/200',?1,?1)"#,
    )
    .bind(gridops_core::now_millis())
    .execute(&database.pool)
    .await?;
    targets::save_pool_fleet(&database.pool, "gh-pool", &docker_platform("")?).await?;
    database
        .pending_gitlab_job("conn-project", 502, &["docker"])
        .await?;
    let configured = ["docker".to_owned()];
    let profiles = targets::pool_profiles(&database.pool, "gh-pool").await?;
    let profile = profiles.first().context("a platform")?;
    database.spare_runner("gh-pool", profile, None).await?;
    assert_eq!(
        gridops_core::assigned_queued_jobs(&database.pool, "gh-pool").await?,
        2,
        "the pool owns both jobs (the scaler's demand)"
    );

    let jobs = targets::platform_jobs(
        &database.pool,
        "gh-pool",
        &configured,
        &profiles,
        &profile.id,
    )
    .await?;
    assert_eq!(
        jobs.iter()
            .map(|job| (job.repository_id, job.labels.clone()))
            .collect::<Vec<_>>(),
        [(10, vec!["self-hosted".to_owned(), "linux".to_owned()])],
        "platform_jobs lists the GitHub job only"
    );
    assert!(
        behind(&database, "gh-pool", &configured).await?,
        "two queued jobs (one GitHub, one GitLab) and one spare runner: behind"
    );
    database.finish().await
}

/// #181 (seam rule): only a spare runner registered to no repository covers a
/// pending GitLab job, never one registered to a GitHub repository.
///
/// An organization pool linked to a GitLab connection has two Linux platforms
/// (x64 first, arm64 second) that can both run its two jobs: a pending GitLab
/// job, queued first, and a queued GitHub job of repository 10. Its only
/// runner is a starting spare on the arm64 platform, registered to repository
/// 10. The GitLab job, attributed first, must not take that runner, so it
/// counts for the first platform as a job with no runner; the repository-10
/// runner stays with repository 10's job, so `platform_jobs` lists that job on
/// the arm64 platform and nothing on x64. If the repository runner covered the
/// GitLab job, the GitHub job would lose it and move to x64.
///
/// The observable is the attribution, not `choose_platform`'s `behind` flag:
/// on one platform every queued job adds to the deficit whichever runner
/// covers it, and a repository runner with no job of its repository waiting
/// is not counted against the deficit, so the flag cannot show this rule.
#[tokio::test]
async fn a_spare_runner_registered_to_a_github_repository_does_not_cover_a_gitlab_job() -> Result<()>
{
    let database = Database::new().await?;
    sqlx::raw_sql(
        r#"
        UPDATE runner_pools SET min_count=0,max_count=4,autoscaling_enabled=1,labels='["docker"]' WHERE id='gh-pool';
        INSERT INTO runner_pool_gitlab_connections (pool_id,connection_id,created_at) VALUES ('gh-pool','conn-project',3);
        INSERT INTO repositories (id,installation_id,owner,name,full_name,private,default_branch,html_url,last_synced_at,created_at,updated_at)
          VALUES (10,1,'octo-org','gridops','octo-org/gridops',0,'master','https://github.com/octo-org/gridops',1,1,1);
        INSERT INTO workflow_runs (id,repository_id,workflow_name,run_number,event,status,head_sha,html_url,github_created_at,github_updated_at,created_at,updated_at)
          VALUES (20,10,'CI',1,'push','queued','abc123','https://github.com/octo-org/gridops/actions/runs/20',1,1,1,1);
        "#,
    )
    .execute(&database.pool)
    .await?;
    // The GitLab job (queued at 10) is older, so it is attributed first.
    database
        .pending_gitlab_job("conn-project", 503, &["docker"])
        .await?;
    sqlx::query(
        r#"INSERT INTO workflow_jobs (id,run_id,name,status,labels,html_url,created_at,updated_at)
             VALUES (200,20,'build','queued','["self-hosted","linux"]','https://github.com/octo-org/gridops/actions/runs/20/job/200',?1,?1)"#,
    )
    .bind(gridops_core::now_millis())
    .execute(&database.pool)
    .await?;
    let platform = |architecture: &str| {
        json!({
            "runtimeKind": "docker",
            "executionOs": "linux",
            "architecture": architecture,
            "image": "",
            "requiresDockerSocket": true,
        })
    };
    let two_platforms: FleetSettingsInput = serde_json::from_value(json!({
        "enabled": true,
        "mode": "ephemeral",
        "cpuMillis": 1000,
        "memoryMib": 1024,
        "diskBytes": 0,
        "platforms": [platform("x64"), platform("arm64")],
    }))?;
    targets::save_pool_fleet(&database.pool, "gh-pool", &two_platforms).await?;
    let configured = ["docker".to_owned()];
    let profiles = targets::pool_profiles(&database.pool, "gh-pool").await?;
    let architecture = |profile: &targets::Profile| -> Result<String> {
        Ok(serde_json::to_value(profile.input.architecture)?
            .as_str()
            .context("an architecture name")?
            .to_owned())
    };
    let [x64, arm64] = profiles.as_slice() else {
        anyhow::bail!("two platforms: {}", profiles.len());
    };
    assert_eq!(
        (architecture(x64)?.as_str(), architecture(arm64)?.as_str()),
        ("x64", "arm64"),
        "x64 is the first platform"
    );
    database.spare_runner("gh-pool", arm64, Some(10)).await?;
    assert_eq!(
        gridops_core::assigned_queued_jobs(&database.pool, "gh-pool").await?,
        2,
        "the pool owns both jobs"
    );

    let listed = |profile_id: String| {
        let database = &database;
        let configured = &configured;
        let profiles = &profiles;
        async move {
            Ok::<_, anyhow::Error>(
                targets::platform_jobs(
                    &database.pool,
                    "gh-pool",
                    configured,
                    profiles,
                    &profile_id,
                )
                .await?
                .iter()
                .map(|job| job.repository_id)
                .collect::<Vec<_>>(),
            )
        }
    };
    assert_eq!(
        listed(arm64.id.clone()).await?,
        [10],
        "the repository-10 runner stays with repository 10's job, not the GitLab job"
    );
    assert_eq!(
        listed(x64.id.clone()).await?,
        Vec::<i64>::new(),
        "no GitHub job is pushed to x64"
    );
    database.finish().await
}

/// #181 (seam rule, inclusion half): a spare runner registered to no
/// repository does cover a pending GitLab job, so the job stays on that
/// runner's platform and no second runner is asked for.
///
/// A fleet GitLab pool has two Linux platforms, x64 first and arm64 second,
/// and one pending job either can run. With no runner the job counts for x64
/// (no room or machine anywhere: the first platform) and the pool is behind.
/// Once a starting spare with no repository sits on arm64, it covers the job
/// there, so no platform is behind. If that spare could not cover a GitLab
/// job, the job would still count for x64, where nothing covers it, and the
/// pool would stay behind. On one platform the two cases look the same,
/// because the job lands on the spare's platform either way.
#[tokio::test]
async fn a_spare_runner_with_no_repository_covers_a_gitlab_job_on_its_own_platform() -> Result<()> {
    let database = Database::new().await?;
    sqlx::query(
        "UPDATE runner_pools SET min_count=0,max_count=4,autoscaling_enabled=1 WHERE id='gl-pool'",
    )
    .execute(&database.pool)
    .await?;
    let platform = |architecture: &str| {
        json!({
            "runtimeKind": "docker",
            "executionOs": "linux",
            "architecture": architecture,
            "image": "",
            "requiresDockerSocket": true,
        })
    };
    let two_platforms: FleetSettingsInput = serde_json::from_value(json!({
        "enabled": true,
        "mode": "ephemeral",
        "cpuMillis": 1000,
        "memoryMib": 1024,
        "diskBytes": 0,
        "platforms": [platform("x64"), platform("arm64")],
    }))?;
    targets::save_pool_fleet(&database.pool, "gl-pool", &two_platforms).await?;
    database
        .pending_gitlab_job("conn-project", 504, &["docker"])
        .await?;
    let configured = ["docker".to_owned()];
    let profiles = targets::pool_profiles(&database.pool, "gl-pool").await?;
    let [x64, arm64] = profiles.as_slice() else {
        anyhow::bail!("two platforms: {}", profiles.len());
    };
    assert_eq!(
        (
            serde_json::to_value(x64.input.architecture)?,
            serde_json::to_value(arm64.input.architecture)?
        ),
        (json!("x64"), json!("arm64")),
        "x64 is the first platform"
    );
    assert!(
        behind(&database, "gl-pool", &configured).await?,
        "one pending GitLab job and no runner: behind"
    );

    database.spare_runner("gl-pool", arm64, None).await?;
    assert!(
        !behind(&database, "gl-pool", &configured).await?,
        "the arm64 spare with no repository covers the GitLab job: not behind"
    );
    database.finish().await
}
