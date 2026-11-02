import type { QueuedJobCandidate } from "./operations.functions";

const STATUS_LABELS: Record<string, string> = {
  manual: "manual mode: no runner is started for queued jobs",
  standby: "another pool runs it first",
};

/** What a run's page says beside a pool that could take a queued job. */
export function diagnosisCandidateLabel(candidate: QueuedJobCandidate) {
  if (candidate.status === "ready") return `${candidate.availableCapacity} slot${candidate.availableCapacity === 1 ? "" : "s"} available`;
  return STATUS_LABELS[candidate.status] ?? candidate.status.replaceAll("_", " ");
}
