//! #138: runner pools with GitLab targets, at the model and fleet-target
//! level.
//!
//! - `CreateRunnerPool` / `UpdateRunnerPool` accept `gitlabConnectionIds` and
//!   `runUntagged`, with the same id rules as Bitbucket's and no mixing with
//!   GitHub or Bitbucket targets. The bodies are built as JSON, the way a
//!   client sends them, so the wire names are what is pinned.
//! - `pool_targets` gives a `scope = 'gitlab'` pool no GitHub target, so
//!   `save_pool_fleet` never links one into `profile_ci_targets`.

use anyhow::{Context as _, Result, anyhow};
use gridops_core::{
    fleet::targets::{FleetSettingsInput, save_pool_fleet},
    models::{CreateRunnerPool, UpdateRunnerPool},
};
use serde_json::{Value, json};

const CONNECTION_1: &str = "a1000000-0000-4000-8000-000000000001";
const CONNECTION_2: &str = "a1000000-0000-4000-8000-000000000002";
const BITBUCKET: &str = "b1000000-0000-4000-8000-000000000001";

/// A valid GitLab pool: Linux Docker runners (no Tart needed, unlike
/// Bitbucket), one GitLab target, untagged jobs on.
fn gitlab_pool() -> Value {
    json!({
        "installationId": 42,
        "name": "gitlab-pool",
        "scope": "gitlab",
        "gitlabConnectionIds": [CONNECTION_1],
        "runUntagged": true,
        "mode": "ephemeral",
        "provider": "docker",
        "providers": ["docker"],
        "labels": [],
        "image": "ghcr.io/actions/actions-runner:latest",
        "dockerImage": "ghcr.io/actions/actions-runner:latest",
        "desiredCount": 1,
        "minCount": 0,
        "maxCount": 4,
        "autoscalingEnabled": true,
        "queueScaleFactor": 1,
        "idleTimeoutMinutes": 5,
        "cpuLimit": 2.0,
        "memoryLimitMb": 2048,
        "runnerGroupId": 1,
    })
}

fn with(mut body: Value, changes: &Value) -> Value {
    for (key, value) in changes.as_object().into_iter().flatten() {
        if value.is_null() {
            if let Some(object) = body.as_object_mut() {
                object.remove(key);
            }
        } else {
            body[key] = value.clone();
        }
    }
    body
}

fn validate_create(body: &Value) -> Result<Result<(), String>> {
    let input: CreateRunnerPool = serde_json::from_value(body.clone())?;
    Ok(input.validate())
}

/// Every update body here is for a GitLab pool: the handler passes
/// `pool.scope == "gitlab"` (#172).
fn validate_update(body: &Value) -> Result<Result<(), String>> {
    let input: UpdateRunnerPool = serde_json::from_value(body.clone())?;
    Ok(input.validate(true))
}

fn many_ids(count: usize) -> Vec<String> {
    (0..count)
        .map(|index| format!("a1000000-0000-4000-8000-{index:012}"))
        .collect()
}

#[test]
fn a_gitlab_pool_on_linux_docker_is_valid_with_or_without_untagged_jobs() -> Result<()> {
    assert_eq!(validate_create(&gitlab_pool())?, Ok(()));
    let tagged_only = with(gitlab_pool(), &json!({"runUntagged": null}));
    assert_eq!(
        validate_create(&tagged_only)?,
        Ok(()),
        "runUntagged defaults"
    );
    let two = with(
        gitlab_pool(),
        &json!({"gitlabConnectionIds": [CONNECTION_1, CONNECTION_2]}),
    );
    assert_eq!(validate_create(&two)?, Ok(()));
    Ok(())
}

#[test]
fn a_pool_never_mixes_gitlab_targets_with_github_or_bitbucket_ones() -> Result<()> {
    // The control case: without it every refusal below would pass for the
    // wrong reason (an unknown scope).
    assert_eq!(validate_create(&gitlab_pool())?, Ok(()), "the control case");
    for (what, changes) in [
        (
            "a GitLab pool without GitLab targets",
            json!({"gitlabConnectionIds": []}),
        ),
        (
            "a GitLab pool without the field",
            json!({"gitlabConnectionIds": null}),
        ),
        (
            "a GitLab pool with repositories",
            json!({"repositoryIds": [7]}),
        ),
        (
            "a GitLab pool with a repository",
            json!({"repositoryId": 7}),
        ),
        (
            "a GitLab pool with a Bitbucket target",
            json!({
                "bitbucketConnectionIds": [BITBUCKET],
                "providers": ["docker", "tart"],
                "tartImage": "ghcr.io/cirruslabs/macos-sequoia-base:latest",
            }),
        ),
        (
            "an organization pool with a GitLab target",
            json!({"scope": "organization"}),
        ),
        (
            "a repository pool with a GitLab target",
            json!({"scope": "repository", "repositoryIds": [7]}),
        ),
    ] {
        let body = with(gitlab_pool(), &changes);
        assert!(validate_create(&body)?.is_err(), "{what} must be refused");
    }
    Ok(())
}

