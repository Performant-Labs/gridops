//! Whether a machine is fit to take new runners, judged by how its recent
//! starts went.
//!
//! A machine whose network drops packets, or whose agent keeps losing its
//! session, fails a share of the runner starts placed on it while it still
//! looks healthy. Each failure is recorded as a `placement_failed` event (see
//! [`super::failure`]). When a machine fails [`START_GATE_FAILURES`] starts
//! within [`START_GATE_WINDOW_MILLIS`], with no successful start after them,
//! admission stops choosing it for new placements until either the failures
//! age out of the window (the cool-down) or one of its starts succeeds.
//!
//! The gate only skips the machine when choosing where a new runner goes. It
//! never changes the machine's lifecycle, never drains or retires it, and
//! never touches what already runs there.

use serde_json::Value;
use sqlx::{Row as _, Sqlite, Transaction};

use super::failure::{PLACEMENT_FAILED_EVENT, PlacementFailureReason};

/// Start failures, with no successful start after them, that take a machine
/// out of placement. Three keeps a machine that loses one start in five or
/// ten to packet loss in use (a success in between resets the count), while
/// one that fails every start is set aside after three runners.
pub const START_GATE_FAILURES: usize = 3;

/// How far back start failures count. It is also the longest cool-down: a
/// gated machine is reconsidered once its third most recent failure is this
/// old. Fifteen minutes is several start attempts' worth (a start has a 30 s
/// command deadline and a 60 s registration window) without parking a machine
/// for long after a passing network fault.
pub const START_GATE_WINDOW_MILLIS: i64 = 15 * 60_000;

/// Agent failure codes that describe the request rather than the machine: a
/// cancelled command, an invalid configuration or an unsupported request
/// would fail the same way anywhere, so they don't count against a machine.
const NOT_THE_MACHINES_FAULT: [&str; 3] = ["cancelled", "configuration", "unsupported"];

/// Whether a recorded failure says a start failed on the machine, as opposed
/// to an administrator or the pool withdrawing it, or no cause being known.
/// `agent_code` is the agent's own failure code, when it reported one.
#[must_use]
pub fn is_start_failure(reason: PlacementFailureReason, agent_code: Option<&str>) -> bool {
    match reason {
        PlacementFailureReason::AgentReported => {
            !agent_code.is_some_and(|code| NOT_THE_MACHINES_FAULT.contains(&code))
        }
        PlacementFailureReason::AgentOutcomeUnknown
        | PlacementFailureReason::PrepareDeadlineExpired
        | PlacementFailureReason::StartDeadlineExpired
        | PlacementFailureReason::AgentSessionLost
        | PlacementFailureReason::HostEpochChanged => true,
        PlacementFailureReason::RunnerRemovedBeforeStart
        | PlacementFailureReason::OperatorReleased
        | PlacementFailureReason::Unknown => false,
    }
}

/// A machine set aside from new placements, and why.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StartGate {
    /// Start failures in the window since the last successful start.
    pub failures: usize,
    /// The oldest of them.
    pub since: i64,
    /// When the machine is reconsidered if nothing else changes: the moment
    /// fewer than [`START_GATE_FAILURES`] failures remain in the window.
    pub until: i64,
}

impl StartGate {
    /// The window in whole minutes, for messages.
    #[must_use]
    pub const fn window_minutes() -> i64 {
        START_GATE_WINDOW_MILLIS / 60_000
    }

    /// What an operator reads on the machine.
    #[must_use]
    pub fn message(&self) -> String {
        format!(
            "Not accepting new runners: {} start failures in the last {} minutes.",
            self.failures,
            Self::window_minutes()
        )
    }
}

/// The gate for a machine whose start failures happened at `failures`
/// (milliseconds, any order) and whose latest successful start, if any, was
/// at `last_success`. `None` means the machine takes new runners.
#[must_use]
pub fn evaluate(failures: &[i64], last_success: Option<i64>, now: i64) -> Option<StartGate> {
    let floor = (now - START_GATE_WINDOW_MILLIS).max(last_success.map_or(i64::MIN, |at| at + 1));
    let mut counted: Vec<i64> = failures
        .iter()
        .copied()
        .filter(|at| *at >= floor && *at <= now)
        .collect();
    if counted.len() < START_GATE_FAILURES {
        return None;
    }
    counted.sort_unstable_by(|a, b| b.cmp(a));
    let nth = counted[START_GATE_FAILURES - 1];
    Some(StartGate {
        failures: counted.len(),
        since: counted[counted.len() - 1],
        until: nth + START_GATE_WINDOW_MILLIS,
    })
}

