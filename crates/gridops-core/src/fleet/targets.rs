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

/// Where a pool with a profile runs its runners.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Placement {
    /// Only on fleet machines.
    #[default]
    Machines,
    /// On a fleet machine when one can take the runner, else the built-in host.
    MachinesFirst,
    /// Wherever there's room: the built-in host when it can start the runner
    /// (fastest), else a ready fleet machine, else wait and try again.
    HostFirst,
}

impl Placement {
    const fn name(self) -> &'static str {
        match self {
            Self::Machines => "machines",
            Self::MachinesFirst => "machines_first",
            Self::HostFirst => "host_first",
        }
    }
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
    #[serde(default)]
    pub placement: Placement,
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

/// One platform a pool runs on: how and where each runner is isolated.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PlatformInput {
    /// The platform's profile, when it already exists.
    #[serde(default)]
    pub id: Option<String>,
    pub runtime_kind: super::RuntimeKind,
    pub execution_os: super::ExecutionOs,
    pub architecture: super::Architecture,
    /// Left empty, a Docker platform gets the pool's own Docker image.
    #[serde(default)]
    pub image: String,
    #[serde(default)]
    pub requires_docker_socket: bool,
}

/// A pool's fleet settings: what all its platforms share, and the platforms.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FleetSettingsInput {
    pub enabled: bool,
    #[serde(default)]
    pub placement: Placement,
    pub mode: super::protocol::agent::RunnerMode,
    pub cpu_millis: u32,
    pub memory_mib: u32,
    pub disk_bytes: u64,
    pub platforms: Vec<PlatformInput>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Platform {
    pub id: String,
    pub revision: i64,
    pub runtime_kind: super::RuntimeKind,
    pub execution_os: super::ExecutionOs,
    pub architecture: super::Architecture,
    pub image: String,
    pub requires_docker_socket: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FleetSettings {
    pub enabled: bool,
    pub placement: Placement,
    pub mode: super::protocol::agent::RunnerMode,
    pub cpu_millis: u32,
    pub memory_mib: u32,
    pub disk_bytes: u64,
    pub platforms: Vec<Platform>,
    pub target_ids: Vec<String>,
}

/// The pool's current (not archived) profiles, oldest first.
pub async fn pool_profiles(pool: &SqlitePool, pool_id: &str) -> Result<Vec<Profile>, FleetError> {
    let rows = sqlx::query(
        "SELECT id,revision,enabled,runtime_kind,execution_os,architecture,mode,image,labels_json,
                cpu_millis,memory_mib,disk_bytes,requires_docker_socket,placement
           FROM pool_execution_profiles WHERE pool_id=? AND archived_at IS NULL
          ORDER BY created_at,rowid",
    )
    .bind(pool_id)
    .fetch_all(pool)
    .await?;
    let mut profiles = Vec::with_capacity(rows.len());
    for row in rows {
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
            "placement": row.get::<String, _>("placement"),
        }))
        .map_err(|_| FleetError::Invariant("stored profile"))?;
        profiles.push(Profile {
            id,
            revision: row.get("revision"),
            input,
            target_ids,
        });
    }
    Ok(profiles)
}

/// The pool's fleet settings, if it has any platform.
pub async fn pool_fleet(
    pool: &SqlitePool,
    pool_id: &str,
) -> Result<Option<FleetSettings>, FleetError> {
    let profiles = pool_profiles(pool, pool_id).await?;
    let Some(first) = profiles.first() else {
        return Ok(None);
    };
    let shared = first.input.clone();
    let mut target_ids = first.target_ids.clone();
    target_ids.sort();
    Ok(Some(FleetSettings {
        enabled: profiles.iter().any(|profile| profile.input.enabled),
        placement: shared.placement,
        mode: shared.mode,
        cpu_millis: shared.cpu_millis,
        memory_mib: shared.memory_mib,
        disk_bytes: shared.disk_bytes,
        platforms: profiles
            .into_iter()
            .map(|profile| Platform {
                id: profile.id,
                revision: profile.revision,
                runtime_kind: profile.input.runtime_kind,
                execution_os: profile.input.execution_os,
                architecture: profile.input.architecture,
                image: profile.input.image,
                requires_docker_socket: profile.input.requires_docker_socket,
            })
            .collect(),
        target_ids,
    }))
}

/// The image a Docker platform runs when the pool has no Docker image of its own.
const STOCK_DOCKER_IMAGE: &str = "ghcr.io/actions/actions-runner:latest";

