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
/// Lines a host may store in a clock hour. A busy machine's timeline is a few
/// hundred an hour and a full agent buffer (5,000) still fits; past this the
/// agent is flooding, and the rest of the hour's lines are dropped.
pub const AGENT_LOG_LINES_PER_HOUR: i64 = 6_000;
const HOUR_MILLIS: i64 = 3_600_000;
/// Target of the lines `GridOps` adds itself.
const NOTICE_TARGET: &str = "gridops";

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentLogBatch {
    pub lines: Vec<AgentLogLine>,
    #[serde(default)]
    pub dropped: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
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
        let mut lines: Vec<AgentLogLine> = batch
            .lines
            .into_iter()
            .map(|line| AgentLogLine {
                level: match line.level.as_str() {
                    "error" | "warn" | "info" | "debug" | "trace" => line.level,
                    _ => "info".to_owned(),
                },
                target: truncate(line.target, 128),
                message: truncate(line.message, 8192),
                logged_at: line.logged_at,
            })
            .collect();
        // An agent that didn't see the reply sends the batch again. Lines come
        // oldest first, so everything up to the host's newest stored line is
        // already here.
        let newest = sqlx::query(
            "SELECT logged_at,level,target,message FROM fleet_agent_logs
             WHERE host_id=? AND target<>? ORDER BY id DESC LIMIT 1",
        )
        .bind(&host_id)
        .bind(NOTICE_TARGET)
        .fetch_optional(&mut *tx)
        .await?;
        if let Some(newest) = newest {
            let newest = AgentLogLine {
                logged_at: newest.get("logged_at"),
                level: newest.get("level"),
                target: newest.get("target"),
                message: newest.get("message"),
            };
            if let Some(at) = lines.iter().rposition(|line| *line == newest) {
                lines.drain(..=at);
            }
        }
        if batch.dropped > 0 {
            lines.push(notice(
                now,
                format!(
                    "{} agent log lines were dropped while GridOps was unreachable",
                    batch.dropped
                ),
            ));
        }
        let hour = now - now.rem_euclid(HOUR_MILLIS);
        let stored: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM fleet_agent_logs WHERE host_id=? AND received_at>=?",
        )
        .bind(&host_id)
        .bind(hour)
        .fetch_one(&mut *tx)
        .await?;
        let room = usize::try_from(AGENT_LOG_LINES_PER_HOUR - stored).unwrap_or(0);
        if lines.len() > room {
            lines.truncate(room);
            // The notice takes the hour past its cap, so it is added once.
            if stored <= AGENT_LOG_LINES_PER_HOUR {
                lines.push(notice(
                    now,
                    format!(
                        "this machine sent more than {AGENT_LOG_LINES_PER_HOUR} log lines this hour; the rest of the hour's lines are dropped"
                    ),
                ));
            }
        }
        for line in lines {
            sqlx::query(
                "INSERT INTO fleet_agent_logs (host_id,logged_at,level,target,message,received_at)
                 VALUES (?,?,?,?,?,?)",
            )
            .bind(&host_id)
            .bind(line.logged_at)
            .bind(line.level)
            .bind(line.target)
            .bind(line.message)
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

fn notice(now: i64, message: String) -> AgentLogLine {
    AgentLogLine {
        logged_at: now,
        level: "warn".into(),
        target: NOTICE_TARGET.into(),
        message,
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
