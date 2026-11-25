//! Reconciler-side placement driving.
//!
//! Admission creates a placement and its prepare command. Everything after
//! that, such as starting the runner once upstream registration exists,
//! inspecting it and cleaning it up, is a command the reconciler issues
//! through here. Commands are sealed to the host's current session, so a host
//! without live work authority gets no command at all.

use sqlx::{Row as _, SqlitePool};
use uuid::Uuid;

use super::{
    FleetError,
    failure::{PlacementFailure, fail_placement},
    ids::{
        AuthoritySessionId, BackendId, BootstrapReference, CommandId, ControlPlaneIncarnation,
        Generation, HostEpoch, HostId, OperationId, PlacementId, PreparedEnvironmentId, Revision,
    },
    protocol::{
        agent::{AgentCommandEnvelope, AgentOperation, CommandScope, PrepareEnvironment},
        outcomes::{AgentCommandResult, CommandOutcome, DiagnosticCode, SuccessEvidence},
        primitives::{IdempotencyKey, ProtocolVersion, WireTimestamp},
    },
};
use crate::Vault;

/// A placement as the reconciler sees it.
#[derive(Debug, Clone)]
pub struct Placement {
    pub id: PlacementId,
    pub workload_id: String,
    pub host_id: HostId,
    pub state: String,
    pub runner_id: Option<String>,
    pub pool_id: String,
    pub profile_id: String,
    pub target_id: String,
    pub runtime_kind: String,
    pub mode: String,
    pub labels: Vec<String>,
    pub updated_at: i64,
}

/// What [`placements`] and [`ended_placements_to_finish`] read of a
/// placement: `$from` (the placements `p`, or what leads to them), then
/// `$filter` (its `WHERE` and `ORDER BY`).
macro_rules! select_placements {
    ($from:literal, $filter:literal) => {
        concat!(
            "SELECT p.id,p.workload_id,p.host_id,p.state,p.runner_id,p.target_id,p.updated_at,i.pool_id,i.profile_id,
                    pr.runtime_kind,pr.mode,pr.labels_json
               FROM ",
            $from,
            "
               JOIN workload_intents i ON i.id=p.intent_id
               JOIN pool_execution_profiles pr ON pr.id=i.profile_id
             ",
            $filter
        )
    };
}

/// Placements in any of `states`, oldest first.
pub async fn placements(pool: &SqlitePool, states: &[&str]) -> Result<Vec<Placement>, FleetError> {
    let states = serde_json::to_string(states).map_err(|_| FleetError::Invariant("states"))?;
    let rows = sqlx::query(select_placements!(
        "workload_placements p",
        "WHERE p.state IN (SELECT value FROM json_each(?))
          ORDER BY p.created_at,p.id"
    ))
    .bind(states)
    .fetch_all(pool)
    .await?;
    to_placements(rows)
}

/// The ended placements (`failed`, `cleaned` or `uncertain`) the reconciler
/// still has to finish, oldest first (#259): those in the finish queue
/// (migration 0052).
///
/// A placement joins the queue when it ends, and leaves it through
/// [`finished`] once a pass has finished it in a failed or cleaned state and
/// nothing is left to do. An uncertain placement stays, and is finished on
/// every pass until it is cleaned. So this reads the placements with work
/// left and none of the history: every placement that ended and was
/// finished, started failures included, is never read again. Reading and
/// finishing every ended placement on every pass made each pass's work, and
/// the reconciler's memory, grow with the fleet's history until the
/// reconciler was killed at its memory limit.
pub async fn ended_placements_to_finish(pool: &SqlitePool) -> Result<Vec<Placement>, FleetError> {
    // The queue leads (`CROSS JOIN` keeps that order), so the read costs one
    // primary-key lookup per queued placement.
    let rows = sqlx::query(select_placements!(
        "placement_finish_queue q CROSS JOIN workload_placements p ON p.id=q.placement_id",
        "WHERE p.state IN ('failed','cleaned','uncertain')
          ORDER BY p.created_at,p.id"
    ))
    .fetch_all(pool)
    .await?;
    to_placements(rows)
}

