//! Retiring a machine.
//!
//! A browser session retires under the original rules: an administrator, the
//! expected revision, and no placement still running. An API token (scope
//! `fleet:retire`) is held to stricter guard rails, checked in the same
//! transaction as the retire so nothing can change in between: the machine
//! must already be out of service, silent for the configured quiet period,
//! and free of every kind of live work. A leaked token can then only retire
//! a machine that was already dead to the fleet, never strand a working one.
//! The guard is chosen by the principal, here in the registry, so no HTTP
//! route can forget it.

use serde_json::json;
use sqlx::{Row as _, Sqlite, Transaction};

use super::{RegistryError, RegistryService, audit_and_event, ensure_system_admin, require_user};
use crate::fleet::{HostId, Principal, protocol::browser::HostRetireBlocker};

/// The guard rails an API-token retire is held to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RetirePolicy {
    /// How long the machine must have been silent (no heartbeat and no
    /// inventory) before a token may retire it. A machine that never
    /// reported in counts as silent.
    pub token_min_offline_millis: i64,
}

impl Default for RetirePolicy {
    fn default() -> Self {
        Self {
            token_min_offline_millis: 15 * 60_000,
        }
    }
}

impl RegistryService {
    /// Retires a host, revoking its credentials and keeping its history.
    ///
    /// For a session principal this is the original retire. For an API-token
    /// principal it is refused with [`RegistryError::HostNotRetirable`],
    /// naming every reason in a fixed order, while any of these holds: the
    /// lifecycle is `active`; the machine reported in within
    /// `policy.token_min_offline_millis`; a placement is not `cleaned`,
    /// `failed` or `uncertain`, or capacity it holds is not released; a
    /// placement is `uncertain`; a runner placed on it has not finished; a
    /// command for it is waiting or running within its deadline. Retiring a
    /// retired machine is a no-op for everyone.
    pub async fn retire_host_with_policy(
        &self,
        principal: &Principal,
        host_id: HostId,
        expected_revision: crate::fleet::Revision,
        policy: RetirePolicy,
    ) -> Result<(), RegistryError> {
        let user_id = require_user(principal).map_err(|_| RegistryError::Forbidden)?;
        let mut tx = self.begin().await?;
        let now = self.clock.now_millis();
        ensure_system_admin(&mut tx, user_id).await?;
        let expected = i64::try_from(expected_revision.get())
            .map_err(|_| RegistryError::InvalidRequest("revision exceeds SQLite integer"))?;
        let Some(row) = sqlx::query(
            "SELECT revision,lifecycle_state FROM fleet_hosts WHERE id=? AND deleted_at IS NULL",
        )
        .bind(host_id.to_string())
        .fetch_optional(&mut *tx)
        .await?
        else {
            return Err(RegistryError::NotFound);
        };
        if row.get::<String, _>("lifecycle_state") == "retired" {
            tx.commit().await?;
            return Ok(());
        }
        if row.get::<i64, _>("revision") != expected {
            return Err(RegistryError::StaleRevision);
        }
        if matches!(principal, Principal::ApiToken { .. }) {
            let blockers = token_retire_blockers(&mut tx, host_id, now, policy).await?;
            if !blockers.is_empty() {
                return Err(RegistryError::HostNotRetirable(blockers));
            }
        }
        let busy: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM workload_placements WHERE host_id=? AND state NOT IN ('cleaned','failed')",
        )
        .bind(host_id.to_string())
        .fetch_one(&mut *tx)
        .await?;
        if busy > 0 {
            return Err(RegistryError::InvalidRequest(
                "the machine still runs work; drain it first",
            ));
        }
        sqlx::query(
            "UPDATE fleet_hosts SET lifecycle_state='retired',retired_at=?,authority_expires_at=NULL,
               revision=revision+1,updated_at=? WHERE id=? AND revision=?",
        )
        .bind(now)
        .bind(now)
        .bind(host_id.to_string())
        .bind(expected)
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            "UPDATE host_credentials SET revoked_at=? WHERE host_id=? AND revoked_at IS NULL",
        )
        .bind(now)
        .bind(host_id.to_string())
        .execute(&mut *tx)
        .await?;
        audit_and_event(
            &mut tx,
            Some(principal),
            Some(host_id),
            "fleet.host.retired",
            "host",
            &host_id.to_string(),
            json!({}),
            now,
        )
        .await?;
        tx.commit().await?;
        Ok(())
    }
}

/// Every guard rail the machine fails for an API-token retire, in the order
/// of [`HostRetireBlocker`]. Read inside the retire transaction.
async fn token_retire_blockers(
    tx: &mut Transaction<'_, Sqlite>,
    host_id: HostId,
    now: i64,
    policy: RetirePolicy,
) -> Result<Vec<HostRetireBlocker>, RegistryError> {
    let id = host_id.to_string();
    let row = sqlx::query(
        "SELECT h.lifecycle_state,h.last_heartbeat_at,h.last_inventory_at,
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
               AND deadline_at>?2) AS commands
           FROM fleet_hosts h WHERE h.id=?1",
    )
    .bind(&id)
    .bind(now)
    .fetch_one(&mut **tx)
    .await?;
    let any = |column: &str| row.get::<i64, _>(column) > 0;
    let last_heartbeat_at: Option<i64> = row.get("last_heartbeat_at");
    let last_inventory_at: Option<i64> = row.get("last_inventory_at");
    let last_seen = last_heartbeat_at.max(last_inventory_at);

    let mut blockers = Vec::new();
    if row.get::<String, _>("lifecycle_state") == "active" {
        blockers.push(HostRetireBlocker::LifecycleActive);
    }
    if last_seen.is_some_and(|at| now.saturating_sub(at) < policy.token_min_offline_millis) {
        blockers.push(HostRetireBlocker::RecentlySeen);
    }
    if any("placements") || any("charged") {
        blockers.push(HostRetireBlocker::LivePlacement);
    }
    if any("unconfirmed") {
        blockers.push(HostRetireBlocker::UnconfirmedStart);
    }
    if any("runners") {
        blockers.push(HostRetireBlocker::LiveRunner);
    }
    if any("commands") {
        blockers.push(HostRetireBlocker::PendingCommand);
    }
    Ok(blockers)
}
