import { Area, AreaChart, CartesianGrid, Legend, ResponsiveContainer, Tooltip as ChartTooltip, XAxis, YAxis } from "recharts";

import { categoryColor, categoryLabel } from "./chart-theme";
import { cpuSize, formatMs, windowPhrase } from "./machine-format";
import { JOB_CATEGORIES, type HealthWindow, type HostHealth, type JobCategory } from "./types";

/**
 * What a machine ran in the window (#202): CPU-hours by category over time as a stacked area
 * chart, the per-category table (jobs, median duration, p90 queue wait) and the job keys behind
 * its speed index — this machine's median against the fleet's.
 */
export function MachineWindowSection({ host, window: windowKey }: { host: HostHealth; window: HealthWindow }) {
  return (
    <section aria-label="In the window" className="space-y-4">
      <h3 className="text-xs font-medium uppercase tracking-wide text-faint">In the window · {windowPhrase(windowKey)}</h3>
      <UsageChart host={host} window={windowKey} />
      <CategoryTable host={host} />
      <SlowestKeys host={host} />
    </section>
  );
}

type UsagePoint = { hour: number } & Partial<Record<JobCategory, number>>;

/** The host's hourly CPU-hours as chart points, one key per category the host spent time on. */
export function usagePoints(host: HostHealth): { categories: JobCategory[]; points: UsagePoint[] } {
  const categories = JOB_CATEGORIES.filter((category) => host.usage.some((row) => (row.cpuHours[category] ?? 0) > 0));
  const points = host.usage.map((row) => {
    const point: UsagePoint = { hour: row.hour };
    for (const category of categories) point[category] = row.cpuHours[category] ?? 0;
    return point;
  });
  return { categories, points };
}

const tickLabel = (millis: number, window: HealthWindow) =>
  window === "7d"
    ? new Date(millis).toLocaleDateString([], { day: "numeric", month: "short" })
    : new Date(millis).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });

/** CPU-hours by category over the window, stacked (Recharts, the page's chart rules). */
function UsageChart({ host, window: windowKey }: { host: HostHealth; window: HealthWindow }) {
  const { categories, points } = usagePoints(host);
  if (categories.length === 0) {
    return <p className="text-sm text-muted-foreground">No CPU-hours recorded in the window.</p>;
  }
  return (
    <div className="h-44 w-full" data-testid="machine-usage-chart">
      <ResponsiveContainer height="100%" width="100%">
        <AreaChart data={points} margin={{ bottom: 0, left: 0, right: 8, top: 4 }}>
          <CartesianGrid stroke="var(--border)" vertical={false} />
          <XAxis
            axisLine={false}
            dataKey="hour"
            minTickGap={40}
            tick={{ fill: "var(--faint)", fontSize: 11 }}
            tickFormatter={(hour: number) => tickLabel(hour, windowKey)}
            tickLine={false}
          />
          <YAxis
            allowDecimals
            axisLine={false}
            tick={{ fill: "var(--faint)", fontSize: 11 }}
            tickFormatter={(value: number) => `${value} h`}
            tickLine={false}
            width={40}
          />
          <ChartTooltip
            formatter={(value) => `${value} h`}
            isAnimationActive={false}
            labelFormatter={(hour) => new Date(Number(hour)).toLocaleString([], { day: "numeric", month: "short", hour: "2-digit", minute: "2-digit" })}
          />
          <Legend iconSize={8} wrapperStyle={{ fontSize: 11 }} />
          {categories.map((category) => (
            <Area
              dataKey={category}
              fill={categoryColor(category)}
              fillOpacity={0.3}
              isAnimationActive={false}
              key={category}
              name={categoryLabel(category)}
              stackId="cpu"
              stroke={categoryColor(category)}
              strokeWidth={1.5}
              type="monotone"
            />
          ))}
        </AreaChart>
      </ResponsiveContainer>
    </div>
  );
}