/// The Docker image the pool's built-in runners use, so a Docker platform
/// saved without an image gets the same tools.
async fn pool_docker_image(pool: &SqlitePool, pool_id: &str) -> Result<String, FleetError> {
    let row: Option<(String, String, String)> =
        sqlx::query_as("SELECT provider,image,docker_image FROM runner_pools WHERE id=?")
            .bind(pool_id)
            .fetch_optional(pool)
            .await?;
    let image = row
        .map(|(provider, image, docker_image)| {
            if docker_image.is_empty() && provider == "docker" {
                image
            } else {
                docker_image
            }
        })
        .unwrap_or_default();
    Ok(match image.trim() {
        "" => STOCK_DOCKER_IMAGE.to_owned(),
        image => image.to_owned(),
    })
}

/// Saves the pool's fleet settings: updates the platforms that are kept,
/// adds new ones and archives the ones left out. A platform's revision only
/// moves when it actually changes, so runners already waiting for it stay
/// valid.
pub async fn save_pool_fleet(
    pool: &SqlitePool,
    pool_id: &str,
    input: &FleetSettingsInput,
) -> Result<FleetSettings, FleetError> {
    use super::{ExecutionOs, RuntimeKind};
    if input.cpu_millis == 0 || input.memory_mib == 0 {
        return Err(FleetError::InvalidRequest("platforms need CPU and memory"));
    }
    if input.platforms.len() > 12 {
        return Err(FleetError::InvalidRequest("too many platforms"));
    }
    if input.enabled && input.platforms.is_empty() {
        return Err(FleetError::InvalidRequest(
            "add a platform to run on machines",
        ));
    }
    let unset = |platform: &PlatformInput| {
        platform.runtime_kind == RuntimeKind::Docker && platform.image.trim().is_empty()
    };
    let mut platforms = input.platforms.clone();
    if platforms.iter().any(unset) {
        let image = pool_docker_image(pool, pool_id).await?;
        for platform in platforms.iter_mut().filter(|platform| unset(platform)) {
            platform.image.clone_from(&image);
        }
    }
    let mut seen = Vec::new();
    for platform in &platforms {
        let fits = match platform.runtime_kind {
            RuntimeKind::Docker => platform.execution_os == ExecutionOs::Linux,
            RuntimeKind::TartVm => platform.execution_os == ExecutionOs::Macos,
            RuntimeKind::NativeProcess => true,
        };
        if !fits {
            return Err(FleetError::InvalidRequest("runtime doesn't run that OS"));
        }
        if platform.image.trim().is_empty() {
            return Err(FleetError::InvalidRequest("platform needs an image"));
        }
        super::protocol::primitives::ImageReference::try_from(platform.image.trim().to_owned())
            .map_err(|_| FleetError::InvalidRequest("image reference"))?;
        let key = (
            name(platform.runtime_kind),
            serde_value(&platform.execution_os)?,
            serde_value(&platform.architecture)?,
        );
        if seen.contains(&key) {
            return Err(FleetError::InvalidRequest("a platform is listed twice"));
        }
        seen.push(key);
    }
    sync_github_targets(pool).await?;
    let targets = pool_targets(pool, pool_id).await?;
    let mut tx = pool.begin_with("BEGIN IMMEDIATE").await?;
    let now = crate::now_millis();
    let existing: Vec<String> = sqlx::query_scalar(
        "SELECT id FROM pool_execution_profiles WHERE pool_id=? AND archived_at IS NULL",
    )
    .bind(pool_id)
    .fetch_all(&mut *tx)
    .await?;
    let mode = serde_value(&input.mode)?;
    let disk = i64::try_from(input.disk_bytes).map_err(|_| FleetError::InvalidRequest("disk"))?;
    let mut kept = Vec::new();
    for platform in &platforms {
        let os = serde_value(&platform.execution_os)?;
        let architecture = serde_value(&platform.architecture)?;
        let docker_socket =
            platform.requires_docker_socket && platform.runtime_kind == RuntimeKind::Docker;
        let id = if let Some(id) = platform.id.as_ref().filter(|id| existing.contains(id)) {
            sqlx::query(
                "UPDATE pool_execution_profiles SET
                   revision=revision+1,enabled=?,runtime_kind=?,execution_os=?,architecture=?,mode=?,
                   requires_docker_socket=?,all_authorized_hosts=1,image=?,labels_json='[]',
                   cpu_millis=?,memory_mib=?,disk_bytes=?,placement=?,updated_at=?
                 WHERE id=? AND NOT (enabled IS ? AND runtime_kind IS ? AND execution_os IS ?
                   AND architecture IS ? AND mode IS ? AND requires_docker_socket IS ? AND image IS ?
                   AND cpu_millis IS ? AND memory_mib IS ? AND disk_bytes IS ? AND placement IS ?)",
            )
            .bind(input.enabled)
            .bind(name(platform.runtime_kind))
            .bind(&os)
            .bind(&architecture)
            .bind(&mode)
            .bind(docker_socket)
            .bind(platform.image.trim())
            .bind(i64::from(input.cpu_millis))
            .bind(i64::from(input.memory_mib))
            .bind(disk)
            .bind(input.placement.name())
            .bind(now)
            .bind(id)
            .bind(input.enabled)
            .bind(name(platform.runtime_kind))
            .bind(&os)
            .bind(&architecture)
            .bind(&mode)
            .bind(docker_socket)
            .bind(platform.image.trim())
            .bind(i64::from(input.cpu_millis))
            .bind(i64::from(input.memory_mib))
            .bind(disk)
            .bind(input.placement.name())
            .execute(&mut *tx)
            .await?;
            id.clone()
        } else {
            let id = Uuid::new_v4().to_string();
            sqlx::query(
                "INSERT INTO pool_execution_profiles
                 (id,pool_id,name,revision,enabled,runtime_kind,execution_os,architecture,mode,
                  requires_docker_socket,all_authorized_hosts,image,labels_json,cpu_millis,memory_mib,
                  disk_bytes,placement,created_at,updated_at)
                 VALUES (?,?,?,1,?,?,?,?,?,?,1,?,'[]',?,?,?,?,?,?)",
            )
            .bind(&id)
            .bind(pool_id)
            .bind(format!("{os}-{architecture}-{}", name(platform.runtime_kind)))
            .bind(input.enabled)
            .bind(name(platform.runtime_kind))
            .bind(&os)
            .bind(&architecture)
            .bind(&mode)
            .bind(docker_socket)
            .bind(platform.image.trim())
            .bind(i64::from(input.cpu_millis))
            .bind(i64::from(input.memory_mib))
            .bind(disk)
            .bind(input.placement.name())
            .bind(now)
            .bind(now)
            .execute(&mut *tx)
            .await?;
            id
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
        kept.push(id);
    }
    for id in existing.iter().filter(|id| !kept.contains(id)) {
        sqlx::query(
            "UPDATE pool_execution_profiles SET enabled=0,archived_at=?,revision=revision+1,updated_at=?
             WHERE id=?",
        )
        .bind(now)
        .bind(now)
        .bind(id)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    pool_fleet(pool, pool_id)
        .await?
        .ok_or(FleetError::Invariant("saved fleet settings"))
}

/// Whether a runner on this platform satisfies every label a job asked for:
/// `self-hosted`, the platform's OS and architecture, and the pool's own
/// labels.
#[must_use]
pub fn platform_supports_labels(
    execution_os: super::ExecutionOs,
    architecture: super::Architecture,
    requested: &[String],
    configured: &[String],
) -> bool {
    let os = match execution_os {
        super::ExecutionOs::Linux => "linux",
        super::ExecutionOs::Macos => "macos",
        super::ExecutionOs::Windows => "windows",
    };
    let arch = match architecture {
        super::Architecture::X64 => "x64",
        super::Architecture::Arm64 => "arm64",
    };
    requested.iter().all(|label| {
        label.eq_ignore_ascii_case("self-hosted")
            || label.eq_ignore_ascii_case(os)
            || label.eq_ignore_ascii_case(arch)
            || (!crate::is_runner_system_label(label)
                && configured
                    .iter()
                    .any(|item| item.eq_ignore_ascii_case(label)))
    })
}

/// Whether one of the pool's fleet workloads may still be on a machine: a
/// placement that hasn't finished, or capacity still charged to a host.
const LIVE_WORKLOADS: &str = "SELECT EXISTS (
       SELECT 1 FROM workload_placements p JOIN workload_intents i ON i.id=p.intent_id
       WHERE i.pool_id=?1 AND (p.state NOT IN ('stopped','cleaned','failed')
         OR EXISTS (SELECT 1 FROM capacity_allocations a
                    WHERE a.placement_id=p.id AND a.state<>'released')))";

pub async fn pool_has_live_workloads(pool: &SqlitePool, pool_id: &str) -> Result<bool, FleetError> {
    Ok(sqlx::query_scalar::<_, bool>(LIVE_WORKLOADS)
        .bind(pool_id)
        .fetch_one(pool)
        .await?)
}

/// Deletes a pool along with its finished fleet history (platforms,
/// workloads, placements and their host commands), which would otherwise
/// keep the pool row alive. Returns false, deleting nothing, while a
/// workload may still be on a machine.
pub async fn delete_pool(pool: &SqlitePool, pool_id: &str) -> Result<bool, FleetError> {
    let mut tx = pool.begin_with("BEGIN IMMEDIATE").await?;
    if sqlx::query_scalar::<_, bool>(LIVE_WORKLOADS)
        .bind(pool_id)
        .fetch_one(&mut *tx)
        .await?
    {
        return Ok(false);
    }
    macro_rules! placements {
        () => {
            "SELECT p.id FROM workload_placements p
             JOIN workload_intents i ON i.id=p.intent_id WHERE i.pool_id=?1"
        };
    }
    macro_rules! profiles {
        () => {
            "SELECT id FROM pool_execution_profiles WHERE pool_id=?1"
        };
    }
    for statement in [
        concat!(
            "DELETE FROM host_command_attempts WHERE command_id IN (SELECT id FROM host_commands WHERE placement_id IN (",
            placements!(),
            "))"
        ),
        concat!(
            "DELETE FROM host_command_results WHERE command_id IN (SELECT id FROM host_commands WHERE placement_id IN (",
            placements!(),
            "))"
        ),
        concat!(
            "DELETE FROM host_commands WHERE placement_id IN (",
            placements!(),
            ")"
        ),
        concat!(
            "DELETE FROM fleet_events WHERE placement_id IN (",
            placements!(),
            ")"
        ),
        concat!(
            "DELETE FROM fleet_bootstrap_material WHERE placement_id IN (",
            placements!(),
            ")"
        ),
        concat!(
            "DELETE FROM capacity_allocations WHERE placement_id IN (",
            placements!(),
            ")"
        ),
        concat!(
            "DELETE FROM adoption_operations WHERE replacement_placement_id IN (",
            placements!(),
            ") OR profile_id IN (",
            profiles!(),
            ")"
        ),
        concat!(
            "DELETE FROM workload_placements WHERE id IN (",
            placements!(),
            ")"
        ),
        "DELETE FROM workload_intents WHERE pool_id=?1",
        concat!(
            "DELETE FROM profile_ci_targets WHERE profile_id IN (",
            profiles!(),
            ")"
        ),
        concat!(
            "DELETE FROM profile_host_selectors WHERE profile_id IN (",
            profiles!(),
            ")"
        ),
        "DELETE FROM pool_execution_profiles WHERE pool_id=?1",
        "DELETE FROM runner_pools WHERE id=?1",
    ] {
        sqlx::query(statement)
            .bind(pool_id)
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;
    Ok(true)
}

/// The targets a pool serves: its repositories, or its organization.
/// Links a repository that joined the pool after its fleet settings were
/// saved to the pool's platforms, as saving them would. Without the link every
/// runner for it is refused. Machines still have to be approved for it.
pub async fn link_pool_target(
    pool: &SqlitePool,
    pool_id: &str,
    target_id: &str,
) -> Result<(), FleetError> {
    sqlx::query(
        "INSERT INTO profile_ci_targets (profile_id,target_id,enabled)
         SELECT p.id,t.id,1 FROM pool_execution_profiles p
           JOIN runner_pools rp ON rp.id=p.pool_id
           JOIN fleet_ci_targets t ON t.id=?2 AND t.kind='github_repository'
          WHERE p.pool_id=?1 AND p.archived_at IS NULL
            AND (rp.repository_id=t.repository_id OR EXISTS (
              SELECT 1 FROM runner_pool_repositories pr
               WHERE pr.pool_id=rp.id AND pr.repository_id=t.repository_id))
         ON CONFLICT(profile_id,target_id) DO UPDATE SET enabled=1,revoked_at=NULL",
    )
    .bind(pool_id)
    .bind(target_id)
    .execute(pool)
    .await?;
    Ok(())
}

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

/// A runner waiting for a fleet machine, with the reason admission recorded.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QueuedRunner {
    pub runner_id: String,
    pub runner_name: Option<String>,
    pub reason: Option<String>,
    pub queued_at: i64,
}

/// The pool's runners still waiting for a machine, oldest first.
pub async fn pool_queue(pool: &SqlitePool, pool_id: &str) -> Result<Vec<QueuedRunner>, FleetError> {
    let rows = sqlx::query(
        "SELECT i.workload_id,r.name,i.reason_code,i.created_at FROM workload_intents i
           LEFT JOIN runners r ON r.id=i.workload_id
          WHERE i.pool_id=? AND i.status='queued' ORDER BY i.created_at LIMIT 50",
    )
    .bind(pool_id)
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|row| QueuedRunner {
            runner_id: row.get("workload_id"),
            runner_name: row.get("name"),
            reason: row.get("reason_code"),
            queued_at: row.get("created_at"),
        })
        .collect())
}

