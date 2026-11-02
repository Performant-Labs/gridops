//! Starting a runner again after its placement failed to start.
//!
//! When a placement fails before its runner started, and nothing of it can
//! exist on the machine any more, the runner is replaced by a new one (a new
//! runner row, name and registration, under a new intent) that admission
//! places with the machines the earlier attempts failed on tried last. The
//! replacement is an ordinary runner: it goes through prepare, registration
//! and start like any other.
//!
//! Safety: the same runner never runs on two machines. A new placement is
//! only made for a failed placement whose local absence and delayed-start
//! exclusion are proven, whose capacity is released, and whose prepare and
//! start commands are all settled; an `uncertain` placement is never retried.
//! The replacement is a different runner with its own one-time registration,
//! so even a start that somehow survived could not take the replacement's
//! job. One failed placement is retried at most once (a unique index on the
//! intent enforces it), and a runner chain gets at most
//! [`MAX_START_ATTEMPTS`] placements.

use serde_json::{Value, json};
use sqlx::{Row as _, SqlitePool};
use uuid::Uuid;

use super::{
    FleetError, Principal, SubmitIntent, WorkloadKind,
    failure::{PLACEMENT_FAILED_EVENT, PlacementFailureReason},
    ids::{Generation, OperationId, PlacementId, WorkloadId},
    service::submit_intent_in,
    start_health::is_start_failure,
};

/// The `fleet_events.kind` recorded, on the machine where the start failed,
/// when a failed start is retried.
pub const PLACEMENT_RETRIED_EVENT: &str = "placement_retried";

/// Placements one runner gets: the first and up to two retries. A start that
/// fails three times on different machines is not a machine problem a fourth
/// attempt would fix; the pool's own provisioning takes over from there.
pub const MAX_START_ATTEMPTS: i64 = 3;

/// A failure older than this is not retried. The reconciler retries within a
/// pass of the failure; a placement that failed longer ago belongs to a runner
/// that was wanted then and may not be now.
pub const RETRY_WINDOW_MILLIS: i64 = 5 * 60_000;

/// The replacement made for a failed start.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Retry {
    /// The new runner row, which is also the new intent's workload.
    pub runner_id: String,
    pub operation_id: OperationId,
    /// Which placement of the runner chain this is: 2 for the first retry.
    pub attempt: i64,
}

/// Why a failed placement was not retried.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotRetried {
    NotFound,
    /// Only CI runners are retried.
    NotARunner,
    /// The placement has not failed; an `uncertain` one never is retried.
    NotFailed,
    /// The runner was removed, retired or already replaced.
    RunnerNotWanted,
    /// Nothing proves the failed placement can't still start on its machine.
    NotProvenAbsent,
    /// The failure has no recorded cause.
    NoRecordedFailure,
    FailureTooOld,
    /// The cause was not the machine's (an administrator or the pool withdrew
    /// it, or the request itself was invalid).
    NotAStartFailure,
    AttemptsExhausted,
    AlreadyRetried,
    /// The pool takes no new runners now.
    PoolNotProvisioning,
    /// The platform was switched off or changed since the runner was placed.
    ProfileChanged,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RetryOutcome {
    Retried(Retry),
    NotRetried(NotRetried),
}

