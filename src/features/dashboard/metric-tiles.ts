import type { HealthTone } from "~/components/status-icon";
import { successRateTile } from "./success-rate";
import type { DashboardOverview } from "./types";

export type MetricTile = {
  label: string;
  value: string | number;
  hint: string;
  to: "/runners" | "/queued-jobs" | "/workflow-runs";
  tone: HealthTone;
};

type Metrics = DashboardOverview["metrics"];

/**
 * The runner states in the order they are listed, each runner in exactly one:
 * the parts always add up to the Runners headline.
 */
export function runnerParts(metrics: Metrics) {
  return [
    { label: "busy", count: metrics.busy },
    { label: "idle", count: metrics.idle },
    { label: "starting", count: metrics.starting },
    { label: "unavailable", count: metrics.unavailable },
  ];
}

/** Jobs a pool can serve, then jobs no pool can; they add up to the Queued headline. */
export function queueParts(metrics: Metrics) {
  return [
    { label: "waiting for capacity", count: metrics.queuedWaiting },
    { label: "no matching pool", count: metrics.queuedUnmatched },
  ];
}

/** The tiles of the Overview strip, built so each hint partitions its headline. */
export function metricTiles(metrics: Metrics): MetricTile[] {
  const onlineRunners = metrics.busy + metrics.idle;
  const runnerHint = runnerParts(metrics)
    .filter((part) => part.count > 0)
    .map((part) => `${part.count} ${part.label}`)
    .join(" · ");
  const queueHint = queueParts(metrics).map((part) => `${part.count} ${part.label}`).join(" · ");
  return [
    { label: "Runners", value: metrics.runners, hint: runnerHint || "No runners yet", to: "/runners", tone: onlineRunners > 0 ? "success" : "neutral" },
    { label: "Busy", value: `${metrics.busy} of ${onlineRunners}`, hint: onlineRunners > 0 ? `${Math.round((metrics.busy / onlineRunners) * 100)}% of online runners` : "No runner is online", to: "/runners", tone: metrics.busy > 0 ? "progress" : "neutral" },
    { label: "Idle", value: metrics.idle, hint: metrics.idle > 0 ? "Online and free for the queue" : "Nothing free right now", to: "/runners", tone: metrics.idle > 0 ? "success" : "neutral" },
    { label: "Queued jobs", value: metrics.queuedJobs, hint: metrics.queuedJobs > 0 ? queueHint : "Queue is clear", to: "/queued-jobs", tone: queuedTone(metrics) },
    { label: "Success rate", ...successRateTile(metrics), to: "/workflow-runs", tone: successTone(metrics.successRate) },
  ];
}

/** Jobs no pool can ever serve need action, so they turn the tile red; plain waiting is a warning. */
export function queuedTone(metrics: Pick<Metrics, "queuedWaiting" | "queuedUnmatched">): HealthTone {
  if (metrics.queuedUnmatched > 0) return "danger";
  return metrics.queuedWaiting > 0 ? "warning" : "success";
}

export function successTone(rate: number | null): HealthTone {
  if (rate === null) return "neutral";
  return rate >= 90 ? "success" : rate >= 70 ? "warning" : "danger";
}
