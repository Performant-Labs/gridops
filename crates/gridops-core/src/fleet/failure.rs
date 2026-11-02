//! Why a placement failed.
//!
//! Every path that moves a placement to `failed` records a `placement_failed`
//! fleet event on its host, with a stable machine-readable `reason`, a short
//! human message and the numbers behind the decision. The runner row of the
//! placement, when there is one, carries the same message as its
//! `failure_reason`. Reasons come only from facts the code has at that point;
//! a path that has no cause records [`PlacementFailureReason::Unknown`]
//! rather than a guess.
//!
//! `upstream_cleanup_state` is not a failure reason: it is a cleanup state
//! that successful placements carry too.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use sqlx::{Row as _, Sqlite, Transaction};
use uuid::Uuid;

/// The `fleet_events.kind` of a recorded failure.
pub const PLACEMENT_FAILED_EVENT: &str = "placement_failed";

/// Longest message kept, in characters.
const MESSAGE_LIMIT: usize = 240;

/// Why a placement failed. The set is closed: a new reason is a new variant
/// and a new stable code, never free text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlacementFailureReason {
    /// The agent answered a prepare or start command with a failure.
    AgentReported,
    /// The agent answered a prepare command with an unknown outcome, which
    /// fails it because a prepare starts nothing.
    AgentOutcomeUnknown,
    /// A prepare command went unanswered past its deadline.
    PrepareDeadlineExpired,
    /// A start command could not be delivered before its deadline.
    StartDeadlineExpired,
    /// The agent's session ended or changed before the command reached it.
    AgentSessionLost,
    /// The machine's epoch changed (re-enrolled or recovered) before the
    /// command reached it.
    HostEpochChanged,
    /// The runner was removed before its start command was ever issued.
    RunnerRemovedBeforeStart,
    /// An administrator released a start that was never confirmed.
    OperatorReleased,
    /// No cause was available.
    Unknown,
}

impl PlacementFailureReason {
    /// The stable code stored in `detail_json` and served to clients.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::AgentReported => "agent_reported",
            Self::AgentOutcomeUnknown => "agent_outcome_unknown",
            Self::PrepareDeadlineExpired => "prepare_deadline_expired",
            Self::StartDeadlineExpired => "start_deadline_expired",
            Self::AgentSessionLost => "agent_session_lost",
            Self::HostEpochChanged => "host_epoch_changed",
            Self::RunnerRemovedBeforeStart => "runner_removed_before_start",
            Self::OperatorReleased => "operator_released",
            Self::Unknown => "unknown",
        }
    }

    /// The reason for a stored code. A code this build does not know is
    /// `Unknown`, so an older server can read what a newer one wrote.
    #[must_use]
    pub fn from_code(code: &str) -> Self {
        [
            Self::AgentReported,
            Self::AgentOutcomeUnknown,
            Self::PrepareDeadlineExpired,
            Self::StartDeadlineExpired,
            Self::AgentSessionLost,
            Self::HostEpochChanged,
            Self::RunnerRemovedBeforeStart,
            Self::OperatorReleased,
        ]
        .into_iter()
        .find(|reason| reason.as_str() == code)
        .unwrap_or(Self::Unknown)
    }
}

/// A cause for a failed placement: a reason, a short message and the facts
/// behind them.
#[derive(Debug, Clone, PartialEq)]
pub struct PlacementFailure {
    reason: PlacementFailureReason,
    message: String,
    details: Map<String, Value>,
}

impl PlacementFailure {
    #[must_use]
    pub fn new(reason: PlacementFailureReason, message: impl Into<String>) -> Self {
        Self {
            reason,
            message: truncate(message.into()),
            details: Map::new(),
        }
    }

    /// Adds one fact, such as `elapsed_ms`, to the recorded detail. The
    /// `reason` and `message` keys are reserved.
    #[must_use]
    pub fn with(mut self, key: &str, value: impl Into<Value>) -> Self {
        if key != "reason" && key != "message" {
            self.details.insert(key.to_owned(), value.into());
        }
        self
    }

    #[must_use]
    pub fn unknown() -> Self {
        Self::new(PlacementFailureReason::Unknown, "No cause was recorded.")
    }

    /// The runner was removed, or failed, before its start was issued.
    #[must_use]
    pub fn runner_removed_before_start(runner_status: &str) -> Self {
        Self::new(
            PlacementFailureReason::RunnerRemovedBeforeStart,
            format!("The runner was marked {runner_status} before its start command was issued."),
        )
        .with("runner_status", runner_status)
    }

    /// An administrator released a start nobody could confirm.
    #[must_use]
    pub fn operator_released(cancelled: &[String]) -> Self {
        Self::new(
            PlacementFailureReason::OperatorReleased,
            "An administrator released this placement because its start was never confirmed.",
        )
        .with("cancelled_commands", json!(cancelled))
    }

