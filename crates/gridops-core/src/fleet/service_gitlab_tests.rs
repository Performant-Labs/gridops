//! #138: fleet authorization for GitLab pools and targets.
//!
//! GitLab has no user grants yet (D11), so GitLab access is system-admin
//! only: a GitLab target is allowed to service principals and system
//! administrators and refused to everyone else, and a pool with GitLab
//! targets needs a system administrator. Before #138, `ensure_target_access`
//! sent every non-GitHub target down the Bitbucket arm, which looks up grants
//! for the target's (NULL) `bitbucket_connection_id` and so refused system
//! administrators too.
//!
//! These call the two private checks directly from inside `service`: the
//! public paths (`submit_intent`, `load_operation`) run
//! `ensure_target_identity_for_pool` first, which refuses every GitLab target
//! until #139 adds its arm, so they cannot show these decisions yet.

use anyhow::{Result, anyhow};
use sqlx::SqlitePool;

use super::{FleetError, ensure_pool_access, ensure_target_access};
use crate::fleet::{CiTarget, ExternalId, GitlabConnectionId, authorization::Principal, targets};

const CONNECTION: &str = "a1000000-0000-4000-8000-000000000001";
const POOL: &str = "gitlab-pool";

struct Fixture {
    database: SqlitePool,
    path: std::path::PathBuf,
    target_id: String,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        for suffix in ["", "-shm", "-wal"] {
            let _ = std::fs::remove_file(format!("{}{suffix}", self.path.display()));
        }
    }
}

/// A GitLab pool on installation 42 with one GitLab project target, a
/// system administrator (`admin-1`) and a member (`member-1`) who
/// administers installation 42.
async fn fixture() -> Result<Fixture> {
    let path = std::env::temp_dir().join(format!(
        "gridops-gitlab-access-{}.sqlite",
        uuid::Uuid::new_v4()
    ));
    let database = crate::connect_database_path(&path).await?;
    sqlx::raw_sql(
        "INSERT INTO users (id,github_id,login,role,access_token,last_login_at,created_at,updated_at)
           VALUES ('admin-1',1,'admin','admin','',1,1,1),('member-1',2,'member','member','',1,1,1);
         INSERT INTO installations (id,account_id,account_login,account_type,target_type,repository_selection,created_at,updated_at)
           VALUES (42,4242,'acme','Organization','Organization','selected',1,1);
         INSERT INTO user_installations (user_id,installation_id,permission,created_at)
           VALUES ('admin-1',42,'admin',1),('member-1',42,'admin',1);
         INSERT INTO gitlab_connections (id,name,base_url,scope_kind,scope_id,scope_path,access_token_key,webhook_secret_key,created_at,updated_at)
           VALUES ('a1000000-0000-4000-8000-000000000001','acme','https://gitlab.example.com','project',42,'acme/app','t.token','t.webhook',1,1);
         INSERT INTO runner_pools (id,installation_id,name,scope,provider,providers,labels,image,docker_image,tart_image,created_at,updated_at)
           VALUES ('gitlab-pool',42,'gitlab-pool','gitlab','docker','[\"docker\"]','[]','runner:latest','runner:latest','',1,1);
         INSERT INTO runner_pool_gitlab_connections (pool_id,connection_id,created_at)
           VALUES ('gitlab-pool','a1000000-0000-4000-8000-000000000001',1);",
    )
    .execute(&database)
    .await?;
    let target_id = targets::ensure_target(
        &database,
        &CiTarget::GitlabProject {
            connection_id: GitlabConnectionId::new(CONNECTION)
                .map_err(|error| anyhow!("{error}"))?,
            project_id: ExternalId::new(42).map_err(|error| anyhow!("{error}"))?,
        },
    )
    .await
    .map_err(|error| anyhow!("{error}"))?;
    Ok(Fixture {
        database,
        path,
        target_id,
    })
}

fn user(id: &str) -> Principal {
    Principal::AuthenticatedUser {
        user_id: id.to_owned(),
    }
}

async fn target_decision(
    fixture: &Fixture,
    principal: &Principal,
) -> Result<Result<(), FleetError>> {
    let mut tx = fixture.database.begin().await?;
    let decision = ensure_target_access(&mut tx, principal, &fixture.target_id).await;
    tx.rollback().await?;
    Ok(decision)
}

async fn pool_decision(fixture: &Fixture, principal: &Principal) -> Result<Result<(), FleetError>> {
    let mut tx = fixture.database.begin().await?;
    let decision = ensure_pool_access(&mut tx, principal, POOL).await;
    tx.rollback().await?;
    Ok(decision)
}

#[tokio::test]
async fn a_gitlab_target_is_allowed_to_service_principals_and_system_admins_only() -> Result<()> {
    let fixture = fixture().await?;
    for principal in [
        Principal::Reconciler,
        Principal::Autoscaler,
        Principal::FixAgent,
    ] {
        assert!(
            target_decision(&fixture, &principal).await?.is_ok(),
            "{principal:?} may act on a GitLab target"
        );
    }
    assert!(
        target_decision(&fixture, &user("admin-1")).await?.is_ok(),
        "a system administrator may act on a GitLab target"
    );
    assert!(
        matches!(
            target_decision(&fixture, &user("member-1")).await?,
            Err(FleetError::Forbidden)
        ),
        "a member is refused, whatever their installation permission"
    );
    Ok(())
}

#[tokio::test]
async fn a_pool_with_gitlab_targets_needs_a_system_admin() -> Result<()> {
    let fixture = fixture().await?;
    assert!(
        pool_decision(&fixture, &Principal::Reconciler)
            .await?
            .is_ok()
    );
    assert!(
        pool_decision(&fixture, &user("admin-1")).await?.is_ok(),
        "a system administrator may use the pool"
    );
    assert!(
        matches!(
            pool_decision(&fixture, &user("member-1")).await?,
            Err(FleetError::Forbidden)
        ),
        "an installation admin who is not a system admin is refused"
    );
    Ok(())
}
