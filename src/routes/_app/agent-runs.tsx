import { createFileRoute, useNavigate } from "@tanstack/react-router";
import { Bot } from "lucide-react";

import { ListPagination } from "~/components/list-pagination";
import { EmptyState, ListGroup, PageBody, PageHeader } from "~/components/page";
import { ResourcePageLoading } from "~/components/resource-page-loading";
import { RunStatusIcon } from "~/components/status-icon";
import { AgentRunRow } from "~/features/agent/agent-run-row";
import { isTerminalAgentStatus } from "~/features/agent/agent-state";
import { getAgentRunsPage } from "~/features/agent/agent.functions";
import { validatePageSearch } from "~/lib/pagination";
import { useLiveRouteRefresh } from "~/lib/use-live-route-refresh";

export const Route = createFileRoute("/_app/agent-runs")({
  validateSearch: validatePageSearch,
  loaderDeps: ({ search }) => ({ page: search.page ?? 1 }),
  loader: ({ deps }) => getAgentRunsPage({ page: deps.page }),
  pendingComponent: () => <ResourcePageLoading icon={Bot} title="Agent runs" />,
  component: AgentRunsPage,
});

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
