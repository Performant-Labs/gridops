import { Link } from "@tanstack/react-router";
import { Hourglass } from "lucide-react";

import { EmptyState, InlineError, LoadingRows } from "~/components/page";
import { ListPagination } from "~/components/list-pagination";
import { Badge } from "~/components/ui/badge";
import { statusLabel } from "~/components/status-icon";
import { cn } from "~/lib/utils";
import type { QueuedJob } from "./types";

type Reason = { label: string; hint: string; tone: "danger" | "warning" | "muted" };

/** A job no pool serves, from a server that doesn't say which labels are missing. */
const NO_MATCHING_POOL: Reason = { label: "No pool offers these labels", hint: "No unpaused autoscaling pool serving this repository can run every requested label, so no runner will be started for it.", tone: "danger" };

const REASONS: Record<string, Reason> = {
  no_matching_pool: NO_MATCHING_POOL,
  pool_at_capacity: { label: "Pool is at its runner limit", hint: "Every pool that can run this job is using all of its runners, so it waits for one to finish.", tone: "warning" },
  runner_starting: { label: "Runner starting", hint: "Its pool is starting a runner.", tone: "muted" },
  waiting_for_runner: { label: "Waiting for a runner", hint: "Its pool has room and will start or free a runner for it.", tone: "muted" },
  pool_not_visible: { label: "Pool not visible to you", hint: "Its pool also covers repositories you can't access, so it isn't named.", tone: "muted" },
};

/** What to say about a job no pool serves, given the labels no pool provides (unknown on older servers). */
function unmatchedReason(missingLabels: string[] | undefined): Reason {
  if (!missingLabels) return NO_MATCHING_POOL;
  if (missingLabels.length > 0) {
    return {
      label: `No pool provides ${missingLabels.join(", ")}`,
      hint: "No runner pool you can see offers these labels, so no runner will be started for this job until a pool is given them.",
      tone: "danger",
    };
  }
  return {
    label: "No available pool",
    hint: "Pools offer each of these labels, but every one that could run this job is paused, has an open provisioning circuit, has autoscaling off or doesn't offer them all together.",
    tone: "danger",
  };
}

/** "45s", "12m", "7h 22m", "2d 3h": how long a job has waited. */
export function formatWait(seconds: number) {
  const value = Math.max(0, Math.floor(seconds));
  if (value < 60) return `${value}s`;
  const minutes = Math.floor(value / 60);
  if (minutes < 60) return `${minutes}m`;
  const hours = Math.floor(minutes / 60);
  if (hours < 24) return `${hours}h ${minutes % 60}m`;
  return `${Math.floor(hours / 24)}d ${hours % 24}h`;
}

type QueuedJobsListProps = {
  error?: string | null;
  items: QueuedJob[];
  loading?: boolean;
  /** Without a page handler the footer pagination is left out. */
  onPageChange?: (page: number) => void;
  onRetry?: () => void;
  page: number;
  perPage: number;
  total: number;
};

/**
 * The jobs counted as queued, longest-waiting first: what each is, the labels it
 * asked for, the pool that will run it (or that none can), and how long and why
 * it has waited.
 */
export function QueuedJobsList({ error = null, items, loading = false, onPageChange, onRetry, page, perPage, total }: QueuedJobsListProps) {
  if (items.length === 0) {
    if (error) return <div className="px-4 py-4"><InlineError onRetry={onRetry} title="Could not load queued jobs">{error}</InlineError></div>;
    if (loading) return <LoadingRows rows={3} />;
    return <EmptyState className="min-h-0 py-10" description="Every job has a runner or has started." icon={Hourglass} title="No queued jobs" />;
  }
  return (
    <div>
      {error ? <div className="border-b border-border px-4 py-2"><InlineError className="mt-0" onRetry={onRetry} title="Could not refresh queued jobs">{error}</InlineError></div> : null}
      <div className="overflow-x-auto">
        <table className="w-full min-w-[44rem] border-collapse text-left text-sm">
          <thead>
            <tr className="border-b border-border bg-panel-subtle text-xs text-muted-foreground">
              <th className="h-9 px-4 font-medium" scope="col">Job</th>
              <th className="h-9 px-3 font-medium" scope="col">Repository</th>
              <th className="h-9 px-3 font-medium" scope="col">Labels</th>
              <th className="h-9 px-3 font-medium" scope="col">Pool</th>
              <th className="h-9 px-4 text-right font-medium" scope="col">Waiting</th>
            </tr>
          </thead>
          <tbody>
            {items.map((job) => <QueuedJobRow job={job} key={job.id} />)}
          </tbody>
        </table>
      </div>
      {onPageChange ? <ListPagination itemCount={items.length} noun="queued jobs" onPageChange={onPageChange} page={page} perPage={perPage} total={total} /> : null}
    </div>
  );
}

function QueuedJobRow({ job }: { job: QueuedJob }) {
  const unmatched = job.waitingReason === "no_matching_pool";
  const reason = unmatched
    ? unmatchedReason(job.missingLabels)
    : REASONS[job.waitingReason] ?? { label: statusLabel(job.waitingReason), hint: "", tone: "muted" };
  const missing = new Set((unmatched ? job.missingLabels ?? [] : []).map((label) => label.toLowerCase()));
  return (
    <tr className="border-b border-border align-top transition-colors last:border-b-0 hover:bg-hover" data-reason={job.waitingReason}>
      <td className="max-w-64 px-4 py-2.5">
        <Link className="block truncate font-medium text-foreground hover:underline" params={{ runId: String(job.runId) }} title={job.name} to="/workflow-runs/$runId">{job.name}</Link>
        <span className="block truncate text-2xs text-muted-foreground" title={job.workflow}>{job.workflow}</span>
      </td>
      <td className="max-w-48 px-3 py-2.5 text-xs text-muted-foreground"><span className="block truncate" title={job.repository}>{job.repository}</span></td>
      <td className="px-3 py-2.5">
        <div className="flex max-w-60 flex-wrap gap-1">
          {job.labels.map((label) => {
            const isMissing = missing.has(label.toLowerCase());
            return <Badge className="font-mono" data-missing={isMissing ? "true" : undefined} key={label} title={isMissing ? "No pool provides this label" : undefined} variant={isMissing ? "destructive" : "outline"}>{label}</Badge>;
          })}
        </div>
      </td>
      <td className="px-3 py-2.5">
        {job.matchedPool ? (
          <Link className="block max-w-40 truncate text-foreground hover:underline" params={{ poolId: job.matchedPool.id }} title={job.matchedPool.name} to="/runner-pools/$poolId">{job.matchedPool.name}</Link>
        ) : unmatched ? (
          <Badge title={reason.hint} variant="destructive">No matching pool</Badge>
        ) : (
          <span className="text-xs text-faint" title={reason.hint}>Hidden</span>
        )}
      </td>
      <td className="whitespace-nowrap px-4 py-2.5 text-right">
        <span className={cn("tabular block font-medium", job.ageSeconds >= 300 ? "text-warning" : "text-foreground")}>{formatWait(job.ageSeconds)}</span>
        <span className={cn("block text-2xs", reason.tone === "danger" ? "text-danger" : reason.tone === "warning" ? "text-warning" : "text-faint")} title={reason.hint}>{reason.label}</span>
      </td>
    </tr>
  );
}
