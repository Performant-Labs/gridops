import { Link } from "@tanstack/react-router";
import { ArrowRight } from "lucide-react";
import type { ReactNode } from "react";

import { listRowClassName } from "~/components/page";
import { StatusDot } from "~/components/status-icon";
import { cn, formatDateTime } from "~/lib/utils";

import type { OverviewAlert } from "./types";

const machinePage = /^\/fleet\/machines\/([^/]+)$/;

/**
 * The alert's detail, followed by when it lifts by itself when the server says.
 * `formatTime` renders the moment in the reader's time.
 */
export function alertDetail(alert: OverviewAlert, formatTime: (iso: string) => string): string {
  return alert.until ? `${alert.detail} Reconsidered at ${formatTime(alert.until)}, or sooner if a start on it succeeds.` : alert.detail;
}

/** A link to the alert's page; a machine's alert goes to that machine. */
function AlertLink({ children, className, href }: { children: ReactNode; className: string; href: OverviewAlert["href"] }) {
  const machine = machinePage.exec(href)?.[1];
  if (machine) {
    return <Link className={className} params={{ hostId: decodeURIComponent(machine) }} to="/fleet/machines/$hostId">{children}</Link>;
  }
  return <Link className={className} to={href as Exclude<OverviewAlert["href"], `/fleet/machines/${string}`>}>{children}</Link>;
}

/** The Overview's "Needs attention" list. */
export function AttentionAlerts({ alerts }: { alerts: OverviewAlert[] }) {
  return (
    <div className="overflow-hidden rounded-lg border border-border">
      <div className="flex h-9 items-center gap-2 border-b border-border bg-panel-subtle px-4 text-xs font-medium">
        Needs attention<span className="tabular text-faint">{alerts.length}</span>
      </div>
      {alerts.length ? alerts.map((alert) => (
        <AlertLink className={cn(listRowClassName, "items-start py-3 last:border-b-0")} href={alert.href} key={`${alert.href}-${alert.title}`}>
          <StatusDot className="mt-1.5" tone={alert.level === "error" ? "danger" : "warning"} />
          <span className="min-w-0 flex-1">
            <span className="block font-medium">{alert.title}</span>
            <span className="mt-0.5 block text-xs leading-5 text-muted-foreground">{alertDetail(alert, formatDateTime)}</span>
          </span>
          <ArrowRight className="mt-1 size-3.5 shrink-0 text-faint" />
        </AlertLink>
      )) : (
        <div className="flex items-center gap-2 px-4 py-3 text-sm text-muted-foreground"><StatusDot tone="success" />No active runner-control incidents.</div>
      )}
    </div>
  );
}