/** Jobs, median duration and p90 queue wait per category, in the fixture's order. */
function CategoryTable({ host }: { host: HostHealth }) {
  if (host.categories.length === 0) {
    return <p className="text-sm text-muted-foreground">No jobs completed on this machine in the window.</p>;
  }
  return (
    <div className="rounded-lg border border-border bg-panel">
      <table className="w-full text-sm">
        <thead className="border-b border-border text-left text-xs text-muted-foreground">
          <tr>
            <th className="px-4 py-2 font-medium" scope="col">Category</th>
            <th className="px-3 py-2 text-right font-medium" scope="col">Jobs</th>
            <th className="px-3 py-2 text-right font-medium" scope="col">Median</th>
            <th className="px-4 py-2 text-right font-medium" scope="col">p90 wait</th>
          </tr>
        </thead>
        <tbody>
          {host.categories.map((entry) => (
            <tr className="border-b border-border last:border-0" data-category={entry.category} key={entry.category}>
              <td className="px-4 py-2">
                <span className="inline-flex items-center gap-1.5">
                  <span aria-hidden="true" className="size-2 rounded-full" style={{ backgroundColor: categoryColor(entry.category) }} />
                  {categoryLabel(entry.category)}
                </span>
              </td>
              <td className="px-3 py-2 text-right tabular-nums">{entry.jobs}</td>
              <td className="px-3 py-2 text-right tabular-nums">{formatMs(entry.medianDurationMs)}</td>
              <td className="px-4 py-2 text-right tabular-nums">{formatMs(entry.p90QueueWaitMs)}</td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}

type SlowKey = { cpuMillis: number; fleetMedianMs: number; fleetRuns: number; hostMedianMs: number; hostRuns: number; key: string };

/** The job keys that push the machine's speed index up most: up to five, slowest first. */
export function slowestKeys(host: HostHealth): SlowKey[] {
  return host.speed
    .flatMap((size) => size.topKeys.map((entry) => ({ ...entry, cpuMillis: size.cpuMillis })))
    .sort((a, b) => b.hostMedianMs / b.fleetMedianMs - a.hostMedianMs / a.fleetMedianMs)
    .slice(0, 5);
}

/**
 * This machine's median against the fleet's, with the run counts behind each median. The keys are
 * the speed index's `topKeys`: only keys slower here than on the rest of the fleet (a ratio above
 * 1.0), so a machine with an index at or below the fleet's median may have none, which is said as
 * such; "not enough data" is only for a machine with no index at any runner size.
 */
function SlowestKeys({ host }: { host: HostHealth }) {
  const keys = slowestKeys(host);
  const indexed = host.speed.some((size) => size.index !== null);
  return (
    <section aria-label="Slowest job keys" className="space-y-2">
      <h4 className="text-xs font-medium text-muted-foreground">Slowest job keys — this machine's median against the fleet's</h4>
      {keys.length === 0 ? (
        <p className="text-sm text-muted-foreground">
          {indexed ? "No job runs slower here than on the rest of the fleet." : "Not enough data: fewer than 3 job keys qualify."}
        </p>
      ) : (
        <ul className="divide-y divide-border rounded-lg border border-border bg-panel">
          {keys.map((entry) => (
            <li className="flex flex-wrap items-baseline justify-between gap-x-4 gap-y-1 px-4 py-2 text-sm" key={`${entry.cpuMillis}:${entry.key}`}>
              <div className="min-w-0">
                <p className="truncate font-medium text-foreground">{entry.key}</p>
                <p className="text-xs text-muted-foreground">at {cpuSize(entry.cpuMillis)}</p>
              </div>
              <p className="tabular text-xs text-muted-foreground">
                {formatMs(entry.hostMedianMs)} vs {formatMs(entry.fleetMedianMs)} fleet · {entry.hostRuns} of {entry.fleetRuns} runs
              </p>
            </li>
          ))}
        </ul>
      )}
    </section>
  );
}
