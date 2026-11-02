/** Words for why placements failed, as the API records them. */
import type { PlacementFailureGroup, StartGate } from "./query-contracts";

type Reason = PlacementFailureGroup["reason"];

const labels: Record<Reason, string> = {
  agent_reported: "Agent reported a failure",
  agent_outcome_unknown: "Agent could not say what happened",
  prepare_deadline_expired: "Prepare timed out",
  start_deadline_expired: "Start timed out",
  agent_session_lost: "Agent session dropped",
  host_epoch_changed: "Machine re-enrolled or recovered",
  runner_removed_before_start: "Runner removed before it started",
  operator_released: "Released by an administrator",
  unknown: "Cause not recorded",
};

export const failureReasonLabel = (reason: Reason) => labels[reason];

/** One line for a group: the reason, then the newest message, or how many when there is none. */
export function failureSummary(group: Pick<PlacementFailureGroup, "reason" | "message" | "count">): string {
  const label = failureReasonLabel(group.reason);
  if (group.message) return `${label}: ${group.message}`;
  return group.count > 1 ? `${label} (${group.count} times)` : label;
}

/** `null` when nothing failed, so the caller shows nothing at all. */
export const failuresLastHourLabel = (count: number): string | null => (count > 0 ? `${count} failed in the last hour` : null);

/**
 * When a machine left out of new placements is reconsidered. `null` when it
 * takes new runners. `formatTime` renders the moment in the reader's time.
 */
export const startGateLabel = (gate: StartGate | null, formatTime: (iso: string) => string): string | null =>
  gate ? `${gate.message} Reconsidered at ${formatTime(gate.until)}, or sooner if a start on it succeeds.` : null;