/// A machine that could run this pool's work but takes none while some of
/// its placements are unconfirmed.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BlockedHost {
    pub host_id: String,
    pub name: String,
    pub unconfirmed: i64,
    /// When its oldest unconfirmed placement became unconfirmed.
    pub since: i64,
}

/// The pool's machines held back by unconfirmed placements.
pub async fn pool_blocked_hosts(
    pool: &SqlitePool,
    pool_id: &str,
) -> Result<Vec<BlockedHost>, FleetError> {
    let rows = sqlx::query(
        "SELECT h.id,h.name,COUNT(*) AS unconfirmed,MIN(w.updated_at) AS since
           FROM fleet_hosts h JOIN workload_placements w ON w.host_id=h.id AND w.state='uncertain'
          WHERE h.lifecycle_state IN ('active','draining') AND EXISTS (
            SELECT 1 FROM pool_execution_profiles p
              JOIN host_backends b ON b.host_id=h.id AND b.runtime_kind=p.runtime_kind
                                  AND b.execution_os=p.execution_os AND b.architecture=p.architecture
                                  AND b.enabled=1
              JOIN profile_ci_targets pt ON pt.profile_id=p.id AND pt.enabled=1 AND pt.revoked_at IS NULL
              JOIN host_target_grants g ON g.host_id=h.id AND g.target_id=pt.target_id
                                       AND g.allow_schedule=1 AND g.revoked_at IS NULL
             WHERE p.pool_id=? AND p.enabled=1 AND p.archived_at IS NULL)
          GROUP BY h.id ORDER BY since",
    )
    .bind(pool_id)
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|row| BlockedHost {
            host_id: row.get("id"),
            name: row.get("name"),
            unconfirmed: row.get("unconfirmed"),
            since: row.get("since"),
        })
        .collect())
}