    #[must_use]
    pub fn reason(&self) -> PlacementFailureReason {
        self.reason
    }

    #[must_use]
    pub fn message(&self) -> &str {
        &self.message
    }

    fn detail(&self) -> Value {
        let mut detail = self.details.clone();
        detail.insert("reason".to_owned(), json!(self.reason.as_str()));
        detail.insert("message".to_owned(), json!(self.message));
        Value::Object(detail)
    }
}

fn truncate(message: String) -> String {
    if message.chars().count() <= MESSAGE_LIMIT {
        return message;
    }
    let mut kept: String = message.chars().take(MESSAGE_LIMIT - 1).collect();
    kept.push('…');
    kept
}

/// Moves a placement to `failed` and records why, in one step.
///
/// The placement must be in one of `from` (any state when `None`) and not
/// already failed; otherwise nothing changes and this returns `false`, so a
/// failure is recorded once however often a path reaches it. The event
/// carries the placement's host and target. The placement's runner row, if it
/// has one that was not deleted and has no reason yet, gets the message as its
/// `failure_reason`.
pub(crate) async fn fail_placement(
    tx: &mut Transaction<'_, Sqlite>,
    placement_id: &str,
    from: Option<&[&str]>,
    failure: &PlacementFailure,
    actor_user_id: Option<&str>,
    now: i64,
) -> Result<bool, sqlx::Error> {
    let Some(row) = sqlx::query(
        "SELECT state,host_id,target_id,workload_id FROM workload_placements WHERE id=?",
    )
    .bind(placement_id)
    .fetch_optional(&mut **tx)
    .await?
    else {
        return Ok(false);
    };
    let state: String = row.get("state");
    if state == "failed" || from.is_some_and(|from| !from.contains(&state.as_str())) {
        return Ok(false);
    }
    sqlx::query("UPDATE workload_placements SET state='failed',updated_at=? WHERE id=?")
        .bind(now)
        .bind(placement_id)
        .execute(&mut **tx)
        .await?;
    sqlx::query(
        "INSERT INTO fleet_events (id,host_id,placement_id,target_id,actor_user_id,kind,detail_json,created_at)
         VALUES (?,?,?,?,?,?,?,?)",
    )
    .bind(Uuid::new_v4().to_string())
    .bind(row.get::<String, _>("host_id"))
    .bind(placement_id)
    .bind(row.get::<String, _>("target_id"))
    .bind(actor_user_id)
    .bind(PLACEMENT_FAILED_EVENT)
    .bind(failure.detail().to_string())
    .bind(now)
    .execute(&mut **tx)
    .await?;
    sqlx::query(
        "UPDATE runners SET failure_reason=?,updated_at=?
          WHERE id=? AND status!='deleted' AND COALESCE(failure_reason,'')=''",
    )
    .bind(&failure.message)
    .bind(now)
    .bind(row.get::<String, _>("workload_id"))
    .execute(&mut **tx)
    .await?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reason_codes_are_stable_and_round_trip() {
        for reason in [
            PlacementFailureReason::AgentReported,
            PlacementFailureReason::AgentOutcomeUnknown,
            PlacementFailureReason::PrepareDeadlineExpired,
            PlacementFailureReason::StartDeadlineExpired,
            PlacementFailureReason::AgentSessionLost,
            PlacementFailureReason::HostEpochChanged,
            PlacementFailureReason::RunnerRemovedBeforeStart,
            PlacementFailureReason::OperatorReleased,
            PlacementFailureReason::Unknown,
        ] {
            assert_eq!(PlacementFailureReason::from_code(reason.as_str()), reason);
            assert_eq!(
                serde_json::to_value(reason).ok(),
                Some(json!(reason.as_str()))
            );
        }
        assert_eq!(
            PlacementFailureReason::from_code("a_future_reason"),
            PlacementFailureReason::Unknown
        );
        assert_eq!(
            PlacementFailureReason::StartDeadlineExpired.as_str(),
            "start_deadline_expired"
        );
    }

    #[test]
    fn detail_holds_reason_message_and_facts_and_reserves_its_keys() {
        let failure = PlacementFailure::new(PlacementFailureReason::AgentReported, "It failed.")
            .with("elapsed_ms", 31_200)
            .with("reason", "spoofed")
            .with("message", "spoofed");
        assert_eq!(
            failure.detail(),
            json!({"reason": "agent_reported", "message": "It failed.", "elapsed_ms": 31_200})
        );
    }

    #[test]
    fn long_messages_are_cut_to_the_limit() {
        let failure = PlacementFailure::new(PlacementFailureReason::Unknown, "x".repeat(1_000));
        assert_eq!(failure.message().chars().count(), MESSAGE_LIMIT);
        assert!(failure.message().ends_with('…'));
    }
}