/// Takes `placement` off the finish queue: a pass finished it in `state`, a
/// failed or cleaned one, and nothing is left to do for it. Answers whether
/// it was taken off. A placement that has since moved on to another state
/// stays queued, so it is finished in that state too; so does one whose
/// runner row came back into service after `finish` retired it (a late job
/// event), which the queue's trigger on `runners` could not queue again while
/// it was still queued: the next pass retires the runner again.
///
/// Nothing is left once `finish` has run for a failed or cleaned placement:
/// its runner row is retired (or replaced through a retried start), and an
/// unstarted failure's capacity released and absence recorded by
/// [`abandon_unstarted`]. A started failure's capacity and absence are the
/// agent's results' to settle, never a pass's, so a pass has nothing more
/// to do for it either.
pub async fn finished(
    pool: &SqlitePool,
    placement: PlacementId,
    state: &str,
) -> Result<bool, FleetError> {
    Ok(sqlx::query(
        "DELETE FROM placement_finish_queue
          WHERE placement_id=?1 AND ?2 IN ('failed','cleaned')
            AND EXISTS (SELECT 1 FROM workload_placements p WHERE p.id=?1 AND p.state=?2
                         AND NOT EXISTS (SELECT 1 FROM runners r WHERE r.id=p.workload_id
                                          AND r.status NOT IN ('deleted','failed')))",
    )
    .bind(placement.to_string())
    .bind(state)
    .execute(pool)
    .await?
    .rows_affected()
        == 1)
}

/// Queues the placements that ended before the finish queue existed and
/// still have something to finish (migration 0052 records how far there is
/// to go): an uncertain one; one whose runner row is still in service; and
/// an unstarted failure that still holds capacity or has no recorded
/// absence. The settled ones, nearly all of them, are never queued.
///
/// The sweep goes through the placements by rowid,
/// [`retention::BATCH_SIZE`](crate::retention::BATCH_SIZE) at a time, each
/// batch its own short write transaction, with
/// [`retention::BATCH_PAUSE`](crate::retention::BATCH_PAUSE) between batches
/// so other writers get the lock (#222), and at most
/// [`retention::MAX_BATCHES`](crate::retention::MAX_BATCHES) batches a call;
/// the rest waits for the next pass. Once it is done, a call reads one row.
/// Answers how many placements it queued.
pub async fn queue_earlier_ended_placements(pool: &SqlitePool) -> Result<usize, FleetError> {
    use crate::retention::{BATCH_PAUSE, BATCH_SIZE, MAX_BATCHES};
    let mut queued = 0;
    for batch in 0..MAX_BATCHES {
        // Read without the write lock first: once the sweep is done, that
        // is all a pass does here.
        let left: Option<(i64, i64)> = sqlx::query_as(
            "SELECT next_rowid,end_rowid FROM placement_finish_sweep WHERE next_rowid<end_rowid",
        )
        .fetch_optional(pool)
        .await?;
        let Some((next, end)) = left else {
            break;
        };
        if batch > 0 {
            tokio::time::sleep(BATCH_PAUSE).await;
        }
        let upto = next.saturating_add(BATCH_SIZE).min(end);
        let mut tx = pool.begin_with("BEGIN IMMEDIATE").await?;
        let added = sqlx::query(
            "INSERT OR IGNORE INTO placement_finish_queue (placement_id)
             SELECT p.id FROM workload_placements p
              WHERE p.rowid>? AND p.rowid<=?
                AND (p.state='uncertain'
                     OR (p.state IN ('failed','cleaned')
                         AND EXISTS (SELECT 1 FROM runners r WHERE r.id=p.workload_id
                                      AND r.status NOT IN ('deleted','failed')))
                     OR (p.state='failed'
                         AND NOT EXISTS (SELECT 1 FROM host_commands c
                                          WHERE c.placement_id=p.id AND c.kind='start_runner')
                         AND (p.local_absence_proven_at IS NULL
                              OR p.delayed_starts_excluded_at IS NULL
                              OR EXISTS (SELECT 1 FROM capacity_allocations a
                                          WHERE a.placement_id=p.id AND a.state!='released'))))",
        )
        .bind(next)
        .bind(upto)
        .execute(&mut *tx)
        .await?
        .rows_affected();
        // Only from where this batch began: a sweep that another process
        // moved on in the meantime is not moved back.
        sqlx::query("UPDATE placement_finish_sweep SET next_rowid=? WHERE next_rowid=?")
            .bind(upto)
            .bind(next)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        queued += usize::try_from(added).unwrap_or(usize::MAX);
    }
    Ok(queued)
}

