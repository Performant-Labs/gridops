import { useQuery } from "@tanstack/react-query";
import { Link } from "@tanstack/react-router";
import { useEffect } from "react";

import { Callout, SectionHeading } from "~/components/page";
import { StatusDot } from "~/components/status-icon";
import { useRefreshInterval, useRefreshSignal } from "~/lib/use-refresh-interval";
import { formatDateTime } from "~/lib/utils";
import { TartAgentCapWarning } from "./tart-agent-cap";
import {
  externalSourceLabel, formatWaited, getTartHost, showsTartHost, slotRows,
  type SlotRow, type TartHost, type TartHostQueueEntry,
} from "./tart-host";

/**
 * The Overview's Tart host card: polls the slots on its own, so a slow or unreachable runner
 * manager never holds up the rest of the page, and renders nothing until there is a Tart host to
 * show (or when the request fails: the card is information, not an alarm).
 */
export function TartHostSection() {
  const interval = useRefreshInterval(10_000);
  const refreshSignal = useRefreshSignal();
  const host = useQuery({
    queryKey: ["overview", "tart-host"],
    queryFn: ({ signal }) => getTartHost(signal),
    refetchInterval: interval ?? false,
  });
  const { refetch } = host;
  // A manual Refresh reloads the card with the page, keeping what it shows meanwhile.
  useEffect(() => {
    if (refreshSignal > 0) void refetch();
  }, [refreshSignal, refetch]);
  return <TartHostCard host={host.data} now={host.dataUpdatedAt} />;
}

/**
 * The Mac's macOS VM slots at a glance: the limit, who holds each slot (a GridOps runner with its
 * pool and since when, a VM another program runs, a start in progress, or nobody), the pools
 * waiting for a slot in order with how long each has waited, and the warning when the Tart agent's
 * own runner cap is below the slot setting. `now` is when `host` was read, for the ages.
 */
export function TartHostCard({ host, now }: { host: TartHost | null | undefined; now: number }) {
  if (!showsTartHost(host)) return null;
  const slots = host.slots;
  return (
    <section aria-label="Tart host" className="space-y-3">
      <SectionHeading
        actions={slots ? (
          <span className="tabular text-xs text-faint">{slots.inUse} of {slots.limit} in use</span>
        ) : null}
      >
        Tart host
      </SectionHeading>
      <div className="overflow-hidden rounded-lg border border-border">
        <div className="flex flex-wrap items-center gap-x-5 gap-y-1 border-b border-border px-4 py-2.5 text-xs text-muted-foreground">
          <span className="flex items-center gap-1.5">
            macOS VM slots on the Tart host
            <span className="tabular font-medium text-foreground">{slots?.limit ?? host.slotSetting ?? "—"}</span>
          </span>
          {slots ? (
            <>
              <span className="flex items-center gap-1.5">Free<span className="tabular font-medium text-foreground">{slots.free}</span></span>
              <span className="flex items-center gap-1.5">Waiting<span className="tabular font-medium text-foreground">{slots.queue.length}</span></span>
            </>
          ) : null}
        </div>
        {host.tartAgent === "unavailable" ? (
          <div className="p-4">
            <Callout title="The Tart agent is unreachable" tone="danger">
              {host.message ?? "The runner manager cannot reach the Tart agent."} Who holds the slots and which pools wait is unknown until it answers again; no runner can start on the Tart host meanwhile.
            </Callout>
          </div>
        ) : !slots ? (
          <p className="px-4 py-3 text-sm text-muted-foreground">The runner manager could not read the Tart agent's VM slots.</p>
        ) : (
          <>
            {slots.agentMaxRunners !== null && (host.slotSetting ?? slots.limit) > slots.agentMaxRunners ? (
              <div className="border-b border-border p-4">
                <TartAgentCapWarning agentMaxRunners={slots.agentMaxRunners} slots={host.slotSetting ?? slots.limit} />
              </div>
            ) : null}
            <ul aria-label="Slots">
              {slotRows(slots).map((row, index) => <SlotLine key={index} now={now} row={row} />)}
            </ul>
            <div className="flex h-9 items-center border-y border-border bg-panel-subtle px-4 text-xs font-medium">
              Waiting for a slot
            </div>
            {slots.queue.length === 0 ? (
              <p className="px-4 py-3 text-sm text-muted-foreground">No pool is waiting for a slot.</p>
            ) : (
              <ol aria-label="Slot queue">
                {slots.queue.map((entry) => <QueueLine entry={entry} key={entry.position} now={now} />)}
              </ol>
            )}
          </>
        )}
      </div>
    </section>
  );
}