/// Each backend, with its host and resource domain, that an active, online
/// machine offers for a platform, with the right grants, no pressure and live
/// work authority. Binds the profile, the heartbeat cutoff and now.
macro_rules! authorized_backends {
    () => {
        "SELECT b.host_id,b.domain_id FROM pool_execution_profiles p
            JOIN host_backends b ON b.runtime_kind=p.runtime_kind AND b.execution_os=p.execution_os
                                AND b.architecture=p.architecture AND b.enabled=1 AND b.readiness='ready'
            JOIN fleet_hosts h ON h.id=b.host_id
            JOIN profile_ci_targets pt ON pt.profile_id=p.id AND pt.enabled=1 AND pt.revoked_at IS NULL
            JOIN host_target_grants g ON g.host_id=h.id AND g.target_id=pt.target_id
                                     AND g.allow_schedule=1 AND g.revoked_at IS NULL
           WHERE p.id=? AND h.enrollment_state='approved' AND h.lifecycle_state='active'
             AND h.integrity_state='verified' AND h.pressure_state='normal'
             AND h.last_heartbeat_at>? AND h.authority_expires_at>?
             AND (p.runtime_kind!='native_process' OR g.allow_native=1)"
    };
}

/// Some machine offers an [`authorized_backends`] backend for the platform.
macro_rules! authorized_machine {
    () => {
        concat!("EXISTS (", authorized_backends!(), ")")
    };
}

