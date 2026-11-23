import { StatusDot } from "~/components/status-icon";
import { formatBytes, formatCores, formatMib } from "~/features/fleet/fleet-ui";
import { formatRelativeTime } from "~/lib/utils";
import { categoryColor, categoryLabel } from "./chart-theme";
import { MachineWindowSection } from "./machine-detail-window";
import { JOB_CATEGORIES, type FleetHealthViewProps, type HostHealth } from "./types";

/**
 * One machine's detail (#202), shown in the expanded row of the machine score table: what the
 * machine is doing now (runners by category, CPU and memory against their budgets, disk, start
 * failures) and what it ran in the window (CPU-hours by category, the per-category table, its
 * slowest job keys). The page also renders it as its own slot, where no machine is expanded yet.
 */
export function MachineDetail({ host, window: windowKey }: FleetHealthViewProps & { host?: HostHealth }) {
  if (!host) {
    return (
      <section aria-label="Machine detail" className="rounded-lg border border-dashed border-border px-4 py-5">
        <h2 className="text-sm font-medium text-foreground">Machine detail</h2>
        <p className="mt-1 text-sm text-muted-foreground">
          Expand a machine above to see what it is doing now and what it ran in the window.
        </p>
      </section>
    );
  }
  return (
    <div className="space-y-5" data-machine-detail={host.id}>
      <NowSection host={host} />
      <MachineWindowSection host={host} window={windowKey} />
    </div>
  );
}

/** The live half: what the machine is doing at the moment the response was built. */
function NowSection({ host }: { host: HostHealth }) {
  const live = host.live;
  return (
    <section aria-label="Now" className="space-y-3">
      <h3 className="text-xs font-medium uppercase tracking-wide text-faint">Now</h3>
      {live === null ? (
        <p className="text-sm text-muted-foreground">No live reading for this machine.</p>
      ) : (
        <div className="grid gap-x-8 gap-y-3 sm:grid-cols-2">
          <div className="space-y-2">
            <p className="inline-flex items-center gap-1.5 text-sm text-foreground">
              <StatusDot tone={live.online ? "success" : "danger"} />
              {live.online ? "Online" : live.lastSeenAt === null ? "Offline" : `Offline · last seen ${formatRelativeTime(new Date(live.lastSeenAt).toISOString())}`}
            </p>
            <RunningByCategory live={live} />
            <p className="text-sm text-muted-foreground">
              {live.startFailuresLastHour === 0
                ? "No start failures in the last hour."
                : `${live.startFailuresLastHour} start ${live.startFailuresLastHour === 1 ? "failure" : "failures"} in the last hour.`}
            </p>
          </div>
          <div className="space-y-3">
            <ResourceBar budget={host.cpuBudgetMillis} format={formatCores} label="CPU" reserved={live.cpuReservedMillis} used={live.cpuUsedMillis} />
            <ResourceBar budget={host.memoryBudgetMib} format={formatMib} label="Memory" reserved={live.memoryReservedMib} used={live.memoryUsedMib} />
            <p className="text-sm text-muted-foreground">
              Disk free: {live.diskFreeBytes === null ? "unknown" : formatBytes(live.diskFreeBytes)}
              {host.diskTotalBytes === null ? "" : ` of ${formatBytes(host.diskTotalBytes)}`}
            </p>
          </div>
        </div>
      )}
    </section>
  );
}

/** Runners running right now, one chip per category with work. */
function RunningByCategory({ live }: { live: NonNullable<HostHealth["live"]> }) {
  const running = JOB_CATEGORIES.filter((category) => (live.runningByCategory[category] ?? 0) > 0);
  return (
    <div className="flex flex-wrap gap-x-3 gap-y-1 text-sm">
      <span className="text-muted-foreground">Runners running:</span>
      {running.length === 0 ? (
        <span className="text-muted-foreground">none</span>
      ) : (
        running.map((category) => (
          <span className="inline-flex items-center gap-1.5 text-foreground" key={category}>
            <span aria-hidden="true" className="size-2 rounded-full" style={{ backgroundColor: categoryColor(category) }} />
            {categoryLabel(category)} · {live.runningByCategory[category]}
          </span>
        ))
      )}
    </div>
  );
}

/**
 * Reserved and used against the budget, as one horizontal stacked bar (the reservation as the
 * wide segment, the latest used measurement over it), like the fleet's capacity meter.
 */
function ResourceBar({ budget, format, label, reserved, used }: {
  budget: number;
  format: (value: number) => string;
  label: string;
  reserved: number;
  used: number | null;
}) {
  const scale = Math.max(budget, reserved, used ?? 0, 1);
  return (
    <div className="space-y-1">
      <div className="flex flex-wrap items-baseline justify-between gap-x-3 gap-y-0.5 text-xs">
        <span className="font-medium text-foreground">{label}</span>
        <span className="tabular text-muted-foreground">
          {format(reserved)} reserved{used === null ? "" : ` · ${format(used)} used`} of {format(budget)}
        </span>
      </div>
      <div className="relative h-2 overflow-hidden rounded-full bg-selected">
        <span className="absolute inset-y-0 left-0 rounded-full bg-info/45" style={{ width: `${(reserved / scale) * 100}%` }} />
        {used === null ? null : <span className="absolute inset-y-0 left-0 rounded-full bg-info" style={{ width: `${(used / scale) * 100}%` }} />}
      </div>
    </div>
  );
}
