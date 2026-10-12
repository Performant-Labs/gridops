//! Runs pools with an enabled execution profile on fleet hosts.
//!
//! Scaling decisions stay with the pool logic in `main.rs`: a fleet runner is
//! an ordinary `runners` row, so capacity counting, scale-down and GitHub
//! cleanup work unchanged. The difference is how a runner comes to exist.
//! Instead of asking the manager for a container, the reconciler submits a
//! fleet intent; admission places it on a host and the agent prepares it.
//! This module then registers the runner with GitHub, hands the agent the
//! one-time registration through a start command, follows the runner while it
//! runs, and cleans it up when it exits or is deleted.
//!
//! Fleet runner rows never get a `container_id`, so the manager-based status
//! sync leaves them alone.

use anyhow::{Context as _, Result};
use gridops_core::{
    JitRequest, RunnerTarget, effective_runner_labels,
    fleet::{
        CiTarget, ExternalId, FleetService, Principal, SubmitIntent, WorkloadKind,
        ids::{Generation, ProfileId, WorkloadId},
        placement::{self, Placement},
        protocol::{
            agent::{AgentOperation, LocalResourceIdentity, RunnerMode, StartRunner},
            outcomes::SuccessEvidence,
            primitives::WireTimestamp,
        },
        targets::{self, Profile},
    },
    next_runner_repository, now_millis,
};
use serde_json::json;
use sqlx::Row as _;

use super::{Pool, ProvisionAttempt, Reconciler, installation_token, runner_registration_url};

/// A queued intent that no host could take within this long is given up.
const QUEUE_TIMEOUT_MILLIS: i64 = 10 * 60_000;
const INSPECT_INTERVAL_MILLIS: i64 = 30_000;
/// The agent has this long to fetch registration material and start.
const START_WINDOW_MILLIS: i64 = 60_000;

/// The pool's profile when it's enabled for fleet execution.
pub(super) async fn profile(app: &Reconciler, pool_id: &str) -> Result<Option<Profile>> {
    Ok(targets::pool_profile(&app.database, pool_id)
        .await?
        .filter(|profile| profile.input.enabled))
}

/// Creates one fleet runner for the pool and tries to place it.
pub(super) async fn provision(
    app: &Reconciler,
    pool: &Pool,
    profile: &Profile,
) -> Result<ProvisionAttempt> {
    let (target_id, repository_id) = if pool.scope == "repository" {
        let repository = next_runner_repository(&app.database, &pool.id, pool.queue_scale_factor)
            .await?
            .context("runner pool has no available repositories")?;
        let target = targets::ensure_target(
            &app.database,
            &CiTarget::GithubRepository {
                installation_id: external(repository.installation_id)?,
                repository_id: external(repository.repository_id)?,
            },
        )
        .await?;
        (target, Some(repository.repository_id))
    } else {
        let target = targets::pool_targets(&app.database, &pool.id)
            .await?
            .into_iter()
            .next()
            .context("organization pool has no fleet target")?;
        (target, None)
    };
    let runner_id = uuid::Uuid::new_v4();
    let suffix = uuid::Uuid::new_v4().simple().to_string()[..8].to_owned();
    let name = format!("{}-{suffix}", pool.name);
    let docker = profile.input.runtime_kind == gridops_core::fleet::RuntimeKind::Docker;
    let ephemeral = profile.input.mode == RunnerMode::Ephemeral;
    let now = now_millis();
    sqlx::query(
        "INSERT INTO runners (id,pool_id,target_repository_id,name,provider,ci_platform,os,architecture,
           status,ephemeral,configuration_version,runtime,created_at,updated_at)
         VALUES (?,?,?,?,?,'github',?,?,'starting',?,?,?,?,?)",
    )
    .bind(runner_id.to_string())
    .bind(&pool.id)
    .bind(repository_id)
    .bind(&name)
    .bind(if docker { "docker" } else { "tart" })
    .bind(match profile.input.execution_os {
        gridops_core::fleet::ExecutionOs::Linux => "linux",
        gridops_core::fleet::ExecutionOs::Macos => "macOS",
        gridops_core::fleet::ExecutionOs::Windows => "windows",
    })
    .bind(match profile.input.architecture {
        gridops_core::fleet::Architecture::X64 => "X64",
        gridops_core::fleet::Architecture::Arm64 => "ARM64",
    })
    .bind(ephemeral)
    .bind(pool.configuration_version)
    .bind(if docker { "vm" } else { "native" })
    .bind(now)
    .bind(now)
    .execute(&app.database)
    .await?;
    let service = FleetService::new(app.database.clone());
    let submitted = service
        .submit_intent(
            &Principal::Reconciler,
            SubmitIntent {
                idempotency_key: format!("runner:{runner_id}"),
                pool_id: pool.id.clone(),
                profile_id: profile.id.parse::<ProfileId>()?,
                target_id: target_id.parse()?,
                workload_id: WorkloadId::try_new(runner_id)?,
                generation: Generation::new(1)?,
                kind: WorkloadKind::CiRunner,
                agent_run_id: None,
                expected_profile_revision: u64::try_from(profile.revision).ok(),
            },
        )
        .await;
    if let Err(error) = submitted {
        sqlx::query("UPDATE runners SET status='failed',failure_reason=?,updated_at=? WHERE id=?")
            .bind("Fleet placement request was rejected.")
            .bind(now_millis())
            .bind(runner_id.to_string())
            .execute(&app.database)
            .await?;
        return Err(error.into());
    }
    service.admit_queued(&Principal::Reconciler, 100).await?;
    Ok(ProvisionAttempt::Provisioned)
}

