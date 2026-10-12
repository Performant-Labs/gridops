//! CI targets and pool execution profiles.
//!
//! Every GitHub repository and organization `GridOps` knows about gets a stable
//! fleet target, so hosts can be granted targets and profiles can name them.
//! A pool runs on fleet hosts when it has an enabled execution profile; the
//! profile covers the pool's repositories (or its organization).

use serde::{Deserialize, Serialize};
use sqlx::{Row as _, SqlitePool};
use uuid::Uuid;

use super::{
    FleetError,
    domain::{CiTarget, ExternalId},
};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TargetSummary {
    pub id: String,
    pub kind: String,
    pub label: String,
}

/// Creates targets for every known repository and organization installation.
pub async fn sync_github_targets(pool: &SqlitePool) -> Result<(), FleetError> {
    let repositories = sqlx::query("SELECT id,installation_id FROM repositories")
        .fetch_all(pool)
        .await?;
    for row in repositories {
        ensure_target(
            pool,
            &CiTarget::GithubRepository {
                installation_id: external(row.get("installation_id"))?,
                repository_id: external(row.get("id"))?,
            },
        )
        .await?;
    }
    let organizations =
        sqlx::query("SELECT id,account_id FROM installations WHERE account_type='Organization'")
            .fetch_all(pool)
            .await?;
    for row in organizations {
        ensure_target(
            pool,
            &CiTarget::GithubOrganization {
                installation_id: external(row.get("id"))?,
                organization_id: external(row.get("account_id"))?,
            },
        )
        .await?;
    }
    Ok(())
}

/// The fleet target ID for `target`, created on first use.
pub async fn ensure_target(pool: &SqlitePool, target: &CiTarget) -> Result<String, FleetError> {
    let key = target.canonical_key();
    let (kind, installation, repository, organization) = match target {
        CiTarget::GithubRepository {
            installation_id,
            repository_id,
        } => (
            "github_repository",
            installation_id.get(),
            Some(repository_id.get()),
            None,
        ),
        CiTarget::GithubOrganization {
            installation_id,
            organization_id,
        } => (
            "github_organization",
            installation_id.get(),
            None,
            Some(organization_id.get()),
        ),
        _ => return Err(FleetError::InvalidRequest("only GitHub targets are synced")),
    };
    sqlx::query(
        "INSERT INTO fleet_ci_targets (id,kind,canonical_key,installation_id,repository_id,organization_id)
         VALUES (?,?,?,?,?,?) ON CONFLICT(canonical_key) DO NOTHING",
    )
    .bind(Uuid::new_v4().to_string())
    .bind(kind)
    .bind(&key)
    .bind(i64::try_from(installation).map_err(|_| FleetError::InvalidRequest("installation"))?)
    .bind(repository.map(i64::try_from).transpose().map_err(|_| FleetError::InvalidRequest("repository"))?)
    .bind(organization.map(i64::try_from).transpose().map_err(|_| FleetError::InvalidRequest("organization"))?)
    .execute(pool)
    .await?;
    Ok(
        sqlx::query_scalar("SELECT id FROM fleet_ci_targets WHERE canonical_key=?")
            .bind(&key)
            .fetch_one(pool)
            .await?,
    )
}

/// Targets with a readable label, for granting hosts.
pub async fn list_targets(pool: &SqlitePool) -> Result<Vec<TargetSummary>, FleetError> {
    sync_github_targets(pool).await?;
    let rows = sqlx::query(
        "SELECT t.id,t.kind,COALESCE(r.full_name,i.account_login,t.canonical_key) AS label
           FROM fleet_ci_targets t
           LEFT JOIN repositories r ON r.id=t.repository_id
           LEFT JOIN installations i ON i.id=t.installation_id AND t.kind='github_organization'
          ORDER BY label",
    )
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|row| TargetSummary {
            id: row.get("id"),
            kind: row.get("kind"),
            label: row.get("label"),
        })
        .collect())
}

/// The editable part of a pool's execution profile.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProfileInput {
    pub enabled: bool,
    pub runtime_kind: super::RuntimeKind,
    pub execution_os: super::ExecutionOs,
    pub architecture: super::Architecture,
    pub mode: super::protocol::agent::RunnerMode,
    pub image: String,
    pub labels: Vec<String>,
    pub cpu_millis: u32,
    pub memory_mib: u32,
    pub disk_bytes: u64,
    #[serde(default)]
    pub requires_docker_socket: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Profile {
    pub id: String,
    pub revision: i64,
    #[serde(flatten)]
    pub input: ProfileInput,
    pub target_ids: Vec<String>,
}