/// Room left on fleet machines, counted in runners per platform. Platforms on
/// one machine draw on the same resource domains, as admission does.
struct Room {
    /// Per (host, resource domain): CPU, memory and disk not yet allocated.
    free: std::collections::HashMap<(String, String), [i64; 3]>,
    parents: std::collections::HashMap<(String, String), Option<String>>,
    /// Per platform: the (host, domain) of each backend that could take it.
    backends: Vec<Vec<(String, String)>>,
    /// Per platform: what one runner reserves.
    request: Vec<[i64; 3]>,
}

impl Room {
    async fn load(pool: &SqlitePool, profiles: &[Profile]) -> Result<Self, FleetError> {
        let now = crate::now_millis();
        let mut backends = Vec::with_capacity(profiles.len());
        for profile in profiles {
            let rows: Vec<(String, String)> = sqlx::query_as(concat!(
                "SELECT DISTINCT * FROM (",
                authorized_backends!(),
                ")"
            ))
            .bind(&profile.id)
            .bind(now - 45_000)
            .bind(now)
            .fetch_all(pool)
            .await?;
            backends.push(rows);
        }
        let mut free = std::collections::HashMap::new();
        let mut parents = std::collections::HashMap::new();
        for row in sqlx::query(
            "SELECT d.host_id,d.id,d.parent_domain_id,d.cpu_millis,d.memory_mib,d.disk_bytes,
                    COALESCE(SUM(a.cpu_millis),0) AS used_cpu,COALESCE(SUM(a.memory_mib),0) AS used_memory,
                    COALESCE(SUM(a.disk_bytes),0) AS used_disk
               FROM host_resource_domains d
               LEFT JOIN capacity_allocations a ON a.host_id=d.host_id AND a.domain_id=d.id
                                              AND a.state!='released'
              GROUP BY d.host_id,d.id",
        )
        .fetch_all(pool)
        .await?
        {
            let key = (row.get::<String, _>("host_id"), row.get::<String, _>("id"));
            free.insert(
                key.clone(),
                [
                    row.get::<i64, _>("cpu_millis") - row.get::<i64, _>("used_cpu"),
                    row.get::<i64, _>("memory_mib") - row.get::<i64, _>("used_memory"),
                    row.get::<i64, _>("disk_bytes") - row.get::<i64, _>("used_disk"),
                ],
            );
            parents.insert(key, row.get("parent_domain_id"));
        }
        let request = profiles
            .iter()
            .map(|profile| {
                [
                    i64::from(profile.input.cpu_millis),
                    i64::from(profile.input.memory_mib),
                    i64::try_from(profile.input.disk_bytes).unwrap_or(i64::MAX),
                ]
            })
            .collect();
        Ok(Self {
            free,
            parents,
            backends,
            request,
        })
    }