/// One fleet pass: admit waiting work, then move every placement forward.
pub(super) async fn tick(app: &Reconciler) -> Result<()> {
    let service = FleetService::new(app.database.clone());
    service.admit_queued(&Principal::Reconciler, 100).await?;
    expire_queued(app, &service).await?;
    for placement in placement::placements(&app.database, &["registering"]).await? {
        if let Err(error) = register_and_start(app, &placement).await {
            tracing::warn!(placement = %placement.id, error = ?error, "could not start fleet runner");
        }
    }
    for placement in
        placement::placements(&app.database, &["starting", "running", "stopped"]).await?
    {
        if let Err(error) = follow(app, &placement).await {
            tracing::warn!(placement = %placement.id, error = ?error, "could not follow fleet runner");
        }
    }
    for placement in
        placement::placements(&app.database, &["failed", "cleaned", "uncertain"]).await?
    {
        finish(app, &placement).await?;
    }
    Ok(())
}

/// Runners whose intent never found a host give up, so the pool can retry.
async fn expire_queued(app: &Reconciler, service: &FleetService) -> Result<()> {
    let stale = sqlx::query(
        "SELECT i.id,i.workload_id FROM workload_intents i JOIN runners r ON r.id=i.workload_id
          WHERE i.status='queued' AND i.created_at<? AND r.status='starting'",
    )
    .bind(now_millis() - QUEUE_TIMEOUT_MILLIS)
    .fetch_all(&app.database)
    .await?;
    for row in stale {
        let intent: String = row.get("id");
        if service.cancel_queued(&intent, "no_capacity").await? {
            sqlx::query(
                "UPDATE runners SET status='failed',failure_reason=?,updated_at=? WHERE id=?",
            )
            .bind("No fleet host had capacity for this runner.")
            .bind(now_millis())
            .bind(row.get::<String, _>("workload_id"))
            .execute(&app.database)
            .await?;
        }
    }
    Ok(())
}