#[test]
fn gitlab_target_ids_follow_the_bitbucket_rules() -> Result<()> {
    assert_eq!(validate_create(&gitlab_pool())?, Ok(()), "the control case");
    let at_capacity = with(
        gitlab_pool(),
        &json!({"gitlabConnectionIds": many_ids(4), "maxCount": 4, "desiredCount": 1}),
    );
    assert_eq!(
        validate_create(&at_capacity)?,
        Ok(()),
        "as many targets as runners"
    );
    for (what, changes) in [
        (
            "an id that is not a UUID",
            json!({"gitlabConnectionIds": ["not-a-uuid"]}),
        ),
        (
            "a repeated id",
            json!({"gitlabConnectionIds": [CONNECTION_1, CONNECTION_1]}),
        ),
        (
            "more targets than runners",
            json!({"gitlabConnectionIds": many_ids(5), "maxCount": 4}),
        ),
        (
            "more than 100 targets",
            json!({"gitlabConnectionIds": many_ids(101), "maxCount": 100}),
        ),
    ] {
        let body = with(gitlab_pool(), &changes);
        assert!(
            validate_create(&body)?.is_err(),
            "create: {what} must be refused"
        );
    }
    Ok(())
}

/// A valid update body (an update carries no scope; the handler checks the
/// scope rules against the stored pool).
fn update() -> Value {
    with(
        gitlab_pool(),
        &json!({"installationId": null, "scope": null}),
    )
}

#[test]
fn an_update_checks_gitlab_target_ids_like_a_create() -> Result<()> {
    assert_eq!(validate_update(&update())?, Ok(()), "the control case");
    let unchanged = with(
        update(),
        &json!({"gitlabConnectionIds": null, "runUntagged": null}),
    );
    assert_eq!(
        validate_update(&unchanged)?,
        Ok(()),
        "both fields may be left out"
    );
    for (what, changes) in [
        (
            "an id that is not a UUID",
            json!({"gitlabConnectionIds": ["not-a-uuid"]}),
        ),
        (
            "a repeated id",
            json!({"gitlabConnectionIds": [CONNECTION_1, CONNECTION_1]}),
        ),
        (
            "more targets than runners",
            json!({"gitlabConnectionIds": many_ids(5), "maxCount": 4}),
        ),
    ] {
        let body = with(update(), &changes);
        assert!(
            validate_update(&body)?.is_err(),
            "update: {what} must be refused"
        );
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// pool_targets / save_pool_fleet
// ---------------------------------------------------------------------------

#[tokio::test]
async fn saving_a_gitlab_pools_fleet_settings_links_no_github_target() -> Result<()> {
    let path = std::env::temp_dir().join(format!(
        "gridops-gitlab-pools-{}.sqlite",
        uuid::Uuid::new_v4()
    ));
    let database = gridops_core::connect_database_path(&path).await?;
    sqlx::raw_sql(
        "INSERT INTO installations (id,account_id,account_login,account_type,target_type,repository_selection,created_at,updated_at)
           VALUES (42,4242,'acme','Organization','Organization','selected',1,1);
         INSERT INTO gitlab_connections (id,name,base_url,scope_kind,scope_id,scope_path,access_token_key,webhook_secret_key,created_at,updated_at)
           VALUES ('a1000000-0000-4000-8000-000000000001','acme','https://gitlab.example.com','project',42,'acme/app','t.token','t.webhook',1,1);
         INSERT INTO runner_pools (id,installation_id,name,scope,provider,providers,labels,image,docker_image,tart_image,created_at,updated_at)
           VALUES ('org-pool',42,'org-pool','organization','docker','[\"docker\"]','[]','runner:latest','runner:latest','',1,1),
                  ('gitlab-pool',42,'gitlab-pool','gitlab','docker','[\"docker\"]','[]','runner:latest','runner:latest','',1,1);
         INSERT INTO runner_pool_gitlab_connections (pool_id,connection_id,created_at)
           VALUES ('gitlab-pool','a1000000-0000-4000-8000-000000000001',1);",
    )
    .execute(&database)
    .await?;
    let settings: FleetSettingsInput = serde_json::from_value(json!({
        "enabled": true,
        "mode": "ephemeral",
        "cpuMillis": 2000,
        "memoryMib": 2048,
        "diskBytes": 10_737_418_240_u64,
        "platforms": [{
            "runtimeKind": "docker",
            "executionOs": "linux",
            "architecture": "x64",
            "image": "ghcr.io/actions/actions-runner:latest",
        }],
    }))?;
    let linked_kinds = |pool_id: &'static str| {
        let database = database.clone();
        async move {
            sqlx::query_scalar::<_, String>(
                "SELECT t.kind FROM profile_ci_targets link
                   JOIN pool_execution_profiles profile ON profile.id=link.profile_id
                   JOIN fleet_ci_targets t ON t.id=link.target_id
                  WHERE profile.pool_id=? AND link.enabled=1",
            )
            .bind(pool_id)
            .fetch_all(&database)
            .await
        }
    };

    // The control case: an organization pool on the same installation gets
    // its GitHub organization target, so the save does link targets.
    save_pool_fleet(&database, "org-pool", &settings)
        .await
        .map_err(|error| anyhow!("org pool: {error}"))?;
    assert_eq!(linked_kinds("org-pool").await?, ["github_organization"]);

    save_pool_fleet(&database, "gitlab-pool", &settings)
        .await
        .map_err(|error| anyhow!("gitlab pool: {error}"))?;
    let kinds = linked_kinds("gitlab-pool").await?;
    assert!(
        kinds.iter().all(|kind| !kind.starts_with("github_")),
        "a GitLab pool is never admitted to a GitHub target: {kinds:?}"
    );
    let profiles: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM pool_execution_profiles WHERE pool_id='gitlab-pool'",
    )
    .fetch_one(&database)
    .await
    .context("profiles")?;
    assert_eq!(profiles, 1, "the save itself succeeded");
    database.close().await;
    for suffix in ["", "-shm", "-wal"] {
        let _ = std::fs::remove_file(format!("{}{suffix}", path.display()));
    }
    Ok(())
}