    /// The domain and its ancestors, which a runner on it draws from.
    fn path(&self, (host, domain): &(String, String)) -> Vec<(String, String)> {
        let mut path = Vec::new();
        let mut next = Some(domain.clone());
        while let Some(domain) = next {
            let key = (host.clone(), domain);
            if path.contains(&key) {
                break;
            }
            next = self.parents.get(&key).cloned().flatten();
            path.push(key);
        }
        path
    }

    /// Runners of platform `index` that still fit on this backend.
    fn fits(&self, index: usize, backend: &(String, String)) -> i64 {
        let request = self.request[index];
        self.path(backend)
            .iter()
            .map(|domain| {
                let free = self.free.get(domain).copied().unwrap_or_default();
                (0..3)
                    .map(|axis| {
                        if request[axis] > 0 {
                            free[axis].max(0) / request[axis]
                        } else if free[axis] >= 0 {
                            i64::MAX
                        } else {
                            0
                        }
                    })
                    .min()
                    .unwrap_or(0)
            })
            .min()
            .unwrap_or(0)
    }

    /// Runners of platform `index` that still fit across its machines.
    fn slots(&self, index: usize) -> i64 {
        self.backends[index]
            .iter()
            .map(|backend| self.fits(index, backend))
            .fold(0, i64::saturating_add)
    }

    /// Takes one runner's room on the platform's roomiest machine, if any has room.
    fn take(&mut self, index: usize) {
        let Some(backend) = self.backends[index]
            .iter()
            .filter(|backend| self.fits(index, backend) > 0)
            .max_by_key(|backend| self.fits(index, backend))
            .cloned()
        else {
            return;
        };
        let request = self.request[index];
        for domain in self.path(&backend) {
            if let Some(free) = self.free.get_mut(&domain) {
                for axis in 0..3 {
                    free[axis] -= request[axis];
                }
            }
        }
    }
}

/// Whether a fleet machine can plausibly take this pool's next runner now:
/// an authorized machine for the platform, and none of the pool's runners
/// already waiting for a machine on it (waiting means the machines are full).
pub async fn machine_ready(
    pool: &SqlitePool,
    pool_id: &str,
    profile_id: &str,
) -> Result<bool, FleetError> {
    let now = crate::now_millis();
    Ok(sqlx::query_scalar(concat!(
        "SELECT NOT EXISTS (SELECT 1 FROM workload_intents WHERE pool_id=? AND profile_id=? AND status='queued') AND ",
        authorized_machine!()
    ))
    .bind(pool_id)
    .bind(profile_id)
    .bind(profile_id)
    .bind(now - 45_000)
    .bind(now)
    .fetch_one(pool)
    .await?)
}

/// Whether any machine could run this platform's runners at all: one is
/// online with work authority, whether or not it has room right now.
pub async fn machine_available(pool: &SqlitePool, profile_id: &str) -> Result<bool, FleetError> {
    let now = crate::now_millis();
    Ok(
        sqlx::query_scalar(concat!("SELECT ", authorized_machine!()))
            .bind(profile_id)
            .bind(now - 45_000)
            .bind(now)
            .fetch_one(pool)
            .await?,
    )
}

