import { useMatch, useRouter } from "@tanstack/react-router";
import { useCallback, useEffect, useState } from "react";

import { signalRefresh, useRefreshInterval } from "~/lib/use-refresh-interval";

export type LiveRouteRefresh = {
  /** False when the page asked not to refresh (signed out, a finished run). */
  enabled: boolean;
  /** The Refresh setting is Off: the page only updates when asked. */
  off: boolean;
  /** When this page's data last loaded, in epoch milliseconds. */
  lastUpdatedAt: number;
  refreshing: boolean;
  /** Reloads the page, its queries and any section that loads its own data. */
  refresh: () => Promise<void>;
};

/**
 * Reloads the current route's data on an interval while the tab is visible,
 * and once when the tab becomes visible again. `baseMilliseconds` is the
 * interval at the default setting; the Refresh preference scales it, and Off
 * stops it (see `scaleRefreshInterval`).
 */
export function useLiveRouteRefresh(baseMilliseconds: number, enabled = true): LiveRouteRefresh {
  const router = useRouter();
  const intervalMilliseconds = useRefreshInterval(baseMilliseconds);
  const updatedAt = useMatch({ strict: false, select: (match) => match.updatedAt });
  const [mountedAt] = useState(() => Date.now());
  const [refreshing, setRefreshing] = useState(false);

  useEffect(() => {
    if (!enabled || intervalMilliseconds === null) return undefined;
    let refreshing = false;
    let cancelled = false;

    async function refresh() {
      if (cancelled || refreshing || document.visibilityState === "hidden") return;
      refreshing = true;
      try {
        await router.invalidate().catch(() => undefined);
      } finally {
        refreshing = false;
      }
    }

    function handleVisibilityChange() {
      if (document.visibilityState === "visible") void refresh();
    }

    const interval = window.setInterval(() => void refresh(), intervalMilliseconds);
    document.addEventListener("visibilitychange", handleVisibilityChange);
    return () => {
      cancelled = true;
      window.clearInterval(interval);
      document.removeEventListener("visibilitychange", handleVisibilityChange);
    };
  }, [enabled, intervalMilliseconds, router]);

  const refresh = useCallback(async () => {
    setRefreshing(true);
    signalRefresh();
    try {
      await Promise.all([
        router.invalidate().catch(() => undefined),
        router.options.context?.queryClient?.refetchQueries({ type: "active" }).catch(() => undefined),
      ]);
    } finally {
      setRefreshing(false);
    }
  }, [router]);

  return {
    enabled,
    off: intervalMilliseconds === null,
    lastUpdatedAt: updatedAt || mountedAt,
    refreshing,
    refresh,
  };
}