fn to_placements(rows: Vec<sqlx::sqlite::SqliteRow>) -> Result<Vec<Placement>, FleetError> {
    rows.into_iter()
        .map(|row| {
            Ok(Placement {
                id: parse(row.get("id"))?,
                workload_id: row.get("workload_id"),
                host_id: parse(row.get("host_id"))?,
                state: row.get("state"),
                runner_id: row.get("runner_id"),
                pool_id: row.get("pool_id"),
                profile_id: row.get("profile_id"),
                target_id: row.get("target_id"),
                runtime_kind: row.get("runtime_kind"),
                mode: row.get("mode"),
                labels: serde_json::from_str(row.get("labels_json"))
                    .map_err(|_| FleetError::Invariant("profile labels"))?,
                updated_at: row.get("updated_at"),
            })
        })
        .collect()
}

/// The environment the agent prepared for this placement, from the prepare
/// command's successful result.
pub async fn prepared_environment(
    pool: &SqlitePool,
    placement: PlacementId,
) -> Result<Option<PreparedEnvironmentId>, FleetError> {
    Ok(succeeded_evidence(pool, placement, "prepare_environment")
        .await?
        .and_then(|evidence| match evidence {
            SuccessEvidence::PreparedEnvironment {
                prepared_environment_id,
            } => Some(prepared_environment_id),
            _ => None,
        }))
}

/// What the placement was prepared with, from its prepare command.
pub async fn prepared_with(
    pool: &SqlitePool,
    placement: PlacementId,
) -> Result<Option<PrepareEnvironment>, FleetError> {
    let Some(json) = sqlx::query_scalar::<_, String>(
        "SELECT payload_json FROM host_commands WHERE placement_id=? AND kind='prepare_environment'
          ORDER BY cursor DESC LIMIT 1",
    )
    .bind(placement.to_string())
    .fetch_optional(pool)
    .await?
    else {
        return Ok(None);
    };
    let envelope: AgentCommandEnvelope =
        serde_json::from_str(&json).map_err(|_| FleetError::Invariant("stored command"))?;
    Ok(match envelope.operation {
        AgentOperation::PrepareEnvironment(prepare) => Some(prepare),
        _ => None,
    })
}

/// Whether the placement's host can receive commands right now.
pub async fn host_has_authority(
    pool: &SqlitePool,
    placement: PlacementId,
) -> Result<bool, FleetError> {
    Ok(sqlx::query_scalar::<_, bool>(
        "SELECT h.authority_expires_at>? AND p.host_epoch=h.epoch FROM workload_placements p
           JOIN fleet_hosts h ON h.id=p.host_id WHERE p.id=?",
    )
    .bind(crate::now_millis() + 5_000)
    .bind(placement.to_string())
    .fetch_optional(pool)
    .await?
    .unwrap_or(false))
}