/// The pool's queued jobs that count for `profile_id`, one of `profiles`.
/// A runner for that platform should go to one of their repositories: a job
/// another platform counts would wait for this platform's machines while
/// that platform's sat idle, and its runner would cover neither queue.
pub async fn platform_jobs(
    pool: &SqlitePool,
    pool_id: &str,
    configured_labels: &[String],
    profiles: &[Profile],
    profile_id: &str,
) -> Result<Vec<crate::QueuedJob>, FleetError> {
    let platform = profiles.iter().position(|profile| profile.id == profile_id);
    let available = machines_available(pool, profiles).await?;
    let runners = pool_runners(pool, pool_id).await?;
    let (jobs, _) = attribute_queued_jobs(
        pool,
        pool_id,
        configured_labels,
        profiles,
        &available,
        &runners,
    )
    .await?;
    Ok(jobs
        .into_iter()
        .filter(|(_, index)| Some(*index) == platform)
        .map(|(job, _)| job)
        .collect())
}

/// The pool's fleet runners that are up or on their way: each one's platform,
/// the repository it is registered to, and whether it is on a job.
async fn pool_runners(
    pool: &SqlitePool,
    pool_id: &str,
) -> Result<Vec<sqlx::sqlite::SqliteRow>, FleetError> {
    Ok(sqlx::query(
        "SELECT i.profile_id,r.target_repository_id,r.busy
           FROM runners r JOIN workload_intents i ON i.workload_id=r.id
          WHERE r.pool_id=? AND r.deleted_at IS NULL
            AND r.status IN ('starting','online','idle','busy','paused','stopped')",
    )
    .bind(pool_id)
    .fetch_all(pool)
    .await?)
}

async fn machines_available(
    pool: &SqlitePool,
    profiles: &[Profile],
) -> Result<Vec<bool>, FleetError> {
    let mut available = Vec::with_capacity(profiles.len());
    for profile in profiles {
        available.push(machine_available(pool, &profile.id).await?);
    }
    Ok(available)
}

