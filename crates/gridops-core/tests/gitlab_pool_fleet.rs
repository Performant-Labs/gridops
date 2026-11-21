//! A GitLab pool's fleet targets and runner image (#139: D6 and the image
//! rule of round-2 block 2), over a real migrated database.
//!
//! A GitLab pool is a `scope='gitlab'` pool with `runner_pool_gitlab_connections`
//! rows (#138's shape). Its fleet targets come from its connections: one
//! `gitlab_project` or `gitlab_group` target per connection, linked to the
//! pool's platforms. A Docker platform saved without an image gets the pool's
//! Docker image; a GitLab pool without one is refused, since the GitHub stock
//! image cannot run GitLab jobs (until #147's `GITLAB_RUNNER_IMAGE` exists;
//! it is not on master at the time of writing, b22c451 / origin c575130).

use anyhow::{Context as _, Result};
use gridops_core::{
    connect_database_path,
    fleet::{
        FleetError,
        targets::{self, FleetSettingsInput},
    },
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

/// Round-2 block 2 and brief-gate B-2 (a), (c): a Docker platform saved
/// without an image takes the pool's Docker image. A GitLab pool with no
/// Docker image of its own (here a Tart pool, whose `docker_image` is empty)
/// is refused with a readable error and nothing is saved, where a GitHub pool
/// in the same position still gets the stock runner image. A GitLab pool's
/// own Docker image is used as given.
#[tokio::test]
async fn a_gitlab_pool_without_a_docker_image_cannot_save_a_docker_platform() -> Result<()> {
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

    let refused = targets::save_pool_fleet(&database.pool, "gl-mac", &docker_platform("")?).await;

    assert!(
        matches!(
            refused,
            Err(FleetError::InvalidRequest(
                "GitLab pool has no runner image"
            ))
        ),
        "{refused:?}"
    );
    assert_eq!(
        profiles("gl-mac").fetch_all(&database.pool).await?,
        [],
        "nothing is saved"
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
