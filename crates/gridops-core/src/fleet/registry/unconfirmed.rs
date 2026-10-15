//! Placements nobody can confirm, and the administrator's repair.
//!
//! A placement is `uncertain` when a command that could have changed its
//! workload went unanswered: its deadline passed, or the agent stopped part
//! way through it. Until it settles the host gets no work authority. Agents
//! settle most of these themselves when they reconnect (see
//! `settle_unreceived`); what's left needs a decision, which this repair
//! records. It takes the same transitions as that automatic settlement:
//! a placement whose runner the agent confirmed goes back to `running` so the
//! reconciler inspects it and cleans it up with the agent's proof; one whose
//! start was never confirmed releases its capacity and fails, on the
//! administrator's word instead of the agent's journal.

use serde::Serialize;
use serde_json::json;
use sqlx::Row as _;

use super::{
    RegistryError, RegistryService, audit_and_event,
    commands::{erase_bootstrap, release},
    ensure_system_admin, require_user,
};
use crate::fleet::{
    Principal,
    ids::{HostId, PlacementId},
};

/// What repairing a placement does.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Repair {
    /// The agent confirmed the runner started: it goes back to `running`, and
    /// the reconciler inspects and cleans it up with the agent's proof.
    Reinspect,
    /// The start was never confirmed: its capacity is released and it fails.
    Release,
}

/// The command a placement is waiting on.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UnansweredCommand {
    pub kind: String,
    pub issued_at: i64,
    /// The agent reported it stopped part way through the command, so even
    /// it doesn't know the outcome. Otherwise no answer arrived at all.
    pub interrupted: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UnconfirmedPlacement {
    pub placement_id: String,
    pub runner_id: Option<String>,
    pub runner_name: Option<String>,
    pub pool_id: String,
    pub pool_name: Option<String>,
    /// When the placement became unconfirmed.
    pub since: i64,
    pub command: Option<UnansweredCommand>,
    pub repair: Repair,
}

/// The agent confirmed this placement's runner started, in the host's current
/// epoch, so a later command can still reach it.
const STARTED: &str = "p.host_epoch=h.epoch AND EXISTS (SELECT 1 FROM host_commands s
    WHERE s.placement_id=p.id AND s.kind='start_runner' AND s.state='succeeded')";

impl RegistryService {
    /// The host's unconfirmed placements, oldest first.
    pub async fn unconfirmed_placements(
        &self,
        principal: &Principal,
        host_id: HostId,
    ) -> Result<Vec<UnconfirmedPlacement>, RegistryError> {
        let user_id = require_user(principal).map_err(|_| RegistryError::Forbidden)?;
        let mut tx = self.pool.begin().await?;
        ensure_system_admin(&mut tx, user_id).await?;
        let exists: bool =
            sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM fleet_hosts WHERE id=?)")
                .bind(host_id.to_string())
                .fetch_one(&mut *tx)
                .await?;
        if !exists {
            return Err(RegistryError::NotFound);
        }
        let rows = sqlx::query(sqlx::AssertSqlSafe(format!(
            "SELECT p.id,p.updated_at,p.runner_id,r.name AS runner_name,i.pool_id,pool.name AS pool_name,
                    {STARTED} AS started,
                    c.kind,c.issued_at,
                    EXISTS (SELECT 1 FROM host_command_results res
                             WHERE res.command_id=c.id AND res.disposition='applied') AS interrupted
               FROM workload_placements p
               JOIN fleet_hosts h ON h.id=p.host_id
               JOIN workload_intents i ON i.id=p.intent_id
               LEFT JOIN runners r ON r.id=p.runner_id
               LEFT JOIN runner_pools pool ON pool.id=i.pool_id
               LEFT JOIN host_commands c ON c.id=(
                 SELECT u.id FROM host_commands u WHERE u.placement_id=p.id
                    AND u.state='uncertain' AND u.kind!='inspect' ORDER BY u.cursor DESC LIMIT 1)
              WHERE p.host_id=? AND p.state='uncertain'
              ORDER BY p.updated_at,p.id"
        )))
        .bind(host_id.to_string())
        .fetch_all(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(rows
            .into_iter()
            .map(|row| UnconfirmedPlacement {
                placement_id: row.get("id"),
                runner_id: row.get("runner_id"),
                runner_name: row.get("runner_name"),
                pool_id: row.get("pool_id"),
                pool_name: row.get("pool_name"),
                since: row.get("updated_at"),
                command: row
                    .get::<Option<String>, _>("kind")
                    .map(|kind| UnansweredCommand {
                        kind,
                        issued_at: row.get("issued_at"),
                        interrupted: row.get("interrupted"),
                    }),
                repair: if row.get("started") {
                    Repair::Reinspect
                } else {
                    Repair::Release
                },
            })
            .collect())
    }

    /// Settles the host's unconfirmed placements in `placements` and returns
    /// the ones it repaired. Placements that settled meanwhile, or aren't
    /// this host's, are left alone.
    pub async fn repair_unconfirmed(
        &self,
        principal: &Principal,
        host_id: HostId,
        placements: &[PlacementId],
    ) -> Result<Vec<PlacementId>, RegistryError> {
        let user_id = require_user(principal).map_err(|_| RegistryError::Forbidden)?;
        let mut tx = self.begin().await?;
        let now = self.clock.now_millis();
        ensure_system_admin(&mut tx, user_id).await?;
        let mut repaired = Vec::new();
        for placement in placements {
            let id = placement.to_string();
            let started: Option<bool> = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
                "SELECT {STARTED} FROM workload_placements p JOIN fleet_hosts h ON h.id=p.host_id
                  WHERE p.id=? AND p.host_id=? AND p.state='uncertain'"
            )))
            .bind(&id)
            .bind(host_id.to_string())
            .fetch_optional(&mut *tx)
            .await?;
            let Some(started) = started else {
                continue;
            };
            // Whatever was still open for it will never be answered now.
            let cancelled: Vec<String> = sqlx::query_scalar(
                "UPDATE host_commands SET state='cancelled'
                  WHERE placement_id=? AND state IN ('pending','delivered','uncertain') RETURNING kind",
            )
            .bind(&id)
            .fetch_all(&mut *tx)
            .await?;
            let repair = if started {
                Repair::Reinspect
            } else {
                erase_bootstrap(&mut tx, &id, now).await?;
                release(&mut tx, &id, now).await?;
                Repair::Release
            };
            sqlx::query("UPDATE workload_placements SET state=?,updated_at=? WHERE id=?")
                .bind(if started { "running" } else { "failed" })
                .bind(now)
                .bind(&id)
                .execute(&mut *tx)
                .await?;
            audit_and_event(
                &mut tx,
                Some(user_id),
                Some(host_id),
                "fleet.placement.repaired",
                "placement",
                &id,
                json!({"repair": repair, "cancelled": cancelled}),
                now,
            )
            .await?;
            repaired.push(*placement);
        }
        tx.commit().await?;
        Ok(repaired)
    }
}
