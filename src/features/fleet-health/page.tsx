import { HeartPulse, RefreshCw } from "lucide-react";
import { type ReactNode, useState } from "react";

import { LiveRefreshNotice } from "~/components/live-refresh-notice";
import { Callout, EmptyState, LoadingRows, PageBody, PageHeader } from "~/components/page";
import { Button } from "~/components/ui/button";
import { fleetErrorMessage } from "~/features/fleet/fleet-ui";
import { useRefreshInterval } from "~/lib/use-refresh-interval";
import { cn } from "~/lib/utils";
import { HEALTH_REFRESH_MS, isHealthForbidden, useFleetHealth } from "./api";
import { AreasOfConcern } from "./areas-of-concern";
import { CategoryLegend } from "./category-legend";
import { FleetScoreCard } from "./fleet-score-card";
import { LiveStrip } from "./live-strip";
import { MachineDetail } from "./machine-detail";
import { MachineScoreTable } from "./machine-score-table";
import { HEALTH_WINDOWS, type FleetHealthViewProps, type HealthWindow } from "./types";

/** Where the page remembers the chosen window. */
export const HEALTH_WINDOW_STORAGE_KEY = "gridops.fleetHealth.window";

/**
 * The window the page opens on: the one last chosen, `24h` by default and when storage fails.
 * #197 always opens on `24h`; #195 reads `HEALTH_WINDOW_STORAGE_KEY`.
 */
export function loadHealthWindow(): HealthWindow {
  return "24h";
}

/** Remembers the chosen window (#195 writes `HEALTH_WINDOW_STORAGE_KEY`). */
export function saveHealthWindow(_window: HealthWindow): void {}

const WINDOW_LABELS: Record<HealthWindow, string> = { "1h": "1 hour", "24h": "24 hours", "7d": "7 days" };

/**
 * `/fleet/health`: the fleet's health over a window. The frame (#195) lays out every section and
 * owns the window; each section takes `{ data, window }` and is filled in by its own issue. Only
 * administrators can read it, so a member sees why instead of an error.
 */
export function FleetHealthPage() {
  const [window_, setWindow_] = useState<HealthWindow>(loadHealthWindow);
  const health = useFleetHealth(window_);
  const interval = useRefreshInterval(HEALTH_REFRESH_MS);
  const choose = (next: HealthWindow) => {
    setWindow_(next);
    saveHealthWindow(next);
  };

  return (
    <>
      <PageHeader
        actions={
          <>
            <WindowPicker onChange={choose} value={window_} />
            <Button disabled={health.refreshing} onClick={health.refresh} size="sm" variant="ghost">
              <RefreshCw className={cn(health.refreshing && "animate-spin")} /><span className="hidden sm:inline">Refresh</span>
            </Button>
          </>
        }
        icon={HeartPulse}
        title="Fleet health"
      />
      <LiveRefreshNotice
        lastUpdatedAt={health.updatedAt}
        off={interval === null}
        onRefresh={health.refresh}
        refreshing={health.refreshing}
      />
      <PageBody>
        {health.data ? (
          <HealthSections data={health.data} window={window_} />
        ) : health.loading ? (
          <LoadingRows />
        ) : isHealthForbidden(health.error) ? (
          <EmptyState description="Fleet health covers every machine, so only system administrators can see it." icon={HeartPulse} title="Administrators only" />
        ) : (
          <div className="p-4">
            <Callout action={<Button onClick={health.refresh} size="sm" variant="outline">Retry</Button>} title="Couldn’t load fleet health" tone="danger">
              {fleetErrorMessage(health.error, "Fleet health could not be loaded.")}
            </Callout>
          </div>
        )}
      </PageBody>
    </>
  );
}

/**
 * The page's composition: each section sits in its own slot, marked `data-health-section`, so the
 * page test can check the composition without reading a section's content. Wave 1 fills a slot's
 * content and may move the slot, but keeps its name (`docs/design/fleet-health.md`, "The page").
 */
type HealthSlot = "live-strip" | "areas-of-concern" | "fleet-score" | "category-legend" | "machine-table" | "machine-detail";

function Slot({ name, children }: { name: HealthSlot; children: ReactNode }) {
  return <div className="min-w-0" data-health-section={name}>{children}</div>;
}

function HealthSections({ data, window }: FleetHealthViewProps) {
  return (
    <div className="mx-auto flex max-w-6xl flex-col gap-4 p-3 sm:p-4" data-testid="fleet-health-sections">
      {data.stale ? (
        <Callout title="Showing the last good reading" tone="warning">
          GridOps couldn’t read the fleet just now, so this is the last response it had.
        </Callout>
      ) : null}
      <Slot name="live-strip"><LiveStrip data={data} window={window} /></Slot>
      <div className="grid gap-4 lg:grid-cols-2">
        <Slot name="areas-of-concern"><AreasOfConcern data={data} window={window} /></Slot>
        <Slot name="fleet-score"><FleetScoreCard data={data} window={window} /></Slot>
      </div>
      <Slot name="category-legend"><CategoryLegend data={data} window={window} /></Slot>
      <Slot name="machine-table"><MachineScoreTable data={data} window={window} /></Slot>
      <Slot name="machine-detail"><MachineDetail data={data} window={window} /></Slot>
    </div>
  );
}

function WindowPicker({ onChange, value }: { onChange: (value: HealthWindow) => void; value: HealthWindow }) {
  return (
    <div aria-label="Window" className="flex rounded-md border border-border-strong p-0.5" role="group">
      {HEALTH_WINDOWS.map((option) => (
        <button
          aria-label={WINDOW_LABELS[option]}
          aria-pressed={value === option}
          className={cn("h-6 rounded-[4px] px-2 text-xs font-medium transition-colors", value === option ? "bg-selected text-foreground" : "text-muted-foreground hover:text-foreground")}
          key={option}
          onClick={() => onChange(option)}
          type="button"
        >
          {option}
        </button>
      ))}
    </div>
  );
}
