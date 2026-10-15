//! Command delivery to agents and result intake.
//!
//! Commands are sealed to the session that held work authority when they were
//! issued and can't be rewritten. Delivery therefore only hands out commands
//! for the agent's current session. A command that never left the outbox and
//! no longer can (its session ended or its deadline passed) is cancelled here,
//! which is also proof that nothing it would have started can exist, so its
//! placement's capacity is released.
//!
//! Results are checked against the stored envelope, recorded once by hash, and
//! move the placement forward. A result for a command that already finished is
//! kept as `stale` and changes nothing.

use serde_json::json;
use sqlx::{Row as _, Sqlite, Transaction};

use super::{AuthenticatedHost, RegistryError, RegistryService, audit_and_event};
use crate::fleet::{
    ids::{AuthoritySessionId, BootstrapReference, HostId},
    protocol::{
        agent::AgentCommandEnvelope,
        outcomes::{AgentCommandResult, CommandOutcome, SuccessEvidence},
    },
};

/// A delivered command is offered again if no result arrived within this long.
const REDELIVERY_MILLIS: i64 = 60_000;
const MAX_ATTEMPTS: i64 = 10;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResultDisposition {
    Applied,
    Stale,
}

impl RegistryService {
    /// Commands the agent should run now, oldest first. Returns nothing when
    /// the session has no current work authority.
    pub async fn poll_commands(
        &self,
        authenticated: &AuthenticatedHost,
        session_id: AuthoritySessionId,
        limit: u32,
    ) -> Result<Vec<AgentCommandEnvelope>, RegistryError> {
        let now = self.clock.now_millis();
        let host_id = authenticated.host_id.to_string();
        // Agents poll every half second. A host with no open commands has
        // nothing to cancel or deliver, so that poll only reads and never
        // queues behind SQLite's write lock.
        let mut read = self.pool.begin().await?;
        super::ensure_authenticated_generation(&mut read, authenticated, now).await?;
        let open: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM host_commands WHERE host_id=? AND state IN ('pending','delivered'))",
        )
        .bind(&host_id)
        .fetch_one(&mut *read)
        .await?;
        read.commit().await?;
        if !open {
            return Ok(Vec::new());
        }
        let mut tx = self.begin().await?;
        super::ensure_authenticated_generation(&mut tx, authenticated, now).await?;
        cancel_undeliverable(&mut tx, &host_id, now).await?;
        let authorized: bool = sqlx::query_scalar(
            "SELECT agent_session_id=? AND authority_expires_at>? FROM fleet_hosts WHERE id=?",
        )
        .bind(session_id.to_string())
        .bind(now)
        .bind(&host_id)
        .fetch_optional(&mut *tx)
        .await?
        .flatten()
        .unwrap_or(false);
        if !authorized {
            tx.commit().await?;
            return Ok(Vec::new());
        }
        let rows = sqlx::query(
            "SELECT c.id,c.payload_json,c.attempt_count FROM host_commands c
              WHERE c.host_id=? AND c.deadline_at>?
                AND json_extract(c.payload_json,'$.authority_session_id')=?
                AND (c.state='pending' OR (c.state='delivered' AND c.attempt_count<? AND NOT EXISTS (
                     SELECT 1 FROM host_command_attempts a WHERE a.command_id=c.id AND a.recorded_at>?)))
              ORDER BY c.cursor LIMIT ?",
        )
        .bind(&host_id)
        .bind(now)
        .bind(session_id.to_string())
        .bind(MAX_ATTEMPTS)
        .bind(now - REDELIVERY_MILLIS)
        .bind(i64::from(limit))
        .fetch_all(&mut *tx)
        .await?;
        let mut commands = Vec::with_capacity(rows.len());
        for row in rows {
            let id: String = row.get("id");
            let attempt = row.get::<i64, _>("attempt_count") + 1;
            sqlx::query("UPDATE host_commands SET state='delivered',attempt_count=? WHERE id=?")
                .bind(attempt)
                .bind(&id)
                .execute(&mut *tx)
                .await?;
            sqlx::query(
                "INSERT INTO host_command_attempts (command_id,attempt,state,recorded_at)
                 VALUES (?,?,'delivered',?)",
            )
            .bind(&id)
            .bind(attempt)
            .bind(now)
            .execute(&mut *tx)
            .await?;
            commands.push(serde_json::from_str(row.get("payload_json"))?);
        }
        tx.commit().await?;
        Ok(commands)
    }

    /// Returns the sealed registration material for a start command on this
    /// host. It stays readable until the start resolves or the material
    /// expires, so a retried fetch gets the same material and never mints
    /// another upstream registration.
    pub async fn claim_bootstrap(
        &self,
        authenticated: &AuthenticatedHost,
        reference: BootstrapReference,
    ) -> Result<String, RegistryError> {
        let mut tx = self.begin().await?;
        let now = self.clock.now_millis();
        super::ensure_authenticated_generation(&mut tx, authenticated, now).await?;
        let sealed: Option<String> = sqlx::query_scalar(
            "SELECT sealed FROM fleet_bootstrap_material
              WHERE reference=? AND host_id=? AND sealed IS NOT NULL AND expires_at>?",
        )
        .bind(reference.to_string())
        .bind(authenticated.host_id.to_string())
        .bind(now)
        .fetch_optional(&mut *tx)
        .await?
        .flatten();
        let Some(sealed) = sealed else {
            return Err(RegistryError::NotFound);
        };
        sqlx::query(
            "UPDATE fleet_bootstrap_material SET claimed_at=COALESCE(claimed_at,?) WHERE reference=?",
        )
        .bind(now)
        .bind(reference.to_string())
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        self.vault
            .open(&sealed)
            .map_err(|_| RegistryError::InvalidRequest("bootstrap material is unreadable"))
    }

    pub async fn record_command_result(
        &self,
        authenticated: &AuthenticatedHost,
        result: &AgentCommandResult,
    ) -> Result<ResultDisposition, RegistryError> {
        let mut tx = self.begin().await?;
        let now = self.clock.now_millis();
        super::ensure_authenticated_generation(&mut tx, authenticated, now).await?;
        let Some(row) = sqlx::query(
            "SELECT id,kind,state,placement_id,payload_json,attempt_count FROM host_commands
              WHERE id=? AND host_id=?",
        )
        .bind(result.command_id.to_string())
        .bind(authenticated.host_id.to_string())
        .fetch_optional(&mut *tx)
        .await?
        else {
            return Err(RegistryError::NotFound);
        };
        let envelope: AgentCommandEnvelope = serde_json::from_str(row.get("payload_json"))?;
        result
            .validate_against(&envelope)
            .map_err(|_| RegistryError::InvalidRequest("result does not match its command"))?;
        let encoded = serde_json::to_string(result)?;
        let result_hash = crate::crypto::hash_token(&encoded);
        if let Some(disposition) = sqlx::query_scalar::<_, String>(
            "SELECT disposition FROM host_command_results WHERE command_id=? AND result_hash=?",
        )
        .bind(result.command_id.to_string())
        .bind(&result_hash)
        .fetch_optional(&mut *tx)
        .await?
        {
            tx.commit().await?;
            return Ok(if disposition == "applied" {
                ResultDisposition::Applied
            } else {
                ResultDisposition::Stale
            });
        }
        let state: String = row.get("state");
        // A command that went uncertain because its deadline passed without
        // a result still waits for one: the agent may have run it and lost
        // the reply. Its late result settles the command.
        let late = state == "uncertain"
            && !sqlx::query_scalar::<_, bool>(
                "SELECT EXISTS (SELECT 1 FROM host_command_results WHERE command_id=? AND disposition='applied')",
            )
            .bind(result.command_id.to_string())
            .fetch_one(&mut *tx)
            .await?;
        let terminal = !late
            && matches!(
                state.as_str(),
                "succeeded" | "failed" | "uncertain" | "cancelled"
            );
        let disposition = if terminal {
            ResultDisposition::Stale
        } else {
            ResultDisposition::Applied
        };
        sqlx::query(
            "INSERT INTO host_command_results (id,command_id,result_hash,disposition,result_json,received_at)
             VALUES (?,?,?,?,?,?)",
        )
        .bind(result.result_id.to_string())
        .bind(result.command_id.to_string())
        .bind(&result_hash)
        .bind(if terminal { "stale" } else { "applied" })
        .bind(&encoded)
        .bind(now)
        .execute(&mut *tx)
        .await?;
        if !terminal {
            let next = match &result.outcome {
                CommandOutcome::Accepted { .. } => "accepted",
                CommandOutcome::Running { .. } => "running",
                CommandOutcome::Succeeded { .. } => "succeeded",
                CommandOutcome::Failed { .. } => "failed",
                CommandOutcome::Uncertain { .. } => "uncertain",
            };
            sqlx::query("UPDATE host_commands SET state=? WHERE id=?")
                .bind(next)
                .bind(result.command_id.to_string())
                .execute(&mut *tx)
                .await?;
            let attempt = row.get::<i64, _>("attempt_count").max(1);
            {
                sqlx::query(
                    "INSERT INTO host_command_attempts (command_id,attempt,state,recorded_at)
                     VALUES (?,?,?,?) ON CONFLICT(command_id,attempt) DO UPDATE SET state=excluded.state,recorded_at=excluded.recorded_at",
                )
                .bind(result.command_id.to_string())
                .bind(attempt)
                .bind(next)
                .bind(now)
                .execute(&mut *tx)
                .await?;
            }
            if let Some(placement_id) = row.get::<Option<String>, _>("placement_id") {
                if !late {
                    apply_to_placement(&mut tx, &placement_id, row.get("kind"), result, now)
                        .await?;
                } else if placement_is_uncertain(&mut tx, &placement_id).await? {
                    // Only a placement still waiting on this answer moves.
                    apply_to_placement(&mut tx, &placement_id, row.get("kind"), result, now)
                        .await?;
                    keep_uncertain_while_unsettled(&mut tx, &placement_id, now).await?;
                }
            }
            audit_and_event(
                &mut tx,
                None,
                Some(authenticated.host_id),
                "fleet.command.result",
                "command",
                &result.command_id.to_string(),
                json!({"kind": row.get::<String, _>("kind"), "state": next}),
                now,
            )
            .await?;
        }
        tx.commit().await?;
        Ok(disposition)
    }
}

