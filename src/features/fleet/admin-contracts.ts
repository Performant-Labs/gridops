/**
 * Browser contracts for fleet administration: host lifecycle, backend
 * enablement, CI targets and pool execution profiles.
 */
import { z } from "zod";
import { fleetCounterSchema, fleetIdSchema } from "./contracts";

const safeInteger = z.number().int().min(0).max(Number.MAX_SAFE_INTEGER);

export const hostLifecycleSchema = z.enum(["active", "paused", "draining", "maintenance"]);
export const lifecycleRequestSchema = z.strictObject({
  expectedRevision: fleetCounterSchema,
  lifecycle: hostLifecycleSchema,
});
export const backendEnabledRequestSchema = z.strictObject({ enabled: z.boolean() });

export const fleetTargetSchema = z.strictObject({
  id: fleetIdSchema,
  kind: z.enum(["github_repository", "github_organization"]),
  label: z.string().min(1).max(512),
});
export const fleetTargetListSchema = z.strictObject({ items: z.array(fleetTargetSchema).max(10_000) });

export const grantPolicySchema = z.strictObject({
  allowSchedule: z.boolean(),
  allowNative: z.boolean(),
  allowDockerSocket: z.boolean(),
});
export const hostGrantSchema = grantPolicySchema.extend({
  targetId: fleetIdSchema,
  label: z.string().min(1).max(512),
});
export const hostGrantListSchema = z.strictObject({ items: z.array(hostGrantSchema).max(10_000) });

// Pool IDs are GridOps' own identifiers, not fleet UUIDs.
export const fleetPoolIdSchema = z.string().min(1).max(128).regex(/^[A-Za-z0-9._-]+$/);

export const profileInputSchema = z.strictObject({
  enabled: z.boolean(),
  runtimeKind: z.enum(["docker", "native_process", "tart_vm"]),
  executionOs: z.enum(["linux", "macos", "windows"]),
  architecture: z.enum(["x64", "arm64"]),
  mode: z.enum(["ephemeral", "persistent"]),
  image: z.string().trim().min(1).max(512),
  labels: z.array(z.string().trim().min(1).max(128)).max(100),
  cpuMillis: safeInteger.min(1).max(0xffff_ffff),
  memoryMib: safeInteger.min(1).max(0xffff_ffff),
  diskBytes: safeInteger,
  requiresDockerSocket: z.boolean().optional(),
  placement: z.enum(["machines", "machines_first", "host_first"]).optional(),
});
export const profileSchema = profileInputSchema.extend({
  id: fleetIdSchema,
  revision: fleetCounterSchema,
  requiresDockerSocket: z.boolean(),
  targetIds: z.array(fleetIdSchema).max(10_000),
});
export const profileResultSchema = z.strictObject({ item: profileSchema.nullable() });
export const savedProfileResultSchema = z.strictObject({ item: profileSchema });

export const agentLogSchema = z.strictObject({
  id: z.number().int(),
  loggedAt: z.number().int(),
  level: z.enum(["error", "warn", "info", "debug", "trace"]),
  target: z.string().max(128),
  message: z.string().max(8192),
});
export const agentLogListSchema = z.strictObject({ items: z.array(agentLogSchema).max(1000) });
export type AgentLog = z.infer<typeof agentLogSchema>;

export const queuedRunnerSchema = z.strictObject({
  runnerId: z.string().min(1).max(128),
  runnerName: z.string().max(256).nullable(),
  reason: z.string().max(64).nullable(),
  queuedAt: z.number().int().nonnegative(),
});
export const poolQueueSchema = z.strictObject({ items: z.array(queuedRunnerSchema).max(50) });
export type QueuedRunner = z.infer<typeof queuedRunnerSchema>;

export type HostLifecycle = z.infer<typeof hostLifecycleSchema>;
export type GrantPolicy = z.infer<typeof grantPolicySchema>;
export type HostGrant = z.infer<typeof hostGrantSchema>;
export type FleetTarget = z.infer<typeof fleetTargetSchema>;
export type ProfileInput = z.input<typeof profileInputSchema>;
export type Profile = z.infer<typeof profileSchema>;
