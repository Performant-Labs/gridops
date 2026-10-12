/** Shared labels and small status pieces for the machines views. */
import { Badge } from "~/components/ui/badge";
import { FleetRequestError } from "./registry-api";

type Freshness = "fresh" | "stale" | "offline" | "unknown";

const freshness: Record<Freshness, { label: string; variant: "success" | "warning" | "destructive" | "outline" }> = {
  fresh: { label: "Online", variant: "success" },
  stale: { label: "Stale", variant: "warning" },
  offline: { label: "Offline", variant: "destructive" },
  unknown: { label: "No heartbeat", variant: "outline" },
};

export function FreshnessBadge({ value }: { value: Freshness }) {
  const entry = freshness[value];
  return <Badge dot={value === "unknown" ? "bg-neutral" : undefined} variant={entry.variant}>{entry.label}</Badge>;
}

export function EnrollmentBadge({ value }: { value: "pending" | "approved" | "revoked" }) {
  if (value === "pending") return <Badge variant="warning">Pending approval</Badge>;
  if (value === "revoked") return <Badge variant="destructive">Revoked</Badge>;
  return <Badge variant="success">Approved</Badge>;
}

export function PressureBadge({ value }: { value: "normal" | "pressured" | "unknown" }) {
  if (value === "pressured") return <Badge variant="warning">Under pressure</Badge>;
  if (value === "unknown") return <Badge dot="bg-neutral" variant="outline">Unknown</Badge>;
  return <Badge variant="success">Normal</Badge>;
}

export const lifecycleLabel: Record<string, string> = {
  active: "Active", paused: "Paused", draining: "Draining", maintenance: "Maintenance", retired: "Retired",
};
export const osLabel: Record<string, string> = { linux: "Linux", macos: "macOS", windows: "Windows" };
export const runtimeLabel: Record<string, string> = { docker: "Docker", native_process: "Native process", tart_vm: "Tart VM" };

export function platformLabel(os: string, architecture: string) {
  return `${osLabel[os] ?? os} · ${architecture}`;
}

export function formatBytes(bytes: number) {
  if (bytes <= 0) return "0 B";
  const units = ["B", "KB", "MB", "GB", "TB"];
  const exponent = Math.min(units.length - 1, Math.floor(Math.log(bytes) / Math.log(1024)));
  const value = bytes / 1024 ** exponent;
  return `${value.toLocaleString(undefined, { maximumFractionDigits: value < 10 ? 1 : 0 })} ${units[exponent]}`;
}

export function formatCores(millis: number) {
  return `${(millis / 1000).toLocaleString(undefined, { maximumFractionDigits: 2 })} cores`;
}

export function formatMib(mib: number) {
  return mib >= 1024 ? `${(mib / 1024).toLocaleString(undefined, { maximumFractionDigits: 1 })} GB` : `${mib} MB`;
}

export function fleetErrorMessage(error: unknown, fallback: string) {
  if (error instanceof FleetRequestError) {
    if (error.code === "revision_conflict") return "The machine changed since this page loaded. Try again.";
    if (error.status === 403) return "Only system administrators can do this.";
    return error.message;
  }
  return error instanceof Error ? error.message : fallback;
}

export const isForbidden = (error: unknown) => error instanceof FleetRequestError && error.status === 403;

export const newIdempotencyKey = () => crypto.randomUUID();