const rowClassName = "flex items-start gap-3 border-b border-border px-4 py-2.5 text-sm last:border-b-0";

function SlotLine({ row, now }: { row: SlotRow; now: number }) {
  if (row.kind === "free") {
    return (
      <li className={rowClassName} data-slot="free">
        <StatusDot className="mt-1.5" pulse={false} tone="neutral" />
        <span className="min-w-0 flex-1 text-muted-foreground">Free</span>
      </li>
    );
  }
  if (row.kind === "external") {
    return (
      <li className={rowClassName} data-slot="external">
        <StatusDot className="mt-1.5" pulse={false} tone="warning" />
        <div className="min-w-0 flex-1">
          <div className="font-medium">Started outside GridOps</div>
          <div className="break-words text-xs text-muted-foreground">
            <span className="font-mono">{row.name}</span>{row.user ? ` (${row.user})` : null} · {externalSourceLabel(row.source)}
          </div>
        </div>
      </li>
    );
  }
  const starting = row.kind === "starting";
  return (
    <li className={rowClassName} data-slot={row.kind}>
      <StatusDot className="mt-1.5" pulse={starting} tone={starting ? "progress" : "success"} />
      <div className="min-w-0 flex-1">
        <div className="font-medium">{starting ? "Starting a GridOps runner" : "GridOps runner"}</div>
        <div className="break-words text-xs text-muted-foreground">
          <RunnerDetail row={row} />
        </div>
      </div>
      {row.since ? (
        <time className="tabular shrink-0 text-xs text-faint" dateTime={row.since} title={`Since ${formatDateTime(row.since)}`}>
          {formatWaited(row.since, now)}
        </time>
      ) : null}
    </li>
  );
}

function RunnerDetail({ row }: { row: Extract<SlotRow, { kind: "runner" | "starting" }> }) {
  if (row.poolHidden) return <>In a pool you can't see</>;
  if (!row.poolId) return <>Not identified: the Tart agent does not name its runners, or GridOps has no record of this one</>;
  return (
    <>
      <PoolLink id={row.poolId} name={row.poolName} />
      {row.runnerId && row.runnerName ? (
        <>
          {" · "}
          <Link className="font-mono hover:underline" search={{ target: row.runnerId }} to="/live-logs">{row.runnerName}</Link>
        </>
      ) : null}
    </>
  );
}

function QueueLine({ entry, now }: { entry: TartHostQueueEntry; now: number }) {
  return (
    <li className={rowClassName}>
      <span className="tabular w-5 shrink-0 text-right text-muted-foreground">{entry.position}</span>
      <span className="min-w-0 flex-1 break-words">
        {entry.poolId ? <PoolLink id={entry.poolId} name={entry.poolName} /> : entry.poolHidden ? "A pool you can't see" : "A pool GridOps no longer has"}
      </span>
      <time className="tabular shrink-0 text-xs text-faint" dateTime={entry.waitingSince} title={`Waiting since ${formatDateTime(entry.waitingSince)}`}>
        waiting {formatWaited(entry.waitingSince, now)}
      </time>
    </li>
  );
}

function PoolLink({ id, name }: { id: string; name: string | null }) {
  return <Link className="font-medium text-foreground hover:underline" params={{ poolId: id }} to="/runner-pools/$poolId">{name ?? id}</Link>;
}
