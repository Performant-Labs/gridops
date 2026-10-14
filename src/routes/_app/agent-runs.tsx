import { Link, createFileRoute, useNavigate } from "@tanstack/react-router";
import { Bot, ExternalLink, GitPullRequestArrow } from "lucide-react";

import { ListPagination } from "~/components/list-pagination";
import { EmptyState, ListGroup, PageBody, PageHeader, listRowClassName } from "~/components/page";
import { ResourcePageLoading } from "~/components/resource-page-loading";
import { RunStatusIcon } from "~/components/status-icon";
import { Badge } from "~/components/ui/badge";
import { buttonVariants } from "~/components/ui/button";
import { Dialog, DialogTrigger, SheetContent } from "~/components/ui/dialog";
import { Tooltip } from "~/components/ui/tooltip";
import { AgentRunPanel } from "~/features/agent/agent-run-panel";
import { agentRunLabel, isTerminalAgentStatus } from "~/features/agent/agent-state";
import { type AgentRunListItem, type AgentRunStatus, getAgentRunsPage } from "~/features/agent/agent.functions";
import { useAgentRun } from "~/features/agent/use-agent-run";
import { validatePageSearch } from "~/lib/pagination";
import { useLiveRouteRefresh } from "~/lib/use-live-route-refresh";
import { formatAge, formatDuration } from "~/lib/utils";

export const Route = createFileRoute("/_app/agent-runs")({
  validateSearch: validatePageSearch,
  loaderDeps: ({ search }) => ({ page: search.page ?? 1 }),
  loader: ({ deps }) => getAgentRunsPage({ page: deps.page }),
  pendingComponent: () => <ResourcePageLoading icon={Bot} title="Agent runs" />,
  component: AgentRunsPage,
});

const STATUS_ICON: Record<AgentRunStatus, string> = {
  queued: "queued", running: "in_progress", succeeded: "success", failed: "failure", cancelled: "cancelled",
};

function AgentRunsPage() {
  const data = Route.useLoaderData();
  const navigate = useNavigate({ from: Route.fullPath });
  useLiveRouteRefresh(5_000, true);
  const active = data.items.filter((run) => !isTerminalAgentStatus(run.status));
  const finished = data.items.filter((run) => isTerminalAgentStatus(run.status));

  return (
    <>
      <PageHeader count={data.total || undefined} icon={Bot} title="Agent runs" />
      <PageBody>
        {data.items.length === 0 ? (
          <EmptyState description="Runs appear here when someone uses Fix with agent on a failed job, or when fixes start automatically." icon={Bot} title="No agent runs yet" />
        ) : (
          <>
            {active.length ? (
              <ListGroup count={active.length} icon={<RunStatusIcon status="in_progress" />} label="Active">
                {active.map((run) => <AgentRunRow key={run.id} run={run} />)}
              </ListGroup>
            ) : null}
            {finished.length ? (
              <ListGroup count={finished.length} icon={<RunStatusIcon status="success" />} label="Finished">
                {finished.map((run) => <AgentRunRow key={run.id} run={run} />)}
              </ListGroup>
            ) : null}
            <ListPagination itemCount={data.items.length} noun="agent runs" onPageChange={(page) => void navigate({ search: { page } })} page={data.page} perPage={data.perPage} total={data.total} />
          </>
        )}
      </PageBody>
    </>
  );
}

function AgentRunRow({ run }: { run: AgentRunListItem }) {
  const repositoryName = run.repository.split("/").pop() ?? run.repository;
  const label = agentRunLabel(run);
  const terminal = isTerminalAgentStatus(run.status);
  return (
    <Dialog>
      <div className={`${listRowClassName} relative gap-3 pr-2 sm:pr-4`}>
        <DialogTrigger asChild>
          <button aria-label={`Show agent run for ${run.jobName}`} className="absolute inset-0 outline-none" data-list-row="" type="button" />
        </DialogTrigger>
        <Tooltip content={label}><span className="relative inline-flex"><RunStatusIcon status={STATUS_ICON[run.status]} /></span></Tooltip>
        <span className="hidden w-40 shrink-0 truncate text-xs text-muted-foreground sm:block" title={run.repository}>
          {repositoryName}{run.runNumber ? <span className="text-faint"> #{run.runNumber}</span> : null}
        </span>
        <span className="min-w-0 flex-1">
          <span className="block truncate font-medium text-foreground">{run.title ?? `Fix ${run.jobName}`}</span>
          <span className="block truncate text-2xs text-muted-foreground">
            {run.workflowName ? `${run.workflowName} · ` : ""}{run.jobName}
            {!terminal && run.stage ? <span className="text-progress"> · {run.stage}</span> : null}
            {run.status === "failed" && run.error ? <span className="text-danger"> · {run.error}</span> : null}
          </span>
        </span>
        {run.pullRequestUrl ? (
          <a className="relative hidden shrink-0 items-center gap-1 rounded-full border border-success/30 bg-success/10 px-2 py-0.5 text-2xs font-medium text-success hover:bg-success/15 md:inline-flex" href={run.pullRequestUrl} rel="noreferrer" target="_blank">
            <GitPullRequestArrow className="size-3" />{run.pullRequestNumber ? `#${run.pullRequestNumber}` : "Pull request"}
          </a>
        ) : terminal ? <Badge className="hidden md:inline-flex" variant="outline">{label}</Badge> : null}
        {run.model ? <span className="hidden max-w-36 shrink-0 truncate font-mono text-2xs text-faint lg:block">{run.model}</span> : null}
        <span className="hidden w-28 shrink-0 truncate text-right text-xs text-muted-foreground lg:block">
          {run.trigger === "automatic" ? "Automatic" : run.requestedBy ? `@${run.requestedBy}` : "Manual"}
        </span>
        <span className="tabular hidden w-16 shrink-0 text-right text-xs text-muted-foreground sm:block">{formatDuration(run.startedAt ?? run.createdAt, run.completedAt)}</span>
        <span className="tabular w-14 shrink-0 text-right text-xs text-faint">{formatAge(run.createdAt)}</span>
      </div>
      <SheetContent
        actions={<Link className={buttonVariants({ size: "sm", variant: "outline" })} params={{ runId: String(run.runId) }} search={{ job: run.jobId }} to="/workflow-runs/$runId"><ExternalLink />Open job</Link>}
        className="max-w-xl"
        description={`${run.repository}${run.runNumber ? ` #${run.runNumber}` : ""} · ${run.jobName}`}
        heading={run.title ?? `Fix ${run.jobName}`}
      >
        <AgentRunDetails run={run} />
      </SheetContent>
    </Dialog>
  );
}

/** The run's live status and timeline, re-read while it's still working. */
function AgentRunDetails({ run }: { run: AgentRunListItem }) {
  const agentRun = useAgentRun(run.id);
  return (
    <div className="min-h-0 flex-1 overflow-y-auto">
      <AgentRunPanel
        error={agentRun.error}
        onRetry={() => void agentRun.reload()}
        onUpdate={agentRun.accept}
        run={agentRun.run}
        summary={{ id: run.id, status: run.status, outcome: run.outcome, pullRequestUrl: run.pullRequestUrl, pullRequestNumber: run.pullRequestNumber, createdAt: run.createdAt, completedAt: run.completedAt }}
      />
    </div>
  );
}
