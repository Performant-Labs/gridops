/**
 * Small formatters shared by the machine score table and the machine detail (#202). The numbers
 * they shape come from `docs/design/fleet-health.md`; times render in the viewer's local time.
 */
import type { HealthWindow, HostHealth, HostSpeed } from "./types";

/** Machines worst score first; a machine with no score last; ties by name. */
export function worstFirst(hosts: HostHealth[]): HostHealth[] {
  return [...hosts].sort((a, b) => {
    if (a.score === null || b.score === null) {
      if (a.score === b.score) return a.name.localeCompare(b.name);
      return a.score === null ? 1 : -1;
    }
    return a.score - b.score || a.name.localeCompare(b.name);
  });
}

/** A duration as "20 s", "4 min 30 s" or "1 h 5 min"; "—" when there is no reading. */
export function formatMs(ms: number | null): string {
  if (ms === null) return "—";
  const totalSeconds = Math.max(0, Math.round(ms / 1000));
  const minutes = Math.floor(totalSeconds / 60);
  const seconds = totalSeconds % 60;
  if (minutes === 0) return `${seconds} s`;
  if (minutes < 60) return seconds === 0 ? `${minutes} min` : `${minutes} min ${seconds} s`;
  const hours = Math.floor(minutes / 60);
  return `${hours} h ${minutes % 60} min`;
}

/** A runner size the way the issue writes it: 1500 CPU millis is "1.5 CPU". */
export function cpuSize(cpuMillis: number): string {
  return `${parseFloat((cpuMillis / 1000).toFixed(2))} CPU`;
}

/** One speed-index line per runner size: "1.42× fleet median at 2 CPU", or "Not enough data". */
export function speedLine(size: HostSpeed): string {
  const at = `at ${cpuSize(size.cpuMillis)}`;
  return size.index === null ? `Not enough data ${at}` : `${size.index.toFixed(2)}× fleet median ${at}`;
}

/** The window as a phrase for headings: "the last 24 hours". */
export function windowPhrase(window: HealthWindow): string {
  const phrases: Record<HealthWindow, string> = { "1h": "the last hour", "24h": "the last 24 hours", "7d": "the last 7 days" };
  return phrases[window];
}

/**
 * The host id a `#machine-<host>` hash points at, or null when the hash is not one (or is
 * malformed). `hash` is the browser's (`#machine-…`, the default) or the router's, which has no
 * leading `#` (`machine-…`).
 */
export function hashHostId(hash: string = globalThis.location?.hash ?? ""): string | null {
  const match = /^#?machine-(.+)$/.exec(hash);
  if (match?.[1] === undefined) return null;
  try {
    return decodeURIComponent(match[1]);
  } catch {
    return null;
  }
}
