/**
 * The Tart host card's data (#95 phase 2): `GET /api/v1/overview/tart-host`, the Mac's macOS VM
 * slots as the runner manager arbitrates them, who holds each, and the pools waiting for one.
 * The schema is the browser's side of the contract in `tests/fixtures/tart-host-contracts.json`.
 */
import { z } from "zod";

import { api } from "~/lib/api";

const nullableString = z.string().nullable();

const runnerHolderSchema = z.object({
  kind: z.enum(["runner", "starting"]),
  runnerId: nullableString,
  runnerName: nullableString,
  poolId: nullableString,
  poolName: nullableString,
  poolHidden: z.boolean(),
  since: nullableString,
});

const externalHolderSchema = z.object({
  kind: z.literal("external"),
  name: z.string(),
  user: nullableString,
  source: z.string(),
});

export const tartHostHolderSchema = z.discriminatedUnion("kind", [runnerHolderSchema, externalHolderSchema]);

export const tartHostQueueEntrySchema = z.object({
  position: z.number().int().min(1),
  poolId: nullableString,
  poolName: nullableString,
  poolHidden: z.boolean(),
  waitingSince: z.string(),
});

export const tartHostSlotsSchema = z.object({
  limit: z.number().int().min(0),
  inUse: z.number().int().min(0),
  free: z.number().int().min(0),
  detection: nullableString,
  agentMaxRunners: z.number().int().nullable(),
  warning: nullableString,
  holders: z.array(tartHostHolderSchema),
  queue: z.array(tartHostQueueEntrySchema),
});

export const tartHostSchema = z.object({
  authenticated: z.literal(false).optional(),
  tartAgent: z.enum(["available", "unavailable", "not_configured", "unknown"]),
  message: nullableString,
  slotSetting: z.number().int().nullable(),
  slots: tartHostSlotsSchema.nullable(),
});

export type TartHost = z.infer<typeof tartHostSchema>;
export type TartHostSlots = z.infer<typeof tartHostSlotsSchema>;
export type TartHostHolder = z.infer<typeof tartHostHolderSchema>;
export type TartHostQueueEntry = z.infer<typeof tartHostQueueEntrySchema>;

export async function getTartHost(signal?: AbortSignal): Promise<TartHost> {
  return tartHostSchema.parse(await api<unknown>("/api/v1/overview/tart-host", { signal }));
}

/**
 * Whether the card is shown. It is hidden without a Tart agent, and when the runner manager could
 * not be asked (then nobody knows whether there is one; Settings → Runner host says the manager is
 * down). An agent the manager cannot reach is shown, since its slots are what is in question.
 */
export function showsTartHost(host: TartHost | null | undefined): host is TartHost {
  return host?.tartAgent === "available" || host?.tartAgent === "unavailable";
}

/** One line of the card's slot list: a holder, or a free slot. */
export type SlotRow = TartHostHolder | { kind: "free" };

/**
 * The slot list: every holder, then a free row for each slot left of the limit. With more VMs
 * running than the limit (another program's VMs, or a limit just lowered), every holder is listed.
 */
export function slotRows(slots: TartHostSlots): SlotRow[] {
  const free = Math.max(0, slots.limit - slots.holders.length);
  return [...slots.holders, ...Array.from({ length: free }, () => ({ kind: "free" as const }))];
}

/** How long since `since`, to the second under a minute: "45s", "3m 20s", "2h 5m", "3d 4h". */
export function formatWaited(since: string, now: number): string {
  const seconds = Math.max(0, Math.floor((now - new Date(since).getTime()) / 1_000));
  if (seconds < 60) return `${seconds}s`;
  const minutes = Math.floor(seconds / 60);
  if (minutes < 60) return `${minutes}m ${seconds % 60}s`;
  const hours = Math.floor(minutes / 60);
  if (hours < 24) return `${hours}h ${minutes % 60}m`;
  return `${Math.floor(hours / 24)}d ${hours % 24}h`;
}

/** How the agent saw another program's VM, in words. */
export function externalSourceLabel(source: string): string {
  if (source === "tart-process") return "seen as a tart run process";
  if (source === "tart-list") return "listed by tart in the agent's Tart home";
  return source;
}
