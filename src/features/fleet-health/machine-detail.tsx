import { useId } from "react";
import { Bar, BarChart, ResponsiveContainer, XAxis, YAxis } from "recharts";

import { StatusDot } from "~/components/status-icon";
import { formatBytes, formatCores, formatMib } from "~/features/fleet/fleet-ui";
import { cn, formatRelativeTime } from "~/lib/utils";
import { TRACK_COLOR, bandColor, categoryColor, categoryLabel, scoreBand } from "./chart-theme";
import { MachineWindowSection } from "./machine-detail-window";
import { JOB_CATEGORIES, type FleetHealthViewProps, type HostHealth } from "./types";

/**
 * One machine's detail (#202), shown in the expanded row of the machine score table: what the
 * machine is doing now (runners by category, CPU and memory against their budgets, disk, start
 * failures) and what it ran in the window (CPU-hours by category, the per-category table, its
 * slowest job keys). The page also renders it as its own slot without a `host`: there it names the
 * machine expanded in the table (`expandedId`, #241), whose detail stays in its row, or asks for
 * one to be expanded when none is (or the id is not a machine in the data).
 */
export function MachineDetail({ data, expandedId, host, window: windowKey }: FleetHealthViewProps & { host?: HostHealth; expandedId?: string | null }) {
  if (!host) {
    const expanded = expandedId == null ? undefined : data.hosts.find((candidate) => candidate.id === expandedId);
    return (
      <section aria-label="Machine detail" className={cn("rounded-lg border px-4 py-5", expanded ? "border-border" : "border-dashed border-border")}>
        <h2 className="text-sm font-medium text-foreground">Machine detail</h2>
        <p className="mt-1 text-sm text-muted-foreground">
          {expanded ? (
            <>
              Showing <span className="font-medium text-foreground">{expanded.name}</span>: what it is doing now and what it
              ran in the window are in its row above.
            </>
          ) : (
            "Expand a machine above to see what it is doing now and what it ran in the window."
          )}
        </p>
      </section>
    );
  }
  return (
    <div className="space-y-5" data-machine-detail={host.id}>
      <ScoreSection host={host} />
      <NowSection host={host} />
      <MachineWindowSection host={host} window={windowKey} />
    </div>
  );
}

/**
 * The machine's score components, each with its score and the sentence behind it (#280): the
 * row's small bars carry these only in a tooltip, which a phone cannot show.
 */
function ScoreSection({ host }: { host: HostHealth }) {
  return (
    <section aria-label="Score" className="space-y-3">
      <h3 className="text-xs font-medium uppercase tracking-wide text-faint">Score</h3>
      <ul className="grid gap-x-8 gap-y-3 sm:grid-cols-2">
        {host.components.map((component) => {
          const insufficient = component.score === null;
          return (
            <li className="min-w-0" data-detail-component={component.key} key={component.key}>
              <div className="flex items-baseline gap-2">
                <span className={cn("min-w-0 text-sm font-medium", insufficient ? "text-muted-foreground" : "text-foreground")}>{component.label}</span>
                <span className="tabular ml-auto shrink-0 text-xs text-muted-foreground">{insufficient ? "—" : Math.round(component.score ?? 0)}</span>
              </div>
              <div className="mt-1 h-1.5 w-full overflow-hidden rounded-full" style={{ backgroundColor: TRACK_COLOR }}>
                <div className="h-full rounded-full" style={{ backgroundColor: bandColor(scoreBand(component.score)), width: `${component.score ?? 0}%` }} />
              </div>
              <p className="mt-1 text-xs leading-4 text-muted-foreground">
                {component.deduction ?? (insufficient ? "Not enough data" : "On target")}
              </p>
            </li>
          );
        })}
      </ul>
    </section>
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
 * A resource's reservation, its latest used reading and its budget as four segments that never
 * overlap, in the order a stacked bar draws them: used within the reservation, used past it,
 * reserved but not used, and the budget left free. `usedWithin + reservedUnused` is always the
 * reservation, so it stays readable as a width even when used is larger (#241); the four add up
 * to the demand or the budget, whichever is larger. `scale` is the bar's axis, at least 1 so a
 * machine with no budget divides by nothing.
 */
export function resourceSegments({ budget, reserved, used }: { budget: number; reserved: number; used: number | null }) {
  const usedNow = used ?? 0;
  return {
    usedWithin: Math.min(usedNow, reserved),
    usedOver: Math.max(usedNow - reserved, 0),
    reservedUnused: Math.max(reserved - usedNow, 0),
    free: Math.max(budget - Math.max(reserved, usedNow), 0),
    scale: Math.max(budget, reserved, usedNow, 1),
  };
}

/**
 * Reserved and used against the budget, as one horizontal stacked bar of `resourceSegments`: used
 * in the info colour, used past the reservation hatched in the warning colour, the unused rest of
 * the reservation in a lighter info, then the free budget. The line above it carries the numbers.
 */
function ResourceBar({ budget, format, label, reserved, used }: {
  budget: number;
  format: (value: number) => string;
  label: string;
  reserved: number;
  used: number | null;
}) {
  // CPU and memory draw in the same detail, so each chart's hatch needs its own id.
  const hatchId = `resource-over-${useId()}`;
  const { scale, ...segments } = resourceSegments({ budget, reserved, used });
  const summary = `${format(reserved)} reserved${used === null ? "" : ` · ${format(used)} used`} of ${format(budget)}`;
  return (
    <div className="space-y-1">
      <div className="flex flex-wrap items-baseline justify-between gap-x-3 gap-y-0.5 text-xs">
        <span className="font-medium text-foreground">{label}</span>
        <span className="tabular text-muted-foreground">{summary}</span>
      </div>
      <div aria-label={`${label}: ${summary}`} className="h-2 w-full overflow-hidden rounded-full" role="img">
        <ResponsiveContainer height="100%" width="100%">
          <BarChart accessibilityLayer={false} barCategoryGap={0} data={[{ name: label, ...segments }]} layout="vertical" margin={{ bottom: 0, left: 0, right: 0, top: 0 }}>
            <defs>
              <pattern height="4" id={hatchId} patternTransform="rotate(45)" patternUnits="userSpaceOnUse" width="4">
                <rect fill="var(--warning)" height="4" width="4" />
                <rect fill="var(--panel)" fillOpacity={0.45} height="4" width="1.5" />
              </pattern>
            </defs>
            <XAxis domain={[0, scale]} hide type="number" />
            <YAxis dataKey="name" hide type="category" />
            <Bar dataKey="usedWithin" fill="var(--info)" isAnimationActive={false} name="Used" stackId="resource" />
            <Bar dataKey="usedOver" fill={`url(#${hatchId})`} isAnimationActive={false} name="Used over reservation" stackId="resource" />
            <Bar dataKey="reservedUnused" fill="var(--info)" fillOpacity={0.45} isAnimationActive={false} name="Reserved, not used" stackId="resource" />
            <Bar dataKey="free" fill="var(--selected)" isAnimationActive={false} name="Free budget" stackId="resource" />
          </BarChart>
        </ResponsiveContainer>
      </div>
    </div>
  );
}
