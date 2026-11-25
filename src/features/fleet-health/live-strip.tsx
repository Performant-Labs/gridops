import { Link } from "@tanstack/react-router";
import { ArrowRight } from "lucide-react";
import { Bar, BarChart, ResponsiveContainer, Tooltip as ChartTooltip, XAxis, YAxis } from "recharts";

import { categoryColor, categoryLabel } from "./chart-theme";
import { JOB_CATEGORIES, type FleetHealthViewProps, type FleetLiveStats, type JobCategory } from "./types";

/**
 * The live strip (#203): what is queued and running right now, by category
 * (`data-queued-category` / `data-running-category`), the totals (`data-testid="live-queued-total"`
 * and `"live-running-total"`) and the oldest queued job's wait. One horizontal stacked bar per
 * side, the same colours as the category legend and the window charts. Under the queued side, the
 * queue by pool (`data-queued-pool`, each linked to its pool page) and the jobs no pool will run
 * (`data-testid="live-queued-unmatched"`, linked to the queue), as the API sends them (#253).
 */
export function LiveStrip({ data }: FleetHealthViewProps) {
  const live = data.live;
  const scale = Math.max(live.queuedTotal, live.runningTotal);
  const rows = [
    { name: "Queued", ...byCategoryCounts(live.queuedByCategory) },
    { name: "Running", ...byCategoryCounts(live.runningByCategory) },
  ];
  return (
    <section aria-label="Right now" className="rounded-lg border border-border bg-panel">
      <div className="flex flex-wrap items-baseline gap-x-3 gap-y-1 border-b border-border px-4 py-2.5">
        <h2 className="text-sm font-medium text-foreground">Right now</h2>
        <OldestQueuedWait oldest={live.oldestQueued} queuedTotal={live.queuedTotal} />
      </div>
      <div className="space-y-2.5 px-4 py-3">
        <LegendRow attribute="data-queued-category" counts={live.queuedByCategory} label="Queued" testId="live-queued-total" total={live.queuedTotal} />
        <QueuedByPool pools={live.queuedByPool} unmatched={live.queuedUnmatched} />
        {scale === 0 ? (
          <p className="py-4 text-center text-sm text-muted-foreground">Nothing is queued or running right now.</p>
        ) : (
          <div className="h-20 w-full">
            <ResponsiveContainer height="100%" width="100%">
              <BarChart data={rows} layout="vertical" margin={{ bottom: 0, left: 0, right: 8, top: 0 }}>
                <XAxis allowDecimals={false} domain={[0, scale]} hide type="number" />
                <YAxis dataKey="name" hide type="category" />
                {/* The legend rows above and below carry the values, so only the cursor is drawn. */}
                <ChartTooltip content={() => null} cursor={{ fill: "var(--hover)" }} isAnimationActive={false} />
                {JOB_CATEGORIES.map((category) => (
                  <Bar barSize={16} dataKey={category} fill={categoryColor(category)} isAnimationActive={false} key={category} name={categoryLabel(category)} stackId="live" />
                ))}
              </BarChart>
            </ResponsiveContainer>
          </div>
        )}
        <LegendRow attribute="data-running-category" counts={live.runningByCategory} label="Running" testId="live-running-total" total={live.runningTotal} />
      </div>
    </section>
  );
}

/**
 * Where the queued jobs wait: one link per pool with jobs queued for it, in the API's order (most
 * first), then the jobs no pool will run. Nothing when neither has any.
 */
function QueuedByPool({ pools, unmatched }: { pools: FleetLiveStats["queuedByPool"]; unmatched: number }) {
  if (pools.length === 0 && unmatched === 0) return null;
  return (
    <div className="flex flex-wrap items-center gap-x-4 gap-y-1.5 text-xs text-muted-foreground">
      <span>By pool</span>
      {pools.map((pool) => (
        <Link
          className="flex min-w-0 items-center gap-1.5 hover:text-foreground"
          data-queued-pool={pool.poolId}
          key={pool.poolId}
          params={{ poolId: pool.poolId }}
          to="/runner-pools/$poolId"
        >
          <span className="tabular font-medium text-foreground">{pool.queued}</span>
          <span className="max-w-40 truncate">{pool.poolName}</span>
        </Link>
      ))}
      {unmatched === 0 ? null : (
        <Link className="flex items-center gap-1.5 hover:text-foreground" to="/queued-jobs">
          <span data-testid="live-queued-unmatched">
            <span className="tabular font-medium text-danger">{unmatched}</span> {unmatched === 1 ? "job" : "jobs"} with no matching pool
          </span>
        </Link>
      )}
    </div>
  );
}