/// The pool's queued jobs, each with the platform (its index in `profiles`)
/// it counts for, and the room left on the platforms' machines once they
/// have theirs. Jobs no platform can run are left out.
///
/// A job counts for the platform of one of `runners` not yet on a job that
/// can take it. Otherwise it needs room: runners waiting for a machine take
/// theirs first, then jobs only one platform can run. A job several platforms
/// can run counts for the one with the most room left on its machines, so
/// work spreads across the fleet instead of queueing on the first platform
/// while another machine sits idle. With no room anywhere it counts for the
/// first platform with a machine available, else the first that can run it.
async fn attribute_queued_jobs(
    pool: &SqlitePool,
    pool_id: &str,
    configured_labels: &[String],
    profiles: &[Profile],
    available: &[bool],
    runners: &[sqlx::sqlite::SqliteRow],
) -> Result<(Vec<(crate::QueuedJob, usize)>, Room), FleetError> {
    let mut room = Room::load(pool, profiles).await?;
    let waiting_runners: Vec<String> = sqlx::query_scalar(
        "SELECT profile_id FROM workload_intents WHERE pool_id=? AND status='queued'",
    )
    .bind(pool_id)
    .fetch_all(pool)
    .await?;
    for profile_id in waiting_runners {
        if let Some(index) = profiles.iter().position(|profile| profile.id == profile_id) {
            room.take(index);
        }
    }
    // Per platform: the repository, if any, of each runner not yet on a job.
    // Such a runner takes a queued job it can run wherever it is.
    let mut spare = profiles
        .iter()
        .map(|profile| {
            runners
                .iter()
                .filter(|row| {
                    row.get::<String, _>("profile_id") == profile.id && !row.get::<bool, _>("busy")
                })
                .map(|row| row.get::<Option<i64>, _>("target_repository_id"))
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    let mut jobs = crate::queued_job_assignments(pool, pool_id)
        .await?
        .into_iter()
        .map(|job| {
            let platforms = (0..profiles.len())
                .filter(|index| {
                    platform_supports_labels(
                        profiles[*index].input.execution_os,
                        profiles[*index].input.architecture,
                        &job.labels,
                        configured_labels,
                    )
                })
                .collect::<Vec<_>>();
            (platforms, job)
        })
        .collect::<Vec<_>>();
    jobs.sort_by_key(|(platforms, _)| platforms.len() > 1);
    let mut attributed = Vec::with_capacity(jobs.len());
    for (platforms, job) in jobs {
        // A job a spare runner covers stays with it; counting it elsewhere
        // because that runner now takes up room would start a second runner.
        let covered = platforms.iter().copied().find(|index| {
            spare[*index]
                .iter()
                .position(|repository| {
                    repository.is_none_or(|repository| repository == job.repository_id)
                })
                .map(|at| spare[*index].swap_remove(at))
                .is_some()
        });
        let index = covered.or_else(|| {
            let index = if let [only] = platforms.as_slice() {
                *only
            } else {
                platforms
                    .iter()
                    .copied()
                    .filter(|index| room.slots(*index) > 0)
                    .max_by_key(|index| (room.slots(*index), std::cmp::Reverse(*index)))
                    .or_else(|| platforms.iter().copied().find(|index| available[*index]))
                    .or_else(|| platforms.first().copied())?
            };
            room.take(index);
            Some(index)
        });
        if let Some(index) = index {
            attributed.push((job, index));
        }
    }
    Ok((attributed, room))
}

/// The platform a pool's next runner should use: the one with the
/// longest-waiting prioritised job still short of a runner, else the one
/// furthest behind the queued jobs it would run, else the first with room on a
/// machine, else the first with a machine available, else the first. The flag
/// says whether that platform is behind. Each job counts for one platform, as
/// [`attribute_queued_jobs`] says.
pub async fn choose_platform<'a>(
    pool: &SqlitePool,
    pool_id: &str,
    configured_labels: &[String],
    queue_scale_factor: i64,
    profiles: &'a [Profile],
) -> Result<Option<(&'a Profile, bool)>, FleetError> {
    let available = machines_available(pool, profiles).await?;
    let mut queued = vec![0_i64; profiles.len()];
    // Repositories with queued jobs each platform would run.
    let mut waiting = vec![std::collections::HashSet::<i64>::new(); profiles.len()];
    // Per platform and repository: prioritised jobs, and when the oldest was
    // queued.
    let mut prioritised = vec![std::collections::HashMap::<i64, (i64, i64)>::new(); profiles.len()];
    let rows = pool_runners(pool, pool_id).await?;
    let (jobs, room) = attribute_queued_jobs(
        pool,
        pool_id,
        configured_labels,
        profiles,
        &available,
        &rows,
    )
    .await?;
    for (job, index) in jobs {
        queued[index] += 1;
        waiting[index].insert(job.repository_id);
        if job.prioritised {
            let (count, since) = prioritised[index]
                .entry(job.repository_id)
                .or_insert((0, job.queued_at));
            *count += 1;
            *since = (*since).min(job.queued_at);
        }
    }
    // An idle runner registered to a repository with no queued job its
    // platform can run cannot take another repository's job, so it doesn't
    // count toward covering the queue.
    let deficit = |index: usize| {
        let (mut active, mut busy) = (0_i64, 0_i64);
        for row in rows
            .iter()
            .filter(|row| row.get::<String, _>("profile_id") == profiles[index].id)
        {
            let is_busy = row.get::<bool, _>("busy");
            let useful = is_busy
                || row
                    .get::<Option<i64>, _>("target_repository_id")
                    .is_none_or(|repository| waiting[index].contains(&repository));
            if useful {
                active += 1;
                busy += i64::from(is_busy);
            }
        }
        busy.saturating_add(queued[index].saturating_mul(queue_scale_factor))
            .saturating_sub(active)
    };
    // When the oldest prioritised job this platform would run was queued,
    // among repositories with more of them than idle runners to take them.
    let unmet_priority = |index: usize| {
        prioritised[index]
            .iter()
            .filter(|(repository, (count, _))| {
                let idle = rows
                    .iter()
                    .filter(|row| {
                        row.get::<String, _>("profile_id") == profiles[index].id
                            && !row.get::<bool, _>("busy")
                            && row.get::<Option<i64>, _>("target_repository_id")
                                == Some(**repository)
                    })
                    .count();
                count.saturating_mul(queue_scale_factor) > i64::try_from(idle).unwrap_or(i64::MAX)
            })
            .map(|(_, (_, since))| *since)
            .min()
    };
    Ok((0..profiles.len())
        .filter_map(|index| unmet_priority(index).map(|since| (since, index)))
        .min()
        .map(|(_, index)| index)
        .or_else(|| {
            (0..profiles.len())
                .filter(|index| deficit(*index) > 0)
                .max_by_key(|index| (deficit(*index), std::cmp::Reverse(*index)))
        })
        .map(|index| (&profiles[index], true))
        .or_else(|| {
            (0..profiles.len())
                .find(|index| room.slots(*index) > 0)
                .or_else(|| (0..profiles.len()).find(|index| available[*index]))
                .or((!profiles.is_empty()).then_some(0))
                .map(|index| (&profiles[index], false))
        }))
}