/// Moves a placement forward from a command outcome.
async fn apply_to_placement(
    tx: &mut Transaction<'_, Sqlite>,
    placement_id: &str,
    kind: &str,
    result: &AgentCommandResult,
    now: i64,
) -> Result<(), RegistryError> {
    if kind == "start_runner"
        && !matches!(
            result.outcome,
            CommandOutcome::Accepted { .. } | CommandOutcome::Running { .. }
        )
    {
        erase_bootstrap(tx, placement_id, now).await?;
    }
    let next = match (&result.outcome, kind) {
        (CommandOutcome::Accepted { .. } | CommandOutcome::Running { .. }, _) => return Ok(()),
        (CommandOutcome::Succeeded { .. }, "prepare_environment") => "registering",
        (CommandOutcome::Succeeded { .. }, "start_runner") => {
            sqlx::query(
                "UPDATE capacity_allocations SET state='committed',updated_at=?
                  WHERE placement_id=? AND state='reserved'",
            )
            .bind(now)
            .bind(placement_id)
            .execute(&mut **tx)
            .await?;
            "running"
        }
        (CommandOutcome::Succeeded { .. }, "stop") => "stopped",
        (CommandOutcome::Succeeded { evidence }, "cleanup") => {
            if matches!(evidence.as_ref(), SuccessEvidence::Cleaned { .. }) {
                release(tx, placement_id, now).await?;
                "cleaned"
            } else {
                return Ok(());
            }
        }
        (CommandOutcome::Succeeded { .. }, _) => return Ok(()),
        // A prepare or start that failed outright never created a runtime
        // object: the agent journals ownership only on success.
        (CommandOutcome::Failed { .. }, "prepare_environment" | "start_runner") => {
            release(tx, placement_id, now).await?;
            "failed"
        }
        (CommandOutcome::Failed { .. }, _) => return Ok(()),
        (CommandOutcome::Uncertain { .. }, _) => "uncertain",
    };
    sqlx::query("UPDATE workload_placements SET state=?,updated_at=? WHERE id=?")
        .bind(next)
        .bind(now)
        .bind(placement_id)
        .execute(&mut **tx)
        .await?;
    Ok(())
}