/// Evidence from the latest applied, successful command of `kind`.
pub async fn succeeded_evidence(
    pool: &SqlitePool,
    placement: PlacementId,
    kind: &str,
) -> Result<Option<SuccessEvidence>, FleetError> {
    let Some(json) = sqlx::query_scalar::<_, String>(
        "SELECT r.result_json FROM host_command_results r JOIN host_commands c ON c.id=r.command_id
          WHERE c.placement_id=? AND c.kind=? AND c.state='succeeded' AND r.disposition='applied'
          ORDER BY r.received_at DESC LIMIT 1",
    )
    .bind(placement.to_string())
    .bind(kind)
    .fetch_optional(pool)
    .await?
    else {
        return Ok(None);
    };
    let result: AgentCommandResult =
        serde_json::from_str(&json).map_err(|_| FleetError::Invariant("stored result"))?;
    Ok(match result.outcome {
        CommandOutcome::Succeeded { evidence } => Some(*evidence),
        _ => None,
    })
}

/// Whether the newest result of `kind` reported the runtime object gone: a
/// failed inspect with `resource_unavailable` means it exited or was removed.
pub async fn reported_gone(
    pool: &SqlitePool,
    placement: PlacementId,
    kind: &str,
) -> Result<bool, FleetError> {
    let Some(json) = sqlx::query_scalar::<_, String>(
        "SELECT r.result_json FROM host_command_results r JOIN host_commands c ON c.id=r.command_id
          WHERE c.placement_id=? AND c.kind=? AND r.disposition='applied'
          ORDER BY r.received_at DESC LIMIT 1",
    )
    .bind(placement.to_string())
    .bind(kind)
    .fetch_optional(pool)
    .await?
    else {
        return Ok(false);
    };
    let result: AgentCommandResult =
        serde_json::from_str(&json).map_err(|_| FleetError::Invariant("stored result"))?;
    Ok(matches!(
        result.outcome,
        CommandOutcome::Failed {
            code: crate::fleet::protocol::outcomes::FailureCode::ResourceUnavailable,
            ..
        }
    ))
}

/// The diagnostic code of the newest applied result of the placement's `kind`
/// command, when that result is a failure that carries one (#139): a GitLab
/// runner's `IdleExpired` or `RegistrationRefused`. `None` when there is no
/// such result, it did not fail, or it failed without a diagnostic (a
/// completed GitLab runner).
pub async fn reported_diagnostic(
    pool: &SqlitePool,
    placement: PlacementId,
    kind: &str,
) -> Result<Option<DiagnosticCode>, FleetError> {
    let Some(json) = sqlx::query_scalar::<_, String>(
        "SELECT r.result_json FROM host_command_results r JOIN host_commands c ON c.id=r.command_id
          WHERE c.placement_id=? AND c.kind=? AND r.disposition='applied'
          ORDER BY r.received_at DESC LIMIT 1",
    )
    .bind(placement.to_string())
    .bind(kind)
    .fetch_optional(pool)
    .await?
    else {
        return Ok(None);
    };
    let result: AgentCommandResult =
        serde_json::from_str(&json).map_err(|_| FleetError::Invariant("stored result"))?;
    Ok(match result.outcome {
        CommandOutcome::Failed {
            diagnostic: Some(diagnostic),
            ..
        } => Some(diagnostic.code),
        _ => None,
    })
}

/// State of the newest command of `kind` for this placement, if any.
pub async fn command_state(
    pool: &SqlitePool,
    placement: PlacementId,
    kind: &str,
) -> Result<Option<(String, i64)>, FleetError> {
    Ok(sqlx::query_as(
        "SELECT state,issued_at FROM host_commands WHERE placement_id=? AND kind=?
          ORDER BY cursor DESC LIMIT 1",
    )
    .bind(placement.to_string())
    .bind(kind)
    .fetch_optional(pool)
    .await?)
}

