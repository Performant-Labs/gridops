import { Server } from "lucide-react";
import { useEffect, useState } from "react";
import { Area, AreaChart, CartesianGrid, Line, LineChart, ResponsiveContainer, Tooltip as ChartTooltip, XAxis, YAxis } from "recharts";

import { SectionHeading } from "~/components/page";
import { cn, formatDateTime } from "~/lib/utils";
import { getFleetWorkload } from "./admin-api";
import type { FleetWorkload, WorkloadWindow } from "./admin-contracts";
import { FleetRequestError } from "./registry-api";

type Metric = "runners" | "cpu" | "memory";

const METRICS: { id: Metric; label: string }[] = [
  { id: "runners", label: "Runners" },
  { id: "cpu", label: "CPU" },
  { id: "memory", label: "Memory" },
];
const COLORS = ["var(--info)", "var(--progress)", "var(--success)", "var(--warning)", "var(--danger)", "var(--neutral)"];

/**
 * How busy each fleet machine has been: runners on it, CPU load and memory in
 * use. Only administrators can see machines, so it renders nothing for others.
 */
export function FleetWorkloadSection() {
  const [window_, setWindow] = useState<WorkloadWindow>("24h");
  const [metric, setMetric] = useState<Metric>("runners");
  const [workload, setWorkload] = useState<FleetWorkload | null>(null);
  const [hidden, setHidden] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [hoverIndex, setHoverIndex] = useState<number | null>(null);

  useEffect(() => {
    const controller = new AbortController();
    async function load() {
      try {
        setWorkload(await getFleetWorkload(window_, controller.signal));
        setError(null);
      } catch (cause) {
        if (controller.signal.aborted) return;
        if (cause instanceof FleetRequestError && (cause.status === 401 || cause.status === 403)) setHidden(true);
        else setError(cause instanceof Error ? cause.message : "Could not load fleet workload.");
      }
    }
    void load();
    const interval = window_ === "24h" ? globalThis.setInterval(() => void load(), 60_000) : undefined;
    return () => {
      controller.abort();
      if (interval !== undefined) globalThis.clearInterval(interval);
    };
  }, [window_]);

  if (hidden || (workload !== null && workload.machines.length === 0)) return null;

  const machines = workload?.machines ?? [];
  const data = (workload?.points ?? []).map((point) => {
    const row: Record<string, number | null> = { recordedAt: point.recordedAt };
    machines.forEach((machine, index) => {
      const load = point.machines[index];
      row[machine.id] = metric === "runners" ? load?.runners ?? 0 : metric === "cpu" ? load?.cpuPercent ?? null : load?.memoryPercent ?? null;
    });
    return row;
  });
  const current = (hoverIndex === null ? undefined : data[hoverIndex]) ?? data.at(-1);
  const percent = metric !== "runners";
  const format = (value: number | null | undefined) =>
    value === null || value === undefined ? "—" : percent ? `${Math.round(value)}%` : String(value);
  const tick = (value: number) => {
    const date = new Date(value);
    return window_ === "24h"
      ? date.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" })
      : date.toLocaleDateString([], { day: "numeric", month: "short" });
  };
  const axes = (
    <>
      <CartesianGrid stroke="var(--border)" vertical={false} />
      <XAxis axisLine={false} dataKey="recordedAt" minTickGap={48} tick={{ fill: "var(--faint)", fontSize: 11 }} tickFormatter={tick} tickLine={false} />
      <YAxis allowDecimals={!percent} axisLine={false} tick={{ fill: "var(--faint)", fontSize: 11 }} tickFormatter={(value: number) => (percent ? `${value}%` : String(value))} tickLine={false} tickMargin={4} width={40} />
      {/* The legend above shows the hovered values, so only the cursor line is drawn here. */}
      <ChartTooltip content={() => null} cursor={{ stroke: "var(--border-strong)" }} isAnimationActive={false} />
    </>
  );
  const pointer = {
    margin: { bottom: 0, left: 0, right: 12, top: 4 },
    onMouseLeave: () => setHoverIndex(null),
    onMouseMove: (state: { activeTooltipIndex?: unknown }) => {
      const index = Number(state.activeTooltipIndex);
      setHoverIndex(Number.isInteger(index) ? index : null);
    },
  };

  return (
    <section className="space-y-3">
      <SectionHeading
        actions={
          <div className="flex flex-wrap gap-2">
            <Segmented label="Metric" onChange={setMetric} options={METRICS} value={metric} />
            <Segmented
              label="Period"
              onChange={(value) => { setHoverIndex(null); setWindow(value); }}
              options={(["24h", "7d", "30d"] as const).map((id) => ({ id, label: id }))}
              value={window_}
            />
          </div>
        }
      >
        Fleet workload
      </SectionHeading>
      <div className="rounded-lg border border-border">
        <div className="flex flex-wrap items-center gap-x-5 gap-y-1 border-b border-border px-4 py-2.5 text-xs text-muted-foreground">
          {machines.map((machine, index) => (
            <span className="flex items-center gap-1.5" key={machine.id}>
              <span className="size-2 rounded-full" style={{ backgroundColor: COLORS[index % COLORS.length] }} />
              {machine.name}
              <span className="tabular font-medium text-foreground">{format(current?.[machine.id])}</span>
            </span>
          ))}
          {error ? <span className="ml-auto text-danger">{error}</span> : current ? (
            <span className="tabular ml-auto text-faint">
              {hoverIndex === null ? "Latest · " : null}{formatDateTime(new Date(Number(current.recordedAt)).toISOString())}
            </span>
          ) : null}
        </div>
        {data.length > 0 ? (
          <div className="h-60 w-full px-2 pb-2 pt-4">
            <ResponsiveContainer height="100%" width="100%">
              {metric === "runners" ? (
                <AreaChart data={data} {...pointer}>
                  {axes}
                  {machines.map((machine, index) => (
                    <Area
                      dataKey={machine.id}
                      fill={COLORS[index % COLORS.length]}
                      fillOpacity={0.18}
                      isAnimationActive={false}
                      key={machine.id}
                      name={machine.name}
                      stackId="runners"
                      stroke={COLORS[index % COLORS.length]}
                      strokeWidth={1.5}
                      type="monotone"
                    />
                  ))}
                </AreaChart>
              ) : (
                <LineChart data={data} {...pointer}>
                  {axes}
                  {machines.map((machine, index) => (
                    <Line
                      connectNulls
                      dataKey={machine.id}
                      dot={false}
                      isAnimationActive={false}
                      key={machine.id}
                      name={machine.name}
                      stroke={COLORS[index % COLORS.length]}
                      strokeWidth={1.5}
                      type="monotone"
                    />
                  ))}
                </LineChart>
              )}
            </ResponsiveContainer>
          </div>
        ) : (
          <div className="grid h-60 place-items-center px-6 text-center">
            <div>
              <Server className="mx-auto size-5 text-faint" />
              <p className="mt-3 text-sm font-medium">{error ? "Fleet workload is unavailable" : "Loading fleet workload…"}</p>
            </div>
          </div>
        )}
        <p className="border-t border-border px-4 py-2 text-xs text-faint">
          {metric === "runners"
            ? "Runners on each machine, stacked, averaged over each interval; the latest point is right now."
            : metric === "cpu"
              ? "Load as a share of each machine's CPUs; above 100% means work waited for a CPU."
              : "Memory in use as a share of each machine's memory."}
        </p>
      </div>
    </section>
  );
}

function Segmented<T extends string>({ label, onChange, options, value }: {
  label: string;
  onChange: (value: T) => void;
  options: readonly { id: T; label: string }[];
  value: T;
}) {
  return (
    <div aria-label={label} className="flex rounded-md border border-border-strong p-0.5" role="group">
      {options.map((option) => (
        <button
          aria-pressed={value === option.id}
          className={cn("h-6 rounded-[4px] px-2 text-xs font-medium transition-colors", value === option.id ? "bg-selected text-foreground" : "text-muted-foreground hover:text-foreground")}
          key={option.id}
          onClick={() => onChange(option.id)}
          type="button"
        >
          {option.label}
        </button>
      ))}
    </div>
  );
}
