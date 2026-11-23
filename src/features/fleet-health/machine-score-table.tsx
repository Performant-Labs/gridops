import { useEffect, useState } from "react";

import { ScoreGauge } from "~/components/charts/score-gauge";
import { SectionHeading } from "~/components/page";
import { Badge } from "~/components/ui/badge";
import { StatusDot } from "~/components/status-icon";
import { formatRelativeTime } from "~/lib/utils";
import { bandColor, scoreBand, type ScoreBand } from "./chart-theme";
import { MachineDetail } from "./machine-detail";
import { hashHostId, speedLine, worstFirst } from "./machine-format";
import type { FleetHealthViewProps, HostHealth } from "./types";

/**
 * The machine score table (#202): every machine worst score first — a small score gauge and band,
 * its score components as small bars (greyed when there is not enough data), one speed-index line
 * per runner size, and whether it is online. A row expands into its `MachineDetail`; opening the
 * page with `#machine-<host>` expands and scrolls to that machine, which concern links point to.
 */
export function MachineScoreTable({ data, window: windowKey }: FleetHealthViewProps) {
  // A deep link (`#machine-<host>`) opens that machine before anything is clicked.
  const [expanded, setExpanded] = useState<string | null>(() => hashHostId());

  // Scroll the deep-linked machine into view once, after the rows exist. No-op in tests.
  useEffect(() => {
    const target = hashHostId();
    if (target) document.getElementById(`machine-${target}`)?.scrollIntoView?.({ block: "center" });
  }, []);

  const hosts = worstFirst(data.hosts);
  return (
    <section aria-label="Machines">
      <SectionHeading>Machines</SectionHeading>
      <div className="divide-y divide-border overflow-hidden rounded-lg border border-border bg-panel">
        {hosts.length === 0 ? (
          <p className="px-4 py-5 text-sm text-muted-foreground">No machines to score.</p>
        ) : (
          hosts.map((host) => (
            <MachineRow
              data={data}
              host={host}
              key={host.id}
              onToggle={() => setExpanded((current) => (current === host.id ? null : host.id))}
              open={expanded === host.id}
              window={windowKey}
            />
          ))
        )}
      </div>
      <p className="mt-2 text-xs text-faint">Worst score first. Expand a machine for what it is doing now and what it ran in the window.</p>
    </section>
  );
}

const BAND_BADGE: Record<ScoreBand, { label: string; variant: "success" | "warning" | "destructive" | "outline" }> = {
  good: { label: "Good", variant: "success" },
  fair: { label: "Fair", variant: "warning" },
  poor: { label: "Poor", variant: "destructive" },
  none: { label: "No score", variant: "outline" },
};

/** One machine: the summary row (the toggle) and, when open, its detail below it. */
function MachineRow({ data, host, open, onToggle, window: windowKey }: { data: FleetHealthViewProps["data"]; host: HostHealth; open: boolean; onToggle: () => void; window: FleetHealthViewProps["window"] }) {
  const band = scoreBand(host.score);
  const live = host.live;
  return (
    <div id={`machine-${host.id}`}>
      <button
        aria-controls={`detail-${host.id}`}
        aria-expanded={open}
        className="w-full px-4 py-3 text-left transition-colors hover:bg-selected/40"
        onClick={onToggle}
        type="button"
      >
        <div className="flex flex-wrap items-center gap-x-6 gap-y-3">
          <ScoreGauge label={host.name} score={host.score} size={44} />
          <div className="flex min-w-40 flex-1 flex-col gap-1">
            <div className="flex flex-wrap items-center gap-2">
              <span className="text-sm font-medium text-foreground">{host.name}</span>
              <Badge variant={BAND_BADGE[band].variant}>{BAND_BADGE[band].label}</Badge>
            </div>
            <span className="inline-flex items-center gap-1.5 text-xs text-muted-foreground">
              <StatusDot tone={live === null ? "neutral" : live.online ? "success" : "danger"} />
              {live === null ? "Unknown" : live.online ? "Online" : live.lastSeenAt === null ? "Offline" : `Offline · last seen ${formatRelativeTime(new Date(live.lastSeenAt).toISOString())}`}
            </span>
          </div>
          <ComponentBars host={host} />
          <SpeedLines host={host} />
        </div>
      </button>
      {open ? (
        <div className="border-t border-border bg-panel-subtle px-4 py-4" id={`detail-${host.id}`}>
          <MachineDetail data={data} host={host} window={windowKey} />
        </div>
      ) : null}
    </div>
  );
}

/** The machine's score components as small bars, greyed when a component has no score. */
function ComponentBars({ host }: { host: HostHealth }) {
  return (
    <div aria-label="Score components" className="flex items-end gap-1">
      {host.components.map((component) => (
        <span
          className="relative h-7 w-3 overflow-hidden rounded-[3px] bg-selected"
          data-component-key={component.key}
          data-insufficient={component.score === null ? "true" : undefined}
          key={component.key}
          title={`${component.label}: ${component.score === null ? "not enough data" : component.score}`}
        >
          <span
            className="absolute inset-x-0 bottom-0"
            style={{
              backgroundColor: component.score === null ? "var(--neutral)" : bandColor(scoreBand(component.score)),
              height: component.score === null ? "100%" : `${component.score}%`,
              opacity: component.score === null ? 0.35 : 1,
            }}
          />
        </span>
      ))}
    </div>
  );
}

/** One speed-index line per runner size: "1.42× fleet median at 2 CPU". */
function SpeedLines({ host }: { host: HostHealth }) {
  return (
    <div className="min-w-0 text-xs text-muted-foreground">
      {host.speed.length === 0 ? <div>Not enough data</div> : host.speed.map((size) => (
        <div data-speed-cpu={size.cpuMillis} key={size.cpuMillis}>{speedLine(size)}</div>
      ))}
    </div>
  );
}