/** Every category's live count, `0` for the ones the response leaves out, in display order. */
function byCategoryCounts(byCategory: Partial<Record<JobCategory, number>>): Record<JobCategory, number> {
  return Object.fromEntries(JOB_CATEGORIES.map((category) => [category, byCategory[category] ?? 0])) as Record<JobCategory, number>;
}

/**
 * One side of the strip: the side's total next to its name, then one chip per category with jobs,
 * each chip leading with its count, so the chips add up to the total the chart stacks.
 */
function LegendRow({ attribute, counts, label, testId, total }: {
  attribute: "data-queued-category" | "data-running-category";
  counts: Partial<Record<JobCategory, number>>;
  label: string;
  testId: string;
  total: number;
}) {
  return (
    <div className="flex flex-wrap items-center gap-x-4 gap-y-1.5 text-xs text-muted-foreground">
      <span className="flex items-baseline gap-1.5">
        <span className="text-foreground">{label}</span>
        <span className="tabular text-sm font-semibold text-foreground" data-testid={testId}>{total}</span>
      </span>
      {JOB_CATEGORIES.map((category) => {
        const count = counts[category] ?? 0;
        if (count === 0) return null;
        return (
          <span className="flex items-center gap-1.5" key={category} {...{ [attribute]: category }}>
            <span className="size-2 rounded-full" style={{ backgroundColor: categoryColor(category) }} />
            <span className="tabular font-medium text-foreground">{count}</span>
            {categoryLabel(category)}
          </span>
        );
      })}
    </div>
  );
}

/** The oldest queued job's wait, linked to the queue it waits in; nothing when nothing waits. */
function OldestQueuedWait({ oldest, queuedTotal }: { oldest: FleetLiveStats["oldestQueued"]; queuedTotal: number }) {
  if (!oldest) {
    return queuedTotal === 0 ? <span className="ml-auto text-xs text-faint">Queue is clear</span> : null;
  }
  return (
    <Link className="ml-auto flex min-w-0 items-center gap-1 text-xs text-muted-foreground hover:text-foreground" to="/queued-jobs">
      <span className="truncate">
        Oldest job waited <span className="tabular font-medium text-foreground">{formatOldestWait(oldestWaitMs(oldest))}</span> · {oldest.repository} · {oldest.jobName}
      </span>
      <ArrowRight className="size-3 shrink-0" />
    </Link>
  );
}

/**
 * How long the oldest queued job has waited by `now`: the response's `waitMs` was fixed when it
 * was built, so a cached or stale response would understate it, and the clock since `queuedAt`
 * is used once it is longer. A viewer's clock behind the server's leaves `waitMs` standing.
 */
export function oldestWaitMs(oldest: Pick<NonNullable<FleetLiveStats["oldestQueued"]>, "queuedAt" | "waitMs">, now = Date.now()): number {
  return Math.max(oldest.waitMs, now - oldest.queuedAt);
}

/**
 * How long the oldest queued job has waited: whole seconds under a minute ("45 s"), whole minutes
 * under an hour ("12 min"), then hours and minutes ("3 h 5 min", "1 h").
 */
export function formatOldestWait(waitMs: number): string {
  const seconds = Math.floor(waitMs / 1000);
  if (seconds < 60) return `${seconds} s`;
  const minutes = Math.floor(waitMs / 60_000);
  if (minutes < 60) return `${minutes} min`;
  const hours = Math.floor(minutes / 60);
  const rest = minutes % 60;
  return rest === 0 ? `${hours} h` : `${hours} h ${rest} min`;
}