/// Replaces the runner of a placement that failed to start with a new runner
/// placed away from the machines its attempts failed on. Safe to call for any
/// placement, any number of times: it acts once, and only when every
/// condition in the module documentation holds.
pub async fn retry_failed_start(
    pool: &SqlitePool,
    placement: PlacementId,
) -> Result<RetryOutcome, FleetError> {
    let id = placement.to_string();
    // Most callers ask about placements that failed long ago; answer them
    // without taking the write lock.
    let Some((state, runner_status)) = sqlx::query_as::<_, (String, Option<String>)>(
        "SELECT p.state,r.status FROM workload_placements p
           LEFT JOIN runners r ON r.id=p.workload_id AND r.deleted_at IS NULL
          WHERE p.id=?",
    )
    .bind(&id)
    .fetch_optional(pool)
    .await?
    else {
        return Ok(RetryOutcome::NotRetried(NotRetried::NotFound));
    };
    if state != "failed" {
        return Ok(RetryOutcome::NotRetried(NotRetried::NotFailed));
    }
    if runner_status.as_deref() != Some("starting") {
        return Ok(RetryOutcome::NotRetried(NotRetried::RunnerNotWanted));
    }

    let mut tx = pool.begin_with("BEGIN IMMEDIATE").await?;
    let now = crate::now_millis();
    let Some(row) = sqlx::query(
        "SELECT p.state,p.host_id,p.target_id,p.workload_id,
                p.local_absence_proven_at IS NOT NULL AND p.delayed_starts_excluded_at IS NOT NULL
                  AS absence_proven,
                EXISTS (SELECT 1 FROM capacity_allocations a
                         WHERE a.placement_id=p.id AND a.state!='released') AS holds_capacity,
                EXISTS (SELECT 1 FROM host_commands c
                         WHERE c.placement_id=p.id AND c.kind IN ('prepare_environment','start_runner')
                           AND c.state IN ('pending','delivered','accepted','running','uncertain'))
                  AS unsettled,
                i.id AS intent_id,i.kind,i.pool_id,i.profile_id,i.expected_profile_revision,
                i.start_attempt,i.avoid_host_ids_json,
                r.status AS runner_status,r.target_repository_id,
                rp.name AS pool_name,rp.paused,rp.state AS pool_state,rp.configuration_version,
                EXISTS (SELECT 1 FROM settings WHERE key='provisioningPaused' AND value='true')
                  AS provisioning_paused,
                pr.enabled AS profile_enabled,pr.archived_at,pr.revision AS profile_revision,
                EXISTS (SELECT 1 FROM workload_intents retry WHERE retry.retry_of_placement_id=p.id)
                  AS already_retried
           FROM workload_placements p
           JOIN workload_intents i ON i.id=p.intent_id
           JOIN runner_pools rp ON rp.id=i.pool_id
           JOIN pool_execution_profiles pr ON pr.id=i.profile_id
           LEFT JOIN runners r ON r.id=p.workload_id AND r.deleted_at IS NULL
          WHERE p.id=?",
    )
    .bind(&id)
    .fetch_optional(&mut *tx)
    .await?
    else {
        return Ok(RetryOutcome::NotRetried(NotRetried::NotFound));
    };
    let failure = sqlx::query(
        "SELECT created_at,detail_json FROM fleet_events
          WHERE placement_id=? AND kind=? ORDER BY cursor DESC LIMIT 1",
    )
    .bind(&id)
    .bind(PLACEMENT_FAILED_EVENT)
    .fetch_optional(&mut *tx)
    .await?;
    let attempt: i64 = row.get("start_attempt");
    let refusal = if row.get::<String, _>("kind") != WorkloadKind::CiRunner.as_str() {
        Some(NotRetried::NotARunner)
    } else if row.get::<String, _>("state") != "failed" {
        Some(NotRetried::NotFailed)
    } else if row.get::<Option<String>, _>("runner_status").as_deref() != Some("starting") {
        Some(NotRetried::RunnerNotWanted)
    } else if row.get::<bool, _>("already_retried") {
        Some(NotRetried::AlreadyRetried)
    } else if !row.get::<bool, _>("absence_proven")
        || row.get::<bool, _>("holds_capacity")
        || row.get::<bool, _>("unsettled")
    {
        Some(NotRetried::NotProvenAbsent)
    } else {
        None
    };
    if let Some(refusal) = refusal {
        return Ok(RetryOutcome::NotRetried(refusal));
    }
    let Some(failure) = failure else {
        return Ok(RetryOutcome::NotRetried(NotRetried::NoRecordedFailure));
    };
    let detail: Value =
        serde_json::from_str(&failure.get::<String, _>("detail_json")).unwrap_or_default();
    let reason = PlacementFailureReason::from_code(
        detail.get("reason").and_then(Value::as_str).unwrap_or(""),
    );
    let refusal = if failure.get::<i64, _>("created_at") < now - RETRY_WINDOW_MILLIS {
        Some(NotRetried::FailureTooOld)
    } else if !is_start_failure(reason, detail.get("agent_code").and_then(Value::as_str)) {
        Some(NotRetried::NotAStartFailure)
    } else if attempt >= MAX_START_ATTEMPTS {
        Some(NotRetried::AttemptsExhausted)
    } else if row.get::<bool, _>("paused")
        || matches!(
            row.get::<String, _>("pool_state").as_str(),
            "paused" | "draining" | "deleting"
        )
        || row.get::<bool, _>("provisioning_paused")
    {
        Some(NotRetried::PoolNotProvisioning)
    } else if !row.get::<bool, _>("profile_enabled")
        || row.get::<Option<i64>, _>("archived_at").is_some()
        || row.get::<i64, _>("profile_revision") != row.get::<i64, _>("expected_profile_revision")
    {
        Some(NotRetried::ProfileChanged)
    } else {
        None
    };
    if let Some(refusal) = refusal {
        return Ok(RetryOutcome::NotRetried(refusal));
    }

    let failed_host: String = row.get("host_id");
    let mut avoided: Vec<String> =
        serde_json::from_str(&row.get::<String, _>("avoid_host_ids_json")).unwrap_or_default();
    if !avoided.contains(&failed_host) {
        avoided.push(failed_host.clone());
    }
    let old_runner: String = row.get("workload_id");
    let runner_id = Uuid::new_v4();
    let suffix = Uuid::new_v4().simple().to_string()[..8].to_owned();
    let name = format!("{}-{suffix}", row.get::<String, _>("pool_name"));
    // The replacement is the same kind of runner for the same repository, on
    // the pool's current configuration.
    sqlx::query(
        "INSERT INTO runners (id,pool_id,target_repository_id,name,provider,ci_platform,os,architecture,
           status,ephemeral,configuration_version,runtime,created_at,updated_at)
         SELECT ?,pool_id,target_repository_id,?,provider,ci_platform,os,architecture,
                'starting',ephemeral,?,runtime,?,?
           FROM runners WHERE id=?",
    )
    .bind(runner_id.to_string())
    .bind(&name)
    .bind(row.get::<i64, _>("configuration_version"))
    .bind(now)
    .bind(now)
    .bind(&old_runner)
    .execute(&mut *tx)
    .await?;
    let submission = submit_intent_in(
        &mut tx,
        &Principal::Reconciler,
        SubmitIntent {
            idempotency_key: format!("runner:{runner_id}"),
            pool_id: row.get("pool_id"),
            profile_id: parse(row.get("profile_id"))?,
            target_id: parse(row.get("target_id"))?,
            workload_id: WorkloadId::try_new(runner_id)
                .map_err(|_| FleetError::Invariant("runner ID"))?,
            generation: Generation::new(1).map_err(|_| FleetError::Invariant("generation"))?,
            kind: WorkloadKind::CiRunner,
            agent_run_id: None,
            expected_profile_revision: u64::try_from(row.get::<i64, _>("profile_revision")).ok(),
        },
    )
    .await?;
    let next_attempt = attempt + 1;
    sqlx::query(
        "UPDATE workload_intents SET start_attempt=?,retry_of_placement_id=?,avoid_host_ids_json=?
          WHERE id=?",
    )
    .bind(next_attempt)
    .bind(&id)
    .bind(json!(avoided).to_string())
    .bind(submission.operation_id.to_string())
    .execute(&mut *tx)
    .await?;
    // The runner that failed is done; the pool counts its replacement instead.
    sqlx::query("UPDATE runners SET status='failed',updated_at=? WHERE id=? AND status='starting'")
        .bind(now)
        .bind(&old_runner)
        .execute(&mut *tx)
        .await?;
    let message = format!(
        "Started the runner again as {name} (attempt {next_attempt} of {MAX_START_ATTEMPTS}), trying other machines first."
    );
    sqlx::query(
        "INSERT INTO fleet_events (id,host_id,placement_id,target_id,kind,detail_json,created_at)
         VALUES (?,?,?,?,?,?,?)",
    )
    .bind(Uuid::new_v4().to_string())
    .bind(&failed_host)
    .bind(&id)
    .bind(row.get::<String, _>("target_id"))
    .bind(PLACEMENT_RETRIED_EVENT)
    .bind(
        json!({
            "attempt": next_attempt,
            "max_attempts": MAX_START_ATTEMPTS,
            "failure_reason": reason.as_str(),
            "retry_runner_id": runner_id.to_string(),
            "retry_operation_id": submission.operation_id.to_string(),
            "avoided_host_ids": avoided,
            "message": message,
        })
        .to_string(),
    )
    .bind(now)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(RetryOutcome::Retried(Retry {
        runner_id: runner_id.to_string(),
        operation_id: submission.operation_id,
        attempt: next_attempt,
    }))
}

fn parse<T: std::str::FromStr>(value: String) -> Result<T, FleetError> {
    value
        .parse()
        .map_err(|_| FleetError::InvalidIdentifier(value))
}
