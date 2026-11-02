/** When a machine can be deleted, and the words for a refusal. */
import type { HostDeleteBlocker } from "./contracts";
import { FleetRequestError } from "./registry-api";
import type { HostReadV1 } from "./query-contracts";

type Host = Extract<HostReadV1, { projection: "host" }>;

/**
 * Whether to offer Delete at all: the machine is retired, or never finished
 * enrolling and has gone quiet. An approved machine has to be retired first,
 * and one still reporting in is not offered. The server enforces this and
 * everything else (running work, pending commands); this only keeps the
 * button off machines that can never qualify.
 */
export function canOfferDelete(host: Pick<Host, "enrollment" | "lifecycle" | "heartbeatFreshness">): boolean {
  if (host.lifecycle === "retired") return true;
  if (host.enrollment === "approved") return false;
  return host.heartbeatFreshness === "offline" || host.heartbeatFreshness === "unknown";
}

const blockerText: Record<HostDeleteBlocker, string> = {
  not_retired: "it is still in service; retire it first",
  still_heartbeating: "it reported in within the last 90 seconds",
  active_credential: "its credential is still active",
  live_placement: "a job placed on it is not cleaned up yet",
  unconfirmed_start: "a job start on it could not be confirmed; repair it first",
  live_runner: "a runner on it has not finished",
  pending_command: "a command for it is still waiting",
};

/** One sentence naming every reason a delete was refused. */
export function blockerSummary(reasons: readonly HostDeleteBlocker[]): string {
  if (reasons.length === 0) return "This machine can’t be deleted yet.";
  return `This machine can’t be deleted yet: ${reasons.map((reason) => blockerText[reason]).join("; ")}.`;
}

/** The sentence for a refused delete, or `null` when the failure was something else. */
export function refusalMessage(error: unknown): string | null {
  if (!(error instanceof FleetRequestError) || error.code !== "host_not_deletable") return null;
  const details = error.details;
  return blockerSummary(details?.kind === "delete_blocked" ? details.reasons : []);
}

/** The question the confirmation asks; it names the machine and says it cannot be undone. */
export const deleteConfirmation = (name: string) =>
  `Delete ${name}? It disappears from the machine list and counts for good, and its credential stays revoked. Past jobs and the audit log keep showing it. This can’t be undone; to use the computer again, enroll it as a new machine.`;
