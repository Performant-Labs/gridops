import { PauseCircle, RefreshCw } from "lucide-react";

import { Button } from "~/components/ui/button";
import type { LiveRouteRefresh } from "~/lib/use-live-route-refresh";
import { cn } from "~/lib/utils";

const updatedTimeFormat = new Intl.DateTimeFormat(undefined, { hour: "numeric", minute: "2-digit", second: "2-digit" });

/** The local time of a page's last update, to the second. */
export function formatUpdatedTime(epochMilliseconds: number) {
  return updatedTimeFormat.format(epochMilliseconds);
}

/**
 * Shown on a live page while the Refresh preference is Off, so stale data is
 * obvious: says so, gives the time of the last update and a Refresh button.
 * Renders nothing while automatic refresh is on.
 */
export function LiveRefreshNotice({
  off,
  lastUpdatedAt,
  refreshing,
  onRefresh,
  message,
  className,
}: {
  off: boolean;
  lastUpdatedAt: number | null;
  refreshing: boolean;
  onRefresh: () => unknown;
  /** What does not update on its own here, when it is worth spelling out. */
  message?: string;
  className?: string;
}) {
  if (!off) return null;
  return (
    <div
      className={cn("flex shrink-0 flex-wrap items-center gap-x-3 gap-y-1.5 border-b border-border bg-panel-subtle px-4 py-2 text-xs text-muted-foreground sm:px-6", className)}
    >
      <PauseCircle aria-hidden className="size-3.5 shrink-0 text-faint" />
      <span className="min-w-0">
        <span className="font-medium text-foreground">Automatic refresh is off.</span>{" "}
        {message ?? "Turn it back on in Settings → Preferences."}
      </span>
      <span className="tabular ml-auto" role="status">
        {lastUpdatedAt ? <>Updated <time dateTime={new Date(lastUpdatedAt).toISOString()}>{formatUpdatedTime(lastUpdatedAt)}</time></> : "Not loaded yet"}
      </span>
      <Button aria-busy={refreshing || undefined} disabled={refreshing} onClick={() => void onRefresh()} size="xs" variant="outline">
        <RefreshCw aria-hidden className={cn(refreshing && "animate-spin")} />Refresh
      </Button>
    </div>
  );
}

/** The notice for a page driven by `useLiveRouteRefresh`; hidden while that page is not live. */
export function LiveRouteRefreshNotice({ live, message }: { live: LiveRouteRefresh; message?: string }) {
  return <LiveRefreshNotice lastUpdatedAt={live.lastUpdatedAt} message={message} off={live.enabled && live.off} onRefresh={live.refresh} refreshing={live.refreshing} />;
}