/// The environment is prepared: register the runner upstream and start it.
async fn register_and_start(app: &Reconciler, placement: &Placement) -> Result<()> {
    if !placement::host_has_authority(&app.database, placement.id).await? {
        return Ok(());
    }
    let prepared_environment_id = placement::prepared_environment(&app.database, placement.id)
        .await?
        .context("prepared environment is missing")?;
    let prepare = placement::prepared_with(&app.database, placement.id)
        .await?
        .context("prepare command is missing")?;
    let runner = sqlx::query(
        "SELECT r.id,r.name,r.provider,r.status,p.installation_id AS pool_installation,p.account_login,p.scope,
                p.labels,p.runner_group_id,repo.installation_id,repo.owner,repo.name AS repository
           FROM runners r JOIN runner_pools p ON p.id=r.pool_id
           LEFT JOIN repositories repo ON repo.id=r.target_repository_id
          WHERE r.id=?",
    )
    .bind(&placement.workload_id)
    .fetch_optional(&app.database)
    .await?
    .context("runner row is missing")?;
    if runner.get::<String, _>("status") != "starting" {
        // Deleted before it started: abandon the placement.
        placement::set_state(&app.database, placement.id, "registering", "failed").await?;
        return Ok(());
    }
    let name: String = runner.get("name");
    let owner: Option<String> = runner.get("owner");
    let repository: Option<String> = runner.get("repository");
    let account_login: String = runner.get("account_login");
    let installation_id: i64 = runner
        .get::<Option<i64>, _>("installation_id")
        .unwrap_or_else(|| runner.get("pool_installation"));
    let token = installation_token(app, installation_id)
        .await?
        .context("GitHub App credentials are unavailable")?;
    let target = match (&owner, &repository) {
        (Some(owner), Some(repository)) => RunnerTarget::Repository { owner, repository },
        _ => RunnerTarget::Organization {
            organization: &account_login,
        },
    };
    let labels: Vec<String> =
        serde_json::from_str(&runner.get::<String, _>("labels")).unwrap_or_default();
    let provider: String = runner.get("provider");
    let material = if prepare.mode == RunnerMode::Ephemeral {
        let jit = app
            .github
            .generate_jit_config(
                target,
                &token,
                &JitRequest {
                    name: name.clone(),
                    runner_group_id: runner.get("runner_group_id"),
                    labels: effective_runner_labels(&provider, &labels),
                    work_folder: "_work".into(),
                },
            )
            .await?;
        sqlx::query("UPDATE runners SET github_runner_id=?,updated_at=? WHERE id=?")
            .bind(jit.runner.id)
            .bind(now_millis())
            .bind(&placement.workload_id)
            .execute(&app.database)
            .await?;
        json!({"kind": "github_jit", "jit_config": jit.encoded_jit_config})
    } else {
        let registration = app
            .github
            .generate_registration_token(target, &token)
            .await?;
        let repository_target = sqlx::query_as::<_, (i64, i64, String, String)>(
            "SELECT id,installation_id,owner,name FROM repositories WHERE owner=? AND name=?",
        )
        .bind(owner.clone().unwrap_or_default())
        .bind(repository.clone().unwrap_or_default())
        .fetch_optional(&app.database)
        .await?
        .map(
            |(repository_id, installation_id, owner, name)| gridops_core::RepositoryCapacity {
                repository_id,
                installation_id,
                owner,
                name,
                queued: 0,
                active: 0,
                busy: 0,
            },
        );
        json!({
            "kind": "github_registration",
            "token": registration.token,
            "url": runner_registration_url(&account_login, repository_target.as_ref()),
            "name": name,
            "labels": effective_runner_labels(&provider, &labels),
            "group": null,
        })
    };
    let now = now_millis();
    let reference = placement::store_bootstrap(
        &app.database,
        &app.vault,
        placement.id,
        &material.to_string(),
        START_WINDOW_MILLIS,
    )
    .await?;
    let operation = AgentOperation::StartRunner(StartRunner {
        prepared_environment_id,
        execution_profile_id: prepare.execution_profile_id,
        profile_revision: prepare.profile_revision,
        bootstrap_reference: reference,
        bootstrap_expires_at: WireTimestamp::from_millis(now + START_WINDOW_MILLIS)?,
        resource_request: prepare.resource_request,
        allocation_ids: prepare.allocation_ids,
    });
    if placement::issue_command(
        &app.database,
        placement.id,
        operation,
        &format!("start:{}", placement.id),
        START_WINDOW_MILLIS - 5_000,
    )
    .await?
    .is_some()
    {
        placement::mark_starting(&app.database, placement.id, &placement.workload_id).await?;
    }
    Ok(())
}