/// Issues a command for a placement, sealed to its host's current session.
/// Returns `None` when the host has no live work authority, or when a command
/// with this idempotency key already exists.
pub async fn issue_command(
    pool: &SqlitePool,
    placement: PlacementId,
    operation: AgentOperation,
    idempotency_key: &str,
    ttl_ms: i64,
) -> Result<Option<CommandId>, FleetError> {
    let mut tx = pool.begin_with("BEGIN IMMEDIATE").await?;
    let now = crate::now_millis();
    let Some(row) = sqlx::query(
        "SELECT p.intent_id,p.host_id,p.backend_id,p.generation,p.profile_revision,h.epoch,
                h.agent_session_id,h.authority_incarnation
           FROM workload_placements p JOIN fleet_hosts h ON h.id=p.host_id
          WHERE p.id=? AND p.host_epoch=h.epoch AND h.authority_expires_at>?
            AND h.agent_session_id IS NOT NULL AND h.authority_incarnation IS NOT NULL",
    )
    .bind(placement.to_string())
    .bind(now)
    .fetch_optional(&mut *tx)
    .await?
    else {
        return Ok(None);
    };
    let command_id =
        CommandId::try_new(Uuid::new_v4()).map_err(|_| FleetError::Invariant("command ID"))?;
    let deadline = now + ttl_ms;
    let host_id: HostId = parse(row.get("host_id"))?;
    let backend_id: BackendId = parse(row.get("backend_id"))?;
    let generation = Generation::new(counter(row.get("generation"))?)
        .map_err(|_| FleetError::Invariant("generation"))?;
    let envelope = AgentCommandEnvelope {
        protocol_version: ProtocolVersion::new(1)
            .map_err(|_| FleetError::Invariant("protocol version"))?,
        command_id,
        operation_id: parse::<OperationId>(row.get("intent_id"))?,
        idempotency_key: IdempotencyKey::parse(idempotency_key.to_owned())
            .map_err(|_| FleetError::InvalidRequest("idempotency key"))?,
        host_id,
        host_epoch: HostEpoch::new(counter(row.get("epoch"))?)
            .map_err(|_| FleetError::Invariant("epoch"))?,
        control_plane_incarnation: parse::<ControlPlaneIncarnation>(
            row.get("authority_incarnation"),
        )?,
        authority_session_id: parse::<AuthoritySessionId>(row.get("agent_session_id"))?,
        scope: CommandScope::Placement {
            placement_id: placement,
            backend_id,
            generation,
        },
        expected_revision: Revision::new(counter(row.get("profile_revision"))?)
            .map_err(|_| FleetError::Invariant("revision"))?,
        issued_at: timestamp(now - crate::fleet::protocol::agent::COMMAND_CLOCK_SKEW_MILLIS)?,
        deadline_at: timestamp(deadline)?,
        request_hash: String::new(),
        operation,
    }
    .seal_hash()
    .map_err(|_| FleetError::InvalidRequest("command shape"))?;
    let inserted = sqlx::query(
        "INSERT INTO host_commands
         (id,operation_id,scope,placement_id,host_id,backend_id,host_epoch,generation,kind,
          protocol_version,request_hash,payload_json,expected_revision,issued_at,deadline_at,idempotency_key)
         VALUES (?,?,'placement',?,?,?,?,?,?,1,?,?,?,?,?,?)
         ON CONFLICT(host_id,idempotency_key) DO NOTHING",
    )
    .bind(command_id.to_string())
    .bind(envelope.operation_id.to_string())
    .bind(placement.to_string())
    .bind(host_id.to_string())
    .bind(backend_id.to_string())
    .bind(row.get::<i64, _>("epoch"))
    .bind(row.get::<i64, _>("generation"))
    .bind(envelope.operation.database_kind())
    .bind(&envelope.request_hash)
    .bind(serde_json::to_string(&envelope).map_err(|_| FleetError::Invariant("envelope"))?)
    .bind(row.get::<i64, _>("profile_revision"))
    .bind(now)
    .bind(deadline)
    .bind(idempotency_key)
    .execute(&mut *tx)
    .await?
    .rows_affected();
    tx.commit().await?;
    Ok((inserted == 1).then_some(command_id))
}