/// The machine's gate now, from its recorded failures and its starts.
pub async fn start_gate(
    tx: &mut Transaction<'_, Sqlite>,
    host_id: &str,
    now: i64,
) -> Result<Option<StartGate>, sqlx::Error> {
    let since = now - START_GATE_WINDOW_MILLIS;
    let rows = sqlx::query(
        "SELECT created_at,detail_json FROM fleet_events
          WHERE host_id=? AND kind=? AND created_at>=?",
    )
    .bind(host_id)
    .bind(PLACEMENT_FAILED_EVENT)
    .bind(since)
    .fetch_all(&mut **tx)
    .await?;
    let failures: Vec<i64> = rows
        .iter()
        .filter(|row| {
            let detail: Value =
                serde_json::from_str(&row.get::<String, _>("detail_json")).unwrap_or_default();
            is_start_failure(
                PlacementFailureReason::from_code(
                    detail.get("reason").and_then(Value::as_str).unwrap_or(""),
                ),
                detail.get("agent_code").and_then(Value::as_str),
            )
        })
        .map(|row| row.get("created_at"))
        .collect();
    if failures.len() < START_GATE_FAILURES {
        return Ok(None);
    }
    // A start's result arrives within its deadline, so a start issued before
    // the window opened can't have succeeded inside it by much; the margin
    // keeps the read on the index without missing one.
    let last_success: Option<i64> = sqlx::query_scalar(
        "SELECT MAX(r.received_at) FROM host_commands c
           JOIN host_command_results r ON r.command_id=c.id
          WHERE c.host_id=? AND c.kind='start_runner' AND c.issued_at>=?
            AND c.state='succeeded' AND r.disposition='applied'",
    )
    .bind(host_id)
    .bind(since - 10 * 60_000)
    .fetch_one(&mut **tx)
    .await?;
    Ok(evaluate(&failures, last_success, now))
}

#[cfg(test)]
mod tests {
    use super::*;

    const MINUTE: i64 = 60_000;
    const NOW: i64 = 1_000 * MINUTE;

    #[test]
    fn fewer_failures_than_the_threshold_leave_the_machine_in_use() {
        assert_eq!(evaluate(&[], None, NOW), None);
        assert_eq!(evaluate(&[NOW - MINUTE, NOW - 2 * MINUTE], None, NOW), None);
    }

    #[test]
    fn enough_recent_failures_set_the_machine_aside_until_they_age_out() {
        let failures = [NOW - MINUTE, NOW - 4 * MINUTE, NOW - 9 * MINUTE];
        // The third most recent failure leaves the window first.
        let gate = StartGate {
            failures: 3,
            since: NOW - 9 * MINUTE,
            until: NOW - 9 * MINUTE + START_GATE_WINDOW_MILLIS,
        };
        assert_eq!(evaluate(&failures, None, NOW), Some(gate));
        assert_eq!(
            gate.message(),
            "Not accepting new runners: 3 start failures in the last 15 minutes."
        );
        // The cool-down: once that failure is older than the window, it lifts.
        assert_eq!(evaluate(&failures, None, gate.until + 1), None);
        assert!(evaluate(&failures, None, gate.until - 1).is_some());
    }

    #[test]
    fn with_more_failures_the_cool_down_follows_the_third_most_recent() {
        let failures = [
            NOW - 14 * MINUTE,
            NOW - MINUTE,
            NOW - 2 * MINUTE,
            NOW - 3 * MINUTE,
        ];
        assert_eq!(
            evaluate(&failures, None, NOW),
            Some(StartGate {
                failures: 4,
                since: NOW - 14 * MINUTE,
                until: NOW - 3 * MINUTE + START_GATE_WINDOW_MILLIS,
            })
        );
    }

    #[test]
    fn a_successful_start_after_the_failures_lifts_the_gate() {
        let failures = [NOW - 3 * MINUTE, NOW - 4 * MINUTE, NOW - 5 * MINUTE];
        assert!(evaluate(&failures, Some(NOW - 6 * MINUTE), NOW).is_some());
        assert_eq!(evaluate(&failures, Some(NOW - 2 * MINUTE), NOW), None);
        // Only failures after the success count.
        let more = [NOW - MINUTE, NOW - 3 * MINUTE, NOW - 4 * MINUTE];
        assert_eq!(evaluate(&more, Some(NOW - 2 * MINUTE), NOW), None);
    }

    #[test]
    fn failures_outside_the_window_do_not_count() {
        let failures = [NOW - MINUTE, NOW - 2 * MINUTE, NOW - 16 * MINUTE];
        assert_eq!(evaluate(&failures, None, NOW), None);
    }

    #[test]
    fn only_failures_the_machine_could_have_caused_count() {
        use PlacementFailureReason as R;
        assert!(is_start_failure(R::AgentReported, None));
        assert!(is_start_failure(R::AgentReported, Some("dependency")));
        assert!(is_start_failure(R::AgentReported, Some("runtime")));
        assert!(!is_start_failure(R::AgentReported, Some("configuration")));
        assert!(!is_start_failure(R::AgentReported, Some("unsupported")));
        assert!(!is_start_failure(R::AgentReported, Some("cancelled")));
        assert!(is_start_failure(R::StartDeadlineExpired, None));
        assert!(is_start_failure(R::PrepareDeadlineExpired, None));
        assert!(is_start_failure(R::AgentSessionLost, None));
        assert!(is_start_failure(R::HostEpochChanged, None));
        assert!(is_start_failure(R::AgentOutcomeUnknown, None));
        assert!(!is_start_failure(R::RunnerRemovedBeforeStart, None));
        assert!(!is_start_failure(R::OperatorReleased, None));
        assert!(!is_start_failure(R::Unknown, None));
    }
}