/// Tracks a started runner: mark it online, check it's still running, and
/// clean it up once it exits or its runner row is deleted.
async fn follow(app: &Reconciler, placement: &Placement) -> Result<()> {
    let runner_status: Option<String> = sqlx::query_scalar("SELECT status FROM runners WHERE id=?")
        .bind(&placement.workload_id)
        .fetch_optional(&app.database)
        .await?;
    let started =
        placement::succeeded_evidence(&app.database, placement.id, "start_runner").await?;
    let identity = match &started {
        Some(SuccessEvidence::RunnerStarted { local_identity, .. }) => Some(local_identity.clone()),
        _ => None,
    };
    if placement.state == "starting" {
        let stuck = placement::command_state(&app.database, placement.id, "start_runner")
            .await?
            .is_some_and(|(state, issued)| {
                matches!(state.as_str(), "cancelled") || issued < now_millis() - 5 * 60_000
            });
        if stuck {
            placement::set_state(&app.database, placement.id, "starting", "uncertain").await?;
        }
        return Ok(());
    }
    let Some(identity) = identity else {
        return Ok(());
    };
    if placement.state == "running" && runner_status.as_deref() == Some("starting") {
        let now = now_millis();
        sqlx::query(
            "UPDATE runners SET status='online',container_name=?,registered_at=?,last_heartbeat_at=?,updated_at=? WHERE id=? AND status='starting'",
        )
        .bind(match &identity {
            LocalResourceIdentity::Docker { object_id } => object_id.as_str().chars().take(12).collect(),
            LocalResourceIdentity::TartVm { name } => name.as_str().to_owned(),
            LocalResourceIdentity::NativeProcess { process_id, .. } => format!("pid-{}", process_id.get()),
        })
        .bind(now)
        .bind(now)
        .bind(now)
        .bind(&placement.workload_id)
        .execute(&app.database)
        .await?;
    }
    let deleted = matches!(runner_status.as_deref(), None | Some("deleted" | "failed"));
    let exited = placement::reported_gone(&app.database, placement.id, "inspect").await?;
    if deleted || exited || placement.state == "stopped" {
        if !deleted {
            // The runner exited by itself (an ephemeral runner after its job).
            sqlx::query("UPDATE runners SET status='failed',failure_reason=NULL,updated_at=? WHERE id=? AND status NOT IN ('deleted','failed')")
                .bind(now_millis())
                .bind(&placement.workload_id)
                .execute(&app.database)
                .await?;
        }
        placement::issue_command(
            &app.database,
            placement.id,
            AgentOperation::Cleanup {
                expected_resource: identity,
            },
            &format!("cleanup:{}:{}", placement.id, now_millis() / 60_000),
            55_000,
        )
        .await?;
        return Ok(());
    }
    let bucket = now_millis() / INSPECT_INTERVAL_MILLIS;
    placement::issue_command(
        &app.database,
        placement.id,
        AgentOperation::Inspect {
            expected_resource: identity,
        },
        &format!("inspect:{}:{bucket}", placement.id),
        INSPECT_INTERVAL_MILLIS - 5_000,
    )
    .await?;
    Ok(())
}

/// A placement that ended: retire its runner row so the pool can replace it.
/// An uncertain placement whose runtime object is known is cleaned up, which
/// proves it gone and frees its capacity.
async fn finish(app: &Reconciler, placement: &Placement) -> Result<()> {
    if placement.state == "uncertain"
        && let Some(SuccessEvidence::RunnerStarted { local_identity, .. }) =
            placement::succeeded_evidence(&app.database, placement.id, "start_runner").await?
    {
        placement::issue_command(
            &app.database,
            placement.id,
            AgentOperation::Cleanup {
                expected_resource: local_identity,
            },
            &format!("cleanup:{}:{}", placement.id, now_millis() / 60_000),
            55_000,
        )
        .await?;
    }
    sqlx::query(
        "UPDATE runners SET status='failed',updated_at=? WHERE id=? AND status NOT IN ('deleted','failed')",
    )
    .bind(now_millis())
    .bind(&placement.workload_id)
    .execute(&app.database)
    .await?;
    Ok(())
}

fn external(value: i64) -> Result<ExternalId> {
    Ok(ExternalId::new(u64::try_from(value)?)?)
}