/// Seals one-time runner registration material for this placement's host.
pub async fn store_bootstrap(
    pool: &SqlitePool,
    vault: &Vault,
    placement: PlacementId,
    material: &str,
    ttl_ms: i64,
) -> Result<BootstrapReference, FleetError> {
    let reference = BootstrapReference::try_new(Uuid::new_v4())
        .map_err(|_| FleetError::Invariant("bootstrap reference"))?;
    let sealed = vault
        .seal(material)
        .map_err(|_| FleetError::Invariant("bootstrap seal"))?;
    let now = crate::now_millis();
    sqlx::query(
        "INSERT INTO fleet_bootstrap_material (reference,placement_id,host_id,sealed,created_at,expires_at)
         SELECT ?,id,host_id,?,?,? FROM workload_placements WHERE id=?",
    )
    .bind(reference.to_string())
    .bind(sealed)
    .bind(now)
    .bind(now + ttl_ms)
    .bind(placement.to_string())
    .execute(pool)
    .await?;
    Ok(reference)
}

/// Links the runner row and marks the placement as starting.
pub async fn mark_starting(
    pool: &SqlitePool,
    placement: PlacementId,
    runner_id: &str,
) -> Result<(), FleetError> {
    sqlx::query(
        "UPDATE workload_placements SET state='starting',runner_id=?,updated_at=? WHERE id=?",
    )
    .bind(runner_id)
    .bind(crate::now_millis())
    .bind(placement.to_string())
    .execute(pool)
    .await?;
    Ok(())
}

/// Ends a placement that was never told to start and frees its capacity.
/// With no start command ever issued, no runner of it can exist on the host,
/// which is the proof releasing needs. `cause` is recorded when the placement
/// first fails. Returns whether anything changed.
pub async fn abandon_unstarted(
    pool: &SqlitePool,
    placement: PlacementId,
    cause: &PlacementFailure,
) -> Result<bool, FleetError> {
    let mut tx = pool.begin_with("BEGIN IMMEDIATE").await?;
    let now = crate::now_millis();
    let id = placement.to_string();
    let started: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM host_commands WHERE placement_id=? AND kind='start_runner')",
    )
    .bind(&id)
    .fetch_one(&mut *tx)
    .await?;
    if started {
        return Ok(false);
    }
    // Recorded once, when the placement first fails; a failed placement that
    // is finished again keeps the cause it already has.
    fail_placement(
        &mut tx,
        &id,
        Some(&["preparing", "registering"]),
        cause,
        None,
        now,
    )
    .await?;
    sqlx::query(
        "UPDATE workload_placements SET state='failed',
           local_absence_proven_at=COALESCE(local_absence_proven_at,?),
           delayed_starts_excluded_at=COALESCE(delayed_starts_excluded_at,?),updated_at=?
         WHERE id=? AND state IN ('preparing','registering','failed')",
    )
    .bind(now)
    .bind(now)
    .bind(now)
    .bind(&id)
    .execute(&mut *tx)
    .await?;
    let released = sqlx::query(
        "UPDATE capacity_allocations SET state='released',released_at=?,updated_at=?
          WHERE placement_id=? AND state!='released'",
    )
    .bind(now)
    .bind(now)
    .bind(&id)
    .execute(&mut *tx)
    .await?
    .rows_affected();
    tx.commit().await?;
    Ok(released > 0)
}

/// Moves a placement to `state` when no command result already did.
pub async fn set_state(
    pool: &SqlitePool,
    placement: PlacementId,
    from: &str,
    to: &str,
) -> Result<bool, FleetError> {
    Ok(
        sqlx::query("UPDATE workload_placements SET state=?,updated_at=? WHERE id=? AND state=?")
            .bind(to)
            .bind(crate::now_millis())
            .bind(placement.to_string())
            .bind(from)
            .execute(pool)
            .await?
            .rows_affected()
            == 1,
    )
}

fn parse<T: std::str::FromStr>(value: String) -> Result<T, FleetError> {
    value
        .parse()
        .map_err(|_| FleetError::InvalidIdentifier(value))
}

fn counter(value: i64) -> Result<u64, FleetError> {
    u64::try_from(value).map_err(|_| FleetError::Invariant("counter"))
}

fn timestamp(value: i64) -> Result<WireTimestamp, FleetError> {
    WireTimestamp::from_millis(value).map_err(|_| FleetError::Invariant("timestamp"))
}