/// The pool's profile, if it has one.
pub async fn pool_profile(pool: &SqlitePool, pool_id: &str) -> Result<Option<Profile>, FleetError> {
    let Some(row) = sqlx::query(
        "SELECT id,revision,enabled,runtime_kind,execution_os,architecture,mode,image,labels_json,
                cpu_millis,memory_mib,disk_bytes,requires_docker_socket
           FROM pool_execution_profiles WHERE pool_id=? ORDER BY created_at LIMIT 1",
    )
    .bind(pool_id)
    .fetch_optional(pool)
    .await?
    else {
        return Ok(None);
    };
    let id: String = row.get("id");
    let target_ids = sqlx::query_scalar(
        "SELECT target_id FROM profile_ci_targets WHERE profile_id=? AND enabled=1 AND revoked_at IS NULL",
    )
    .bind(&id)
    .fetch_all(pool)
    .await?;
    let input = serde_json::from_value(serde_json::json!({
        "enabled": row.get::<bool, _>("enabled"),
        "runtimeKind": row.get::<String, _>("runtime_kind"),
        "executionOs": row.get::<String, _>("execution_os"),
        "architecture": row.get::<String, _>("architecture"),
        "mode": row.get::<String, _>("mode"),
        "image": row.get::<String, _>("image"),
        "labels": serde_json::from_str::<Vec<String>>(row.get("labels_json")).unwrap_or_default(),
        "cpuMillis": row.get::<i64, _>("cpu_millis"),
        "memoryMib": row.get::<i64, _>("memory_mib"),
        "diskBytes": row.get::<i64, _>("disk_bytes"),
        "requiresDockerSocket": row.get::<bool, _>("requires_docker_socket"),
    }))
    .map_err(|_| FleetError::Invariant("stored profile"))?;
    Ok(Some(Profile {
        id,
        revision: row.get("revision"),
        input,
        target_ids,
    }))
}

