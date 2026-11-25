import { Link } from "@tanstack/react-router";
import { ArrowUpRight } from "lucide-react";

import { StatusDot } from "~/components/status-icon";
import { useFleetHealth } from "~/features/fleet-health/api";
import { cn } from "~/lib/utils";
import { metricTiles } from "./metric-tiles";
import type { DashboardOverview } from "./types";

/**
 * The Overview's row of metric tiles, with the Fleet health tile for an administrator. Fleet
 * health is administrators only, so it is asked for only when the viewer is one (`admin`): a
 * member's Overview sends no request and logs no 403 (#256). A failure still leaves the strip
 * without the tile.
 */
export function MetricStrip({ data, admin }: { data: DashboardOverview; admin: boolean }) {
  const health = useFleetHealth("24h", { enabled: data.authenticated && admin });
  const metrics = metricTiles(data.metrics, health.data);
  return (
    <section aria-label="Runner metrics" className={cn("grid grid-cols-2 gap-px overflow-hidden rounded-lg border border-border bg-border", metrics.length > 5 ? "xl:grid-cols-6" : "xl:grid-cols-5")}>
      {metrics.map((metric) => (
        <Link className="group flex flex-col gap-1 bg-panel px-4 py-3.5 transition-colors hover:bg-hover" key={metric.label} to={metric.to}>
          <span className="flex items-center gap-2 text-xs text-muted-foreground">
            <StatusDot pulse={false} tone={metric.tone} />
            <span className="truncate">{metric.label}</span>
            <ArrowUpRight className="ml-auto size-3.5 shrink-0 text-faint opacity-0 transition-opacity group-hover:opacity-100" />
          </span>
          <span className="tabular text-2xl font-semibold tracking-tight text-foreground">{metric.value}</span>
          <span className={cn("text-xs", metric.tone === "danger" ? "text-danger" : "text-faint")}>{metric.hint}</span>
        </Link>
      ))}
    </section>
  );
}
