import { createFileRoute, useNavigate } from "@tanstack/react-router";
import { Hourglass } from "lucide-react";

import { PageBody, PageHeader } from "~/components/page";
import { ResourcePageLoading } from "~/components/resource-page-loading";
import { getQueuedJobsPage } from "~/features/dashboard/dashboard.functions";
import { QueuedJobsList } from "~/features/dashboard/queued-jobs";
import { validatePageSearch } from "~/lib/pagination";
import { useLiveRouteRefresh } from "~/lib/use-live-route-refresh";

export const Route = createFileRoute("/_app/queued-jobs")({
  validateSearch: validatePageSearch,
  loaderDeps: ({ search }) => ({ page: search.page ?? 1 }),
  loader: ({ deps }) => getQueuedJobsPage({ page: deps.page }),
  pendingComponent: () => <ResourcePageLoading icon={Hourglass} title="Queued jobs" />,
  component: QueuedJobsPage,
});

function QueuedJobsPage() {
  const data = Route.useLoaderData();
  const navigate = useNavigate({ from: Route.fullPath });
  useLiveRouteRefresh(5_000, data.authenticated);

  return (
    <>
      <PageHeader count={data.total || undefined} icon={Hourglass} title="Queued jobs" />
      <PageBody>
        <QueuedJobsList items={data.items} onPageChange={(page) => void navigate({ search: { page } })} page={data.page} perPage={data.perPage} total={data.total} />
      </PageBody>
    </>
  );
}