/// Creates or updates the pool's profile and points it at the pool's targets.
/// Each change bumps the revision, so placements keep the configuration they
/// were admitted with.
pub async fn save_pool_profile(
    pool: &SqlitePool,
    pool_id: &str,
    input: &ProfileInput,
) -> Result<Profile, FleetError> {
    if input.cpu_millis == 0 || input.memory_mib == 0 || input.image.trim().is_empty() {
        return Err(FleetError::InvalidRequest(
            "profile needs CPU, memory and an image",
        ));
    }
    if input.labels.len() > 100 {
        return Err(FleetError::InvalidRequest("too many labels"));
    }
    super::protocol::primitives::ImageReference::try_from(input.image.clone())
        .map_err(|_| FleetError::InvalidRequest("image reference"))?;
    sync_github_targets(pool).await?;
    let targets = pool_targets(pool, pool_id).await?;
    let mut tx = pool.begin_with("BEGIN IMMEDIATE").await?;
    let now = crate::now_millis();
    let existing: Option<String> = sqlx::query_scalar(
        "SELECT id FROM pool_execution_profiles WHERE pool_id=? ORDER BY created_at LIMIT 1",
    )
    .bind(pool_id)
    .fetch_optional(&mut *tx)
    .await?;
    let values = (
        name(input.runtime_kind),
        serde_value(&input.execution_os)?,
        serde_value(&input.architecture)?,
        serde_value(&input.mode)?,
        serde_json::to_string(&input.labels).map_err(|_| FleetError::Invariant("labels"))?,
    );
    let id = if let Some(id) = existing {
        {
            sqlx::query(
                "UPDATE pool_execution_profiles SET revision=revision+1,enabled=?,runtime_kind=?,
                   execution_os=?,architecture=?,mode=?,requires_docker_socket=?,all_authorized_hosts=1,
                   image=?,labels_json=?,cpu_millis=?,memory_mib=?,disk_bytes=?,updated_at=? WHERE id=?",
            )
            .bind(input.enabled)
            .bind(values.0)
            .bind(&values.1)
            .bind(&values.2)
            .bind(&values.3)
            .bind(input.requires_docker_socket)
            .bind(input.image.trim())
            .bind(&values.4)
            .bind(i64::from(input.cpu_millis))
            .bind(i64::from(input.memory_mib))
            .bind(i64::try_from(input.disk_bytes).map_err(|_| FleetError::InvalidRequest("disk"))?)
            .bind(now)
            .bind(&id)
            .execute(&mut *tx)
            .await?;
            id
        }
    } else {
        {
            let id = Uuid::new_v4().to_string();
            sqlx::query(
                "INSERT INTO pool_execution_profiles
                 (id,pool_id,name,revision,enabled,runtime_kind,execution_os,architecture,mode,
                  requires_docker_socket,all_authorized_hosts,image,labels_json,cpu_millis,memory_mib,
                  disk_bytes,created_at,updated_at)
                 VALUES (?,?,'default',1,?,?,?,?,?,?,1,?,?,?,?,?,?,?)",
            )
            .bind(&id)
            .bind(pool_id)
            .bind(input.enabled)
            .bind(values.0)
            .bind(&values.1)
            .bind(&values.2)
            .bind(&values.3)
            .bind(input.requires_docker_socket)
            .bind(input.image.trim())
            .bind(&values.4)
            .bind(i64::from(input.cpu_millis))
            .bind(i64::from(input.memory_mib))
            .bind(i64::try_from(input.disk_bytes).map_err(|_| FleetError::InvalidRequest("disk"))?)
            .bind(now)
            .bind(now)
            .execute(&mut *tx)
            .await?;
            id
        }
    };
    sqlx::query("UPDATE profile_ci_targets SET enabled=0 WHERE profile_id=?")
        .bind(&id)
        .execute(&mut *tx)
        .await?;
    for target in &targets {
        sqlx::query(
            "INSERT INTO profile_ci_targets (profile_id,target_id,enabled) VALUES (?,?,1)
             ON CONFLICT(profile_id,target_id) DO UPDATE SET enabled=1,revoked_at=NULL",
        )
        .bind(&id)
        .bind(target)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    pool_profile(pool, pool_id)
        .await?
        .ok_or(FleetError::Invariant("saved profile"))
}

/// The targets a pool serves: its repositories, or its organization.
pub async fn pool_targets(pool: &SqlitePool, pool_id: &str) -> Result<Vec<String>, FleetError> {
    let row = sqlx::query("SELECT scope,installation_id FROM runner_pools WHERE id=?")
        .bind(pool_id)
        .fetch_optional(pool)
        .await?
        .ok_or(FleetError::NotFound)?;
    if row.get::<String, _>("scope") == "repository" {
        let repositories = sqlx::query(
            "SELECT r.id,r.installation_id FROM runner_pool_repositories pr
               JOIN repositories r ON r.id=pr.repository_id WHERE pr.pool_id=?",
        )
        .bind(pool_id)
        .fetch_all(pool)
        .await?;
        let mut targets = Vec::new();
        for repository in repositories {
            targets.push(
                ensure_target(
                    pool,
                    &CiTarget::GithubRepository {
                        installation_id: external(repository.get("installation_id"))?,
                        repository_id: external(repository.get("id"))?,
                    },
                )
                .await?,
            );
        }
        return Ok(targets);
    }
    let installation_id: i64 = row.get("installation_id");
    let organization: Option<i64> =
        sqlx::query_scalar("SELECT account_id FROM installations WHERE id=?")
            .bind(installation_id)
            .fetch_optional(pool)
            .await?;
    let Some(organization) = organization else {
        return Ok(Vec::new());
    };
    Ok(vec![
        ensure_target(
            pool,
            &CiTarget::GithubOrganization {
                installation_id: external(installation_id)?,
                organization_id: external(organization)?,
            },
        )
        .await?,
    ])
}

fn external(value: i64) -> Result<ExternalId, FleetError> {
    u64::try_from(value)
        .ok()
        .and_then(|value| ExternalId::new(value).ok())
        .ok_or(FleetError::InvalidRequest("external ID"))
}

const fn name(kind: super::RuntimeKind) -> &'static str {
    match kind {
        super::RuntimeKind::Docker => "docker",
        super::RuntimeKind::TartVm => "tart_vm",
        super::RuntimeKind::NativeProcess => "native_process",
    }
}

fn serde_value<T: Serialize>(value: &T) -> Result<String, FleetError> {
    match serde_json::to_value(value) {
        Ok(serde_json::Value::String(value)) => Ok(value),
        _ => Err(FleetError::Invariant("enum value")),
    }
}
