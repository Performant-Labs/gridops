//! Deleting a machine that can never take work again.
//!
//! Deleting is soft. Placements, commands, events, samples and grants all
//! reference the host, and the machine page, the runs list and the audit trail
//! must keep rendering them, so the row stays and `deleted_at` hides it from
//! the machine list, the counts and capacity. The machine is also left retired
//! and revoked, so every credential, recovery and approval check that already
//! refuses a retired or revoked host refuses a deleted one too, and an agent
//! still talking as it is turned away instead of recreating anything.
//! Re-enrolling the same computer creates a new, clean record.
//!
//! The registry refuses unless nothing could still be using the machine, and
//! names every reason, so one refusal says all there is to clean up.

use serde_json::json;
use sqlx::Row as _;

use super::{
    RegistryError, RegistryService, audit_and_event, ensure_system_admin, require_user,
    revoke_open_rotations,
};
use crate::fleet::{HostId, Principal, protocol::browser::HostDeleteBlocker};

/// A machine that reported in more recently than this may still be running.
/// It is the "offline" threshold the machine page uses.
const RECENTLY_SEEN_MILLIS: i64 = 90_000;

/// What a delete removed from view.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeletedHost {
    pub host_id: HostId,
    pub name: String,
}

impl RegistryService {
    /// Deletes a machine that is retired, or that never finished enrolling and
    /// has gone quiet, once nothing live refers to it.
    ///
    /// Refused with [`RegistryError::HostNotDeletable`] while any of these
    /// holds: the machine is approved but not retired; it reported in within
    /// the last 90 seconds (unless retired, whose credential is already
    /// revoked); it has a placement that is not cleaned up or failed, an
    /// unconfirmed start, capacity still charged, a runner that has not
    /// finished, a command still waiting within its deadline, or (approved
    /// only) an unrevoked credential. A machine that was never approved has
    /// its credential and open recovery codes revoked by the delete itself.
    ///
    /// What is kept: the host row, resource domains and budget, placements,
    /// intents, commands, results, events, samples, inventory and agent logs,
    /// and consumed enrollments, so history renders and nothing dangles. What
    /// is removed from use: credentials, open rotations and recovery codes,
    /// target and user grants, and backend enablement. Profile host selectors
    /// are left alone: dropping the last one would widen a profile that
    /// otherwise selects "all authorized hosts".
    pub async fn delete_host(
        &self,
        principal: &Principal,
        host_id: HostId,
    ) -> Result<DeletedHost, RegistryError> {
        let user_id = require_user(principal).map_err(|_| RegistryError::Forbidden)?;
        let mut tx = self.begin().await?;
        let now = self.clock.now_millis();
        ensure_system_admin(&mut tx, user_id).await?;
        let id = host_id.to_string();
        let Some(host) = sqlx::query(
            "SELECT name,enrollment_state,lifecycle_state,last_heartbeat_at,last_inventory_at
               FROM fleet_hosts WHERE id=? AND deleted_at IS NULL",
        )
        .bind(&id)
        .fetch_optional(&mut *tx)
        .await?
        else {
            return Err(RegistryError::NotFound);
        };
        let name: String = host.get("name");
        let enrollment: String = host.get("enrollment_state");
        let lifecycle: String = host.get("lifecycle_state");
        let last_heartbeat_at: Option<i64> = host.get("last_heartbeat_at");
        let last_inventory_at: Option<i64> = host.get("last_inventory_at");
        let approved = enrollment == "approved";
        let retired = lifecycle == "retired";

        let counts = sqlx::query(
            "SELECT
               (SELECT COUNT(*) FROM workload_placements
                 WHERE host_id=?1 AND state NOT IN ('cleaned','failed','uncertain')) AS placements,
               (SELECT COUNT(*) FROM workload_placements
                 WHERE host_id=?1 AND state='uncertain') AS unconfirmed,
               (SELECT COUNT(*) FROM capacity_allocations
                 WHERE host_id=?1 AND state!='released') AS charged,
               (SELECT COUNT(*) FROM runners r WHERE r.status NOT IN ('deleted','failed')
                  AND EXISTS (SELECT 1 FROM workload_placements p
                               WHERE p.host_id=?1 AND (p.runner_id=r.id OR p.workload_id=r.id))) AS runners,
               (SELECT COUNT(*) FROM host_commands
                 WHERE host_id=?1 AND state IN ('pending','delivered','accepted','running')
                   AND deadline_at>?2) AS commands,
               (SELECT COUNT(*) FROM host_credentials
                 WHERE host_id=?1 AND revoked_at IS NULL) AS credentials",
        )
        .bind(&id)
        .bind(now)
        .fetch_one(&mut *tx)
        .await?;
        let any = |column: &str| counts.get::<i64, _>(column) > 0;

        let mut blockers = Vec::new();
        if approved && !retired {
            blockers.push(HostDeleteBlocker::NotRetired);
        }
        let last_seen = last_heartbeat_at.max(last_inventory_at);
        if !retired && last_seen.is_some_and(|at| now.saturating_sub(at) < RECENTLY_SEEN_MILLIS) {
            blockers.push(HostDeleteBlocker::StillHeartbeating);
        }
        if approved && any("credentials") {
            blockers.push(HostDeleteBlocker::ActiveCredential);
        }
        if any("placements") || any("charged") {
            blockers.push(HostDeleteBlocker::LivePlacement);
        }
        if any("unconfirmed") {
            blockers.push(HostDeleteBlocker::UnconfirmedStart);
        }
        if any("runners") {
            blockers.push(HostDeleteBlocker::LiveRunner);
        }
        if any("commands") {
            blockers.push(HostDeleteBlocker::PendingCommand);
        }
        if !blockers.is_empty() {
            return Err(RegistryError::HostNotDeletable(blockers));
        }

        // Credentials go first: the schema refuses to mark a host deleted
        // while one is live.
        sqlx::query(
            "UPDATE host_credentials SET revoked_at=? WHERE host_id=? AND revoked_at IS NULL",
        )
        .bind(now)
        .bind(&id)
        .execute(&mut *tx)
        .await?;
        revoke_open_rotations(
            &mut tx,
            host_id,
            now,
            Some(user_id),
            "fleet.credential.rotation_revoked_by_delete",
        )
        .await?;
        sqlx::query(
            "UPDATE host_enrollments SET revoked_at=?
              WHERE intended_host_id=? AND consumed_at IS NULL AND revoked_at IS NULL",
        )
        .bind(now)
        .bind(&id)
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            "UPDATE host_backends SET enabled=0,updated_at=? WHERE host_id=? AND enabled=1",
        )
        .bind(now)
        .bind(&id)
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            "UPDATE host_target_grants SET revoked_at=?,revision=revision+1
              WHERE host_id=? AND revoked_at IS NULL",
        )
        .bind(now)
        .bind(&id)
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            "UPDATE host_user_grants SET revoked_at=?,revision=revision+1
              WHERE host_id=? AND revoked_at IS NULL",
        )
        .bind(now)
        .bind(&id)
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            "UPDATE fleet_hosts SET deleted_at=?,enrollment_state='revoked',lifecycle_state='retired',
               retired_at=COALESCE(retired_at,?),authority_expires_at=NULL,
               revision=revision+1,updated_at=? WHERE id=? AND deleted_at IS NULL",
        )
        .bind(now)
        .bind(now)
        .bind(now)
        .bind(&id)
        .execute(&mut *tx)
        .await?;
        audit_and_event(
            &mut tx,
            Some(principal),
            Some(host_id),
            "fleet.host.deleted",
            "host",
            &id,
            json!({
                "hostId": id,
                "name": name,
                "lastHeartbeatAt": last_heartbeat_at,
                "enrollment": enrollment,
                "lifecycle": lifecycle,
            }),
            now,
        )
        .await?;
        tx.commit().await?;
        Ok(DeletedHost { host_id, name })
    }
}
