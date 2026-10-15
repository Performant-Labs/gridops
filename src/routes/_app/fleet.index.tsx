import { useQuery } from "@tanstack/react-query";
import { Link, createFileRoute } from "@tanstack/react-router";
import { RefreshCw, Server } from "lucide-react";

import { Callout, EmptyState, LoadingRows, PageBody, PageHeader } from "~/components/page";
import { Badge } from "~/components/ui/badge";
import { Button } from "~/components/ui/button";
import { AddMachineButton } from "~/features/fleet/add-machine-sheet";
import {
  EnrollmentBadge, FreshnessBadge, PressureBadge, fleetErrorMessage, lifecycleLabel, platformLabel,
} from "~/features/fleet/fleet-ui";
import { listHosts } from "~/features/fleet/query-api";
import type { HostReadV1 } from "~/features/fleet/query-contracts";
import { cn, formatRelativeTime } from "~/lib/utils";

export const Route = createFileRoute("/_app/fleet/")({
  component: MachinesPage,
});

function MachinesPage() {
  const hosts = useQuery({
    queryKey: ["fleet", "hosts"],
    queryFn: ({ signal }) => listHosts({ limit: 100 }, signal),
    refetchInterval: 10_000,
  });
  const items = hosts.data?.items ?? [];

  return (
    <>
      <PageHeader
        actions={
          <>
            <Button disabled={hosts.isFetching} onClick={() => void hosts.refetch()} size="sm" variant="ghost">
              <RefreshCw className={cn(hosts.isFetching && "animate-spin")} /><span className="hidden sm:inline">Refresh</span>
            </Button>
            <AddMachineButton />
          </>
        }
        count={items.length || undefined}
        icon={Server}
        title="Machines"
      />
      <PageBody>
        {hosts.isPending ? (
          <LoadingRows />
        ) : hosts.isError ? (
          <div className="p-4">
            <Callout action={<Button onClick={() => void hosts.refetch()} size="sm" variant="outline">Retry</Button>} title="Couldn’t load machines" tone="danger">
              {fleetErrorMessage(hosts.error, "Machines could not be loaded.")}
            </Callout>
          </div>
        ) : items.length === 0 ? (
          <EmptyState
            description="Add a Linux or macOS machine you own and GridOps can run pool runners on it instead of the built-in runner host."
            icon={Server}
            title="No machines yet"
          >
            <AddMachineButton />
          </EmptyState>
        ) : (
          <>
            <table className="w-full text-sm">
              <thead className="sticky top-0 z-10 border-b border-border bg-panel-subtle/95 text-left text-xs text-muted-foreground backdrop-blur">
                <tr>
                  <th className="px-4 py-2 font-medium" scope="col">Name</th>
                  <th className="hidden px-3 py-2 font-medium sm:table-cell" scope="col">Platform</th>
                  <th className="px-3 py-2 font-medium" scope="col">Status</th>
                  <th className="hidden px-3 py-2 font-medium md:table-cell" scope="col">Lifecycle</th>
                  <th className="hidden px-3 py-2 font-medium lg:table-cell" scope="col">Pressure</th>
                  <th className="hidden px-3 py-2 text-right font-medium lg:table-cell" scope="col">Backends</th>
                  <th className="hidden px-3 py-2 font-medium xl:table-cell" scope="col">Agent</th>
                  <th className="hidden px-4 py-2 text-right font-medium md:table-cell" scope="col">Last heartbeat</th>
                </tr>
              </thead>
              <tbody>{items.map((host) => <MachineRow host={host} key={host.id} />)}</tbody>
            </table>
            {hosts.data?.nextCursor ? <p className="px-4 py-3 text-xs text-muted-foreground">Showing the first 100 machines.</p> : null}
          </>
        )}
      </PageBody>
    </>
  );
}

function MachineRow({ host }: { host: HostReadV1 }) {
  const full = host.projection === "host" ? host : null;
  const dash = <span className="text-faint">—</span>;
  return (
    <tr className="border-b border-border transition-colors hover:bg-hover">
      <td className="px-4 py-2.5">
        <Link className="font-medium text-foreground hover:underline focus-visible:underline" params={{ hostId: host.id }} to="/fleet/machines/$hostId">{host.name}</Link>
        <div className="text-xs text-muted-foreground sm:hidden">{platformLabel(host.hostOs, host.architecture)}</div>
      </td>
      <td className="hidden px-3 py-2.5 text-muted-foreground sm:table-cell">{platformLabel(host.hostOs, host.architecture)}</td>
      <td className="px-3 py-2.5">
        {full ? (
          <div className="flex flex-wrap gap-1">
            {full.enrollment === "approved" ? null : <EnrollmentBadge value={full.enrollment} />}
            <FreshnessBadge value={full.heartbeatFreshness} />
            {full.duplicateHardware ? <Badge variant="destructive">Duplicate hardware</Badge> : null}
            {full.counts.uncertainPlacementCount > 0 ? (
              <Badge title="Unconfirmed jobs keep this machine from taking new ones. Open it to repair them." variant="destructive">
                Blocked · {full.counts.uncertainPlacementCount} unconfirmed
              </Badge>
            ) : null}
          </div>
        ) : dash}
      </td>
      <td className="hidden px-3 py-2.5 text-muted-foreground md:table-cell">{full ? lifecycleLabel[full.lifecycle] : dash}</td>
      <td className="hidden px-3 py-2.5 lg:table-cell">{full ? <PressureBadge value={full.pressure} /> : dash}</td>
      <td className="tabular hidden px-3 py-2.5 text-right text-muted-foreground lg:table-cell">{full ? full.backendCount : dash}</td>
      <td className="hidden px-3 py-2.5 font-mono text-xs text-muted-foreground xl:table-cell">{full?.agentVersion ?? dash}</td>
      <td className="hidden px-4 py-2.5 text-right text-muted-foreground md:table-cell">
        {full?.lastHeartbeatAt ? <time dateTime={full.lastHeartbeatAt} title={full.lastHeartbeatAt}>{formatRelativeTime(full.lastHeartbeatAt)}</time> : full ? "Never" : dash}
      </td>
    </tr>
  );
}
