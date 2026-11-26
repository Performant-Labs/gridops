import { CartesianGrid, Line, LineChart, ResponsiveContainer, Tooltip as ChartTooltip, XAxis, YAxis } from "recharts";

import { ScoreGauge } from "~/components/charts/score-gauge";
import { Tooltip, TooltipProvider } from "~/components/ui/tooltip";
import { cn } from "~/lib/utils";
import { TRACK_COLOR, bandColor, bandTextColor, scoreBand, type ScoreBand } from "./chart-theme";
import type { FleetHealth, FleetHealthViewProps, HealthWindow, ScoreComponent } from "./types";

/** The word under the gauge: which band the score is in. */
const BAND_LABELS: Record<ScoreBand, string> = { good: "Good", fair: "Fair", poor: "Poor", none: "No score" };

/**
 * Why a component with no score is greyed out (`docs/design/fleet-health.md`, Scores), when the
 * API gives no reason of its own in the component's `deduction`.
 */
const INSUFFICIENT_REASON = "Not enough data in this window: fewer than 20 jobs, or no samples.";

/** The local time of a trend point: an hour on the shorter windows, a day on 7 d. */
const trendTick = (value: number, window: HealthWindow) =>
  window === "7d"
    ? new Date(value).toLocaleDateString([], { day: "numeric", month: "short" })
    : new Date(value).toLocaleTimeString([], { hour: "numeric", minute: "2-digit" });

/**
 * The fleet score card (#195): the score in its band (`ScoreGauge` from
 * `~/components/charts/score-gauge`, which carries `data-band`) with the trend from snapshots
 * under it as a small `LineChart`, and one row per component (`data-component-key`, anchored as
 * `#fleet-component-<key>` for its concern's link) beside it:
 * its score as a bar in its band's colour, its weight, and its deduction sentence. A component
 * without a score (`data-insufficient="true"`) is greyed out with a tooltip saying why: its name in
 * the muted colour and an empty bar, never faded with opacity, which took its text below 4.5:1
 * (#256).
 */
export function FleetScoreCard({ data, window }: FleetHealthViewProps) {
  const fleet = data.fleet;
  const band = scoreBand(fleet.score);
  return (
    <section aria-label="Fleet score" className="rounded-lg border border-border bg-panel">
      <h2 className="border-b border-border px-4 py-2.5 text-sm font-medium text-foreground">Fleet score</h2>
      <div className="flex flex-col gap-4 p-4 sm:flex-row sm:gap-5">
        <div className="flex w-full shrink-0 flex-col items-center gap-1 sm:w-44">
          <ScoreGauge label="Fleet score" score={fleet.score} size={132} />
          <span className="text-xs font-medium" style={{ color: bandTextColor(band) }}>{BAND_LABELS[band]}</span>
          <TrendChart score={fleet.score} trend={fleet.trend} window={window} />
        </div>
        <TooltipProvider delayDuration={200}>
          <ul className="min-w-0 flex-1 space-y-3">
            {fleet.components.map((component) => (
              <ComponentRow component={component} key={component.key} />
            ))}
          </ul>
        </TooltipProvider>
      </div>
    </section>
  );
}

/** The score's snapshots as a small line, in the colour of the score's current band. */
function TrendChart({ score, trend, window }: { score: number | null; trend: FleetHealth["fleet"]["trend"]; window: HealthWindow }) {
  if (trend.length === 0) return <p className="pt-1 text-xs text-faint">No trend yet.</p>;
  return (
    <div className="h-24 w-full pt-2" data-testid="fleet-score-trend">
      <ResponsiveContainer height="100%" width="100%">
        {/* The right margin holds half the last time label, which the SVG would otherwise clip (#258). */}
        <LineChart data={trend} margin={{ bottom: 0, left: 0, right: 24, top: 4 }}>
          <CartesianGrid stroke="var(--border)" vertical={false} />
          <XAxis
            axisLine={false}
            dataKey="at"
            minTickGap={24}
            tick={{ fill: "var(--faint)", fontSize: 11 }}
            tickFormatter={(value: number) => trendTick(value, window)}
            tickLine={false}
          />
          <YAxis
            allowDecimals={false}
            axisLine={false}
            domain={[0, 100]}
            tick={{ fill: "var(--faint)", fontSize: 11 }}
            tickFormatter={(value: number) => String(value)}
            tickLine={false}
            width={26}
          />
          <ChartTooltip content={<TrendTooltip window={window} />} cursor={{ stroke: "var(--border-strong)" }} isAnimationActive={false} />
          <Line
            connectNulls
            dataKey="score"
            dot={false}
            isAnimationActive={false}
            name="Fleet score"
            stroke={bandColor(scoreBand(score))}
            strokeWidth={1.5}
            type="monotone"
          />
        </LineChart>
      </ResponsiveContainer>
    </div>
  );
}

/** One snapshot's value and time, in the theme's popover colours. */
function TrendTooltip({ active, label, payload, window }: {
  active?: boolean;
  label?: unknown;
  payload?: { value?: unknown }[];
  window: HealthWindow;
}) {
  if (!active || payload?.length !== 1) return null;
  const score = payload[0]?.value;
  const at = typeof label === "number" ? label : Number(label);
  return (
    <div className="rounded-md border border-border-strong bg-popover px-2 py-1 text-xs text-foreground shadow-popover">
      {typeof score === "number" ? score : "—"} · {Number.isFinite(at) ? trendTick(at, window) : null}
    </div>
  );
}

/**
 * One component: its score as a bar in its band's colour, its weight as a whole percent ("—" when
 * it has no score) and its deduction. A component with no score gives the API's reason (its
 * `deduction`), or the card's own when the API gives none.
 */
function ComponentRow({ component }: { component: ScoreComponent }) {
  const insufficient = component.score === null;
  const reason = component.deduction || INSUFFICIENT_REASON;
  const row = (
    <li
      className={cn("min-w-0 scroll-mt-4", insufficient && "cursor-help")}
      data-component-key={component.key}
      // The anchor a `fleet-component:<key>` concern links to (#253).
      id={`fleet-component-${component.key}`}
      // The reason rides in a tooltip, so the row itself takes focus for keyboard and touch.
      tabIndex={insufficient ? 0 : undefined}
      {...(insufficient ? { "data-insufficient": "true" as const } : {})}
    >
      <div className="flex items-baseline gap-2">
        <span className={cn("min-w-0 truncate text-sm font-medium", insufficient ? "text-muted-foreground" : "text-foreground")}>
          {component.label}
        </span>
        <span className="ml-auto shrink-0 text-xs text-faint tabular">{insufficient ? "—" : `${Math.round(component.weight)}%`}</span>
      </div>
      <div className="mt-1 h-1.5 w-full overflow-hidden rounded-full" style={{ backgroundColor: TRACK_COLOR }}>
        <div
          className="h-full rounded-full"
          style={{ backgroundColor: bandColor(scoreBand(component.score)), width: `${component.score ?? 0}%` }}
        />
      </div>
      <p className={cn("mt-1 text-xs leading-4", insufficient ? "text-faint" : "text-muted-foreground")}>
        {insufficient ? reason : (component.deduction ?? "On target")}
      </p>
    </li>
  );
  return insufficient ? (
    <Tooltip content={reason} side="top">
      {row}
    </Tooltip>
  ) : (
    row
  );
}
