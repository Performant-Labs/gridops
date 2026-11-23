/**
 * The fleet health page's one read, `GET /api/v1/fleet/health?window=`, same-origin and uncached,
 * through a private fleet `request` of the same shape as `~/features/fleet/admin-api.ts`: a
 * failure keeps only the fleet error envelope (`FleetRequestError`, whose `status` tells a member's
 * 403 apart), and a body the schema refuses is a `FleetRequestError` too. Administrators only: the
 * page and the Overview tile hide on a 403, as the workload chart does.
 */
import { useEffect, useState } from "react";
import type { z } from "zod";

import { fleetErrorSchema } from "~/features/fleet/contracts";
import { FleetRequestError } from "~/features/fleet/registry-api";
import { useRefreshInterval, useRefreshSignal } from "~/lib/use-refresh-interval";
import { fleetHealthSchema, healthWindowSchema, type FleetHealth, type HealthWindow } from "./types";

async function request<S extends z.ZodType>(path: string, schema: S, signal?: AbortSignal): Promise<z.output<S>> {
  const response = await fetch(path, {
    method: "GET", credentials: "same-origin", cache: "no-store",
    redirect: "error", headers: new Headers({ Accept: "application/json" }), signal,
  });
  if (!response.ok) {
    const parsed = fleetErrorSchema.safeParse(await response.json().catch(() => null));
    throw new FleetRequestError(response.status, parsed.success ? parsed.data : undefined);
  }
  const parsed = schema.safeParse(await response.json().catch(() => null));
  if (!parsed.success) throw new FleetRequestError(response.status);
  return parsed.data;
}

/** The fleet's health over `window`. */
export const fetchFleetHealth = (window: HealthWindow, signal?: AbortSignal): Promise<FleetHealth> =>
  request(`/api/v1/fleet/health?window=${healthWindowSchema.parse(window)}`, fleetHealthSchema, signal);

/** The server caches each window for 30 s; asking more often only returns the same response. */
export const HEALTH_REFRESH_MS = 30_000;

/** Whether a failure is the caller not being an administrator (the page and tile then hide). */
export const isHealthForbidden = (error: unknown) =>
  error instanceof FleetRequestError && (error.status === 401 || error.status === 403);

export type FleetHealthState = {
  /** The last response for this window; kept while a refresh is in flight or fails. */
  data: FleetHealth | undefined;
  /** The last refresh's failure, if it failed. */
  error: unknown;
  /** No response yet for this window, and no failure either. */
  loading: boolean;
  refreshing: boolean;
  /** When `data` arrived (epoch ms), or `null`. */
  updatedAt: number | null;
  refresh: () => void;
};

/**
 * The fleet's health over `window`, reloaded at the user's refresh setting and on a manual
 * Refresh, the way the workload chart loads (`fleet-workload-section.tsx`): component state, no
 * query client, so it can sit in any page. A 401 or 403 stops the reloads: it is the answer for a
 * member, not a blip. `enabled: false` loads nothing (a signed-out Overview).
 */
export function useFleetHealth(window: HealthWindow, { enabled = true }: { enabled?: boolean } = {}): FleetHealthState {
  const interval = useRefreshInterval(HEALTH_REFRESH_MS);
  const refreshSignal = useRefreshSignal();
  const [manual, setManual] = useState(0);
  const [result, setResult] = useState<{ window: HealthWindow; data?: FleetHealth; error?: unknown; updatedAt: number | null }>({ window, updatedAt: null });
  const [refreshing, setRefreshing] = useState(false);

  useEffect(() => {
    if (!enabled) return undefined;
    const controller = new AbortController();
    let timer: ReturnType<typeof globalThis.setTimeout> | undefined;
    async function load() {
      setRefreshing(true);
      let stop = false;
      try {
        const data = await fetchFleetHealth(window, controller.signal);
        setResult({ window, data, updatedAt: Date.now() });
      } catch (error) {
        if (controller.signal.aborted) return;
        stop = isHealthForbidden(error);
        setResult((previous) => ({ ...(previous.window === window ? previous : { updatedAt: null }), window, error }));
      } finally {
        if (!controller.signal.aborted) setRefreshing(false);
      }
      if (!stop && interval !== null && !controller.signal.aborted) timer = globalThis.setTimeout(() => void load(), interval);
    }
    void load();
    return () => {
      controller.abort();
      if (timer !== undefined) globalThis.clearTimeout(timer);
    };
  }, [enabled, interval, manual, refreshSignal, window]);

  const current = result.window === window ? result : { window, updatedAt: null };
  return {
    data: current.data,
    error: current.error,
    loading: enabled && current.data === undefined && current.error === undefined,
    refreshing,
    updatedAt: current.updatedAt,
    refresh: () => setManual((count) => count + 1),
  };
}
