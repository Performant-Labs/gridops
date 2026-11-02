import { createFileRoute, getRouteApi } from "@tanstack/react-router";
import { Activity } from "lucide-react";
import { useState } from "react";

import { ListPagination } from "~/components/list-pagination";
import { LiveRefreshNotice } from "~/components/live-refresh-notice";
import { Callout, EmptyState, LoadingRows } from "~/components/page";
import { PoolEventTable, usePoolEvents } from "~/features/runner-pools/pool-events";
import { signalRefresh, useRefreshInterval } from "~/lib/use-refresh-interval";

export const Route = createFileRoute("/_app/runner-pools/$poolId/activity")({
  component: PoolActivityTab,
});

const poolRoute = getRouteApi("/_app/runner-pools/$poolId");

function PoolActivityTab() {
  const pool = poolRoute.useLoaderData();
  const [page, setPage] = useState(1);
  const state = usePoolEvents(pool.id, page);
  const off = useRefreshInterval(5_000) === null;
  const notice = state.status === "ready" ? (
    <LiveRefreshNotice lastUpdatedAt={state.updatedAt} off={off} onRefresh={signalRefresh} refreshing={false} />
  ) : null;

  if (state.status === "loading") return <LoadingRows />;
  if (state.status === "error") return <div className="p-4"><Callout title="Activity could not be loaded" tone="danger">{state.error}</Callout></div>;
  if (state.data.items.length === 0) {
    return (
      <>
        {notice}
        <EmptyState className="min-h-[50vh]" description="Provisioning, capacity, autoscaling, and routing events for this pool appear here." icon={Activity} title="No activity yet" />
      </>
    );
  }
  return (
    <>
      {notice}
      <PoolEventTable events={state.data.items} />
      <ListPagination itemCount={state.data.items.length} noun="events" onPageChange={setPage} page={state.data.page} perPage={state.data.perPage} total={state.data.total} />
    </>
  );
}
