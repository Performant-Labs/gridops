//! Log lines shipped by fleet agents, kept for a day.

use serde::{Deserialize, Serialize};
use sqlx::Row as _;

use super::{
    AuthenticatedHost, Principal, RegistryError, RegistryService, ensure_system_admin, require_user,
};
use crate::fleet::ids::HostId;

/// How long shipped lines are kept.
pub const AGENT_LOG_RETENTION_MILLIS: i64 = 86_400_000;
const MAX_BATCH: usize = 500;

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentLogBatch {
    pub lines: Vec<AgentLogLine>,
    #[serde(default)]
    pub dropped: u64,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AgentLogLine {
    pub logged_at: i64,
    pub level: String,
    pub target: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StoredAgentLog {
    pub id: i64,
    pub logged_at: i64,
    pub level: String,
    pub target: String,
    pub message: String,
}

impl RegistryService {
    /// Stores a batch from the host's agent and prunes its expired lines.
    pub async fn store_agent_logs(
        &self,
        authenticated: &AuthenticatedHost,
        batch: AgentLogBatch,
    ) -> Result<(), RegistryError> {
        if batch.lines.len() > MAX_BATCH {
            return Err(RegistryError::InvalidRequest("too many log lines"));
        }
        let mut tx = self.begin().await?;
        let now = self.clock.now_millis();
        super::ensure_authenticated_generation(&mut tx, authenticated, now).await?;
        let host_id = authenticated.host_id.to_string();
        let mut lines = batch.lines;
        if batch.dropped > 0 {
            lines.push(AgentLogLine {
                logged_at: now,
                level: "warn".into(),
                target: "gridops".into(),
                message: format!(
                    "{} agent log lines were dropped while GridOps was unreachable",
                    batch.dropped
                ),
            });
        }
        for line in lines {
            let level = match line.level.as_str() {
                "error" | "warn" | "info" | "debug" | "trace" => line.level,
                _ => "info".to_owned(),
            };
            sqlx::query(
                "INSERT INTO fleet_agent_logs (host_id,logged_at,level,target,message,received_at)
                 VALUES (?,?,?,?,?,?)",
            )
            .bind(&host_id)
            .bind(line.logged_at)
            .bind(level)
            .bind(truncate(line.target, 128))
            .bind(truncate(line.message, 8192))
            .bind(now)
            .execute(&mut *tx)
            .await?;
        }
        sqlx::query("DELETE FROM fleet_agent_logs WHERE host_id=? AND received_at<?")
            .bind(&host_id)
            .bind(now - AGENT_LOG_RETENTION_MILLIS)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(())
    }

    /// The host's newest log lines, oldest first.
    pub async fn agent_logs(
        &self,
        principal: &Principal,
        host_id: HostId,
        limit: u32,
    ) -> Result<Vec<StoredAgentLog>, RegistryError> {
        let user_id = require_user(principal).map_err(|_| RegistryError::Forbidden)?;
        let mut tx = self.begin().await?;
        ensure_system_admin(&mut tx, user_id).await?;
        let rows = sqlx::query(
            "SELECT * FROM (SELECT id,logged_at,level,target,message FROM fleet_agent_logs
               WHERE host_id=? ORDER BY id DESC LIMIT ?) ORDER BY id",
        )
        .bind(host_id.to_string())
        .bind(i64::from(limit.min(1000)))
        .fetch_all(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(rows
            .into_iter()
            .map(|row| StoredAgentLog {
                id: row.get("id"),
                logged_at: row.get("logged_at"),
                level: row.get("level"),
                target: row.get("target"),
                message: row.get("message"),
            })
            .collect())
    }
}

fn truncate(mut value: String, max: usize) -> String {
    if value.len() > max {
        let mut end = max;
        while !value.is_char_boundary(end) {
            end -= 1;
        }
        value.truncate(end);
    }
    value
}
