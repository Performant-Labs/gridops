import { ExternalLink } from "lucide-react";

import { cn, formatDateTime } from "~/lib/utils";

import type { QueuedJobReference } from "./types";

/** When GitHub last reported the job's status, in the reader's time by `formatTime`. */
export function jobConfirmation(job: QueuedJobReference, formatTime: (iso: string) => string): string {
  return job.confirmedAt ? `GitHub last reported it ${job.status} at ${formatTime(job.confirmedAt)}.` : "Not yet confirmed with GitHub.";
}

/** The job, its repository and its workflow, linking to the job on GitHub. */
export function QueuedJobLink({ job, className }: { job: QueuedJobReference; className?: string }) {
  return (
    <a
      className={cn("inline-flex min-w-0 max-w-full items-center gap-1 text-xs text-muted-foreground underline-offset-2 hover:text-foreground hover:underline", className)}
      href={job.htmlUrl}
      rel="noreferrer"
      target="_blank"
      title={`Open ${job.name} on GitHub`}
    >
      <span className="truncate">{`${job.name} · ${job.repository} · ${job.workflow}`}</span>
      <ExternalLink aria-hidden className="size-3 shrink-0" />
    </a>
  );
}

/** Under the Oldest queued tile: which job it is and when GitHub last reported it. Nothing when no job is named. */
export function OldestQueuedJob({ job }: { job: QueuedJobReference | null | undefined }) {
  if (!job) return null;
  return (
    <div className="min-w-0 space-y-0.5">
      <QueuedJobLink className="flex" job={job} />
      <div className="text-2xs text-faint">{jobConfirmation(job, formatDateTime)}</div>
    </div>
  );
}