/// Registration material is single-use: once a start resolves it is erased.
pub(super) async fn erase_bootstrap(
    tx: &mut Transaction<'_, Sqlite>,
    placement_id: &str,
    now: i64,
) -> Result<(), RegistryError> {
    sqlx::query(
        "UPDATE fleet_bootstrap_material SET sealed=NULL,erased_at=?
          WHERE placement_id=? AND erased_at IS NULL",
    )
    .bind(now)
    .bind(placement_id)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// Records that nothing of this placement can exist on the host and frees
/// its capacity.
pub(super) async fn release(
    tx: &mut Transaction<'_, Sqlite>,
    placement_id: &str,
    now: i64,
) -> Result<(), RegistryError> {
    sqlx::query(
        "UPDATE workload_placements SET
           local_absence_proven_at=COALESCE(local_absence_proven_at,?),
           delayed_starts_excluded_at=COALESCE(delayed_starts_excluded_at,?),updated_at=?
         WHERE id=?",
    )
    .bind(now)
    .bind(now)
    .bind(now)
    .bind(placement_id)
    .execute(&mut **tx)
    .await?;
    sqlx::query(
        "UPDATE capacity_allocations SET state='released',released_at=?,updated_at=?
          WHERE placement_id=? AND state!='released'",
    )
    .bind(now)
    .bind(now)
    .bind(placement_id)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

async fn placement_is_uncertain(
    tx: &mut Transaction<'_, Sqlite>,
    placement_id: &str,
) -> Result<bool, RegistryError> {
    Ok(sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS (SELECT 1 FROM workload_placements WHERE id=? AND state='uncertain')",
    )
    .bind(placement_id)
    .fetch_one(&mut **tx)
    .await?)
}

/// A placement stays uncertain while any other command that could have
/// changed its workload is still unanswered.
async fn keep_uncertain_while_unsettled(
    tx: &mut Transaction<'_, Sqlite>,
    placement_id: &str,
    now: i64,
) -> Result<(), RegistryError> {
    sqlx::query(
        "UPDATE workload_placements SET state='uncertain',updated_at=?
          WHERE id=? AND state!='uncertain' AND EXISTS (
            SELECT 1 FROM host_commands c WHERE c.placement_id=workload_placements.id
              AND c.state='uncertain' AND c.kind!='inspect')",
    )
    .bind(now)
    .bind(placement_id)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// Settles commands that a reconnecting agent never received.
///
/// The agent journals a command before acting on it, and before this
/// handshake it delivered the result of everything it journaled. A command
/// delivered to one of its earlier sessions that still has no result was
/// therefore lost on the way (a poll reply that never arrived), and it never
/// will run: the agent refuses envelopes bound to another session. Such a
/// prepare or start created nothing, so its capacity is released and the
/// placement fails; any other command leaves the workload as it was, so an
/// uncertain placement goes back to `running` for the reconciler to inspect.
pub(super) async fn settle_unreceived(
    tx: &mut Transaction<'_, Sqlite>,
    host_id: HostId,
    epoch: i64,
    session_id: &str,
    now: i64,
) -> Result<(), RegistryError> {
    let rows = sqlx::query(
        "SELECT c.id,c.kind,c.placement_id FROM host_commands c
          WHERE c.host_id=? AND c.host_epoch=? AND c.state IN ('delivered','uncertain')
            AND json_extract(c.payload_json,'$.authority_session_id') IS NOT ?
            AND NOT EXISTS (SELECT 1 FROM host_command_results r WHERE r.command_id=c.id)",
    )
    .bind(host_id.to_string())
    .bind(epoch)
    .bind(session_id)
    .fetch_all(&mut **tx)
    .await?;
    let mut touched = Vec::new();
    for row in rows {
        let id: String = row.get("id");
        let kind: String = row.get("kind");
        sqlx::query("UPDATE host_commands SET state='cancelled' WHERE id=?")
            .bind(&id)
            .execute(&mut **tx)
            .await?;
        audit_and_event(
            tx,
            None,
            Some(host_id),
            "fleet.command.unreceived",
            "command",
            &id,
            json!({"kind": kind}),
            now,
        )
        .await?;
        let Some(placement_id) = row.get::<Option<String>, _>("placement_id") else {
            continue;
        };
        if kind == "prepare_environment" || kind == "start_runner" {
            erase_bootstrap(tx, &placement_id, now).await?;
            release(tx, &placement_id, now).await?;
            sqlx::query(
                "UPDATE workload_placements SET state='failed',updated_at=?
                  WHERE id=? AND state IN ('preparing','registering','starting','uncertain')",
            )
            .bind(now)
            .bind(&placement_id)
            .execute(&mut **tx)
            .await?;
        } else if kind != "inspect" {
            // An unanswered inspect never made its placement uncertain.
            touched.push(placement_id);
        }
    }
    for placement_id in touched {
        sqlx::query(
            "UPDATE workload_placements SET state='running',updated_at=?
              WHERE id=? AND state='uncertain'",
        )
        .bind(now)
        .bind(&placement_id)
        .execute(&mut **tx)
        .await?;
        keep_uncertain_while_unsettled(tx, &placement_id, now).await?;
    }
    Ok(())
}

/// Cancels pending commands that can no longer be delivered: their session
/// ended or their deadline passed. A delivered command past its deadline is
/// uncertain, because the agent may have acted on it. Only commands that can
/// change a workload make its placement uncertain; an unanswered `inspect`
/// leaves the placement as it was, and a later inspect checks again.
async fn cancel_undeliverable(
    tx: &mut Transaction<'_, Sqlite>,
    host_id: &str,
    now: i64,
) -> Result<(), RegistryError> {
    let rows = sqlx::query(
        "SELECT c.id,c.kind,c.state,c.placement_id FROM host_commands c JOIN fleet_hosts h ON h.id=c.host_id
          WHERE c.host_id=? AND c.state IN ('pending','delivered') AND (
            c.deadline_at<=? OR (c.state='pending' AND (
              h.agent_session_id IS NULL
              OR json_extract(c.payload_json,'$.authority_session_id') IS NOT h.agent_session_id
              OR c.host_epoch IS NOT h.epoch)))",
    )
    .bind(host_id)
    .bind(now)
    .fetch_all(&mut **tx)
    .await?;
    for row in rows {
        let id: String = row.get("id");
        let delivered = row.get::<String, _>("state") == "delivered";
        sqlx::query("UPDATE host_commands SET state=? WHERE id=?")
            .bind(if delivered { "uncertain" } else { "cancelled" })
            .bind(&id)
            .execute(&mut **tx)
            .await?;
        let Some(placement_id) = row.get::<Option<String>, _>("placement_id") else {
            continue;
        };
        let kind: String = row.get("kind");
        if delivered && kind == "inspect" {
            continue;
        }
        if delivered {
            sqlx::query("UPDATE workload_placements SET state='uncertain',updated_at=? WHERE id=?")
                .bind(now)
                .bind(&placement_id)
                .execute(&mut **tx)
                .await?;
        } else if kind == "prepare_environment" || kind == "start_runner" {
            erase_bootstrap(tx, &placement_id, now).await?;
            release(tx, &placement_id, now).await?;
            sqlx::query("UPDATE workload_placements SET state='failed',updated_at=? WHERE id=?")
                .bind(now)
                .bind(&placement_id)
                .execute(&mut **tx)
                .await?;
        }
    }
    Ok(())
}
