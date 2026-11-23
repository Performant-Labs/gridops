/**
 * The fleet health response (`GET /api/v1/fleet/health`), as zod schemas and the types inferred
 * from them. They mirror the Rust types in `crates/gridops-core/src/fleet_health/mod.rs` field for
 * field and refuse a field they don't know, so `fixture.json`, the Rust types and these schemas
 * can't drift apart (`tests/fleet-health-shared.test.ts` parses the fixture with them). The rules
 * behind every number are in `docs/design/fleet-health.md`.
 */
import { z } from "zod";

export const HEALTH_WINDOWS = ["1h", "24h", "7d"] as const;
export const healthWindowSchema = z.enum(HEALTH_WINDOWS);
export type HealthWindow = z.infer<typeof healthWindowSchema>;

/** Every category, in display order. `fix-agent` only appears in the live counts. */
export const JOB_CATEGORIES = ["test", "build", "check", "deploy", "fix-agent", "other"] as const;
export const jobCategorySchema = z.enum(JOB_CATEGORIES);
export type JobCategory = z.infer<typeof jobCategorySchema>;

export const severitySchema = z.enum(["critical", "warning", "info"]);
export type Severity = z.infer<typeof severitySchema>;

const millis = z.number().int();
const count = z.number().int().nonnegative();
const score = z.number().int().min(0).max(100).nullable();
const byCategory = <T extends z.ZodType>(value: T) => z.partialRecord(jobCategorySchema, value);

export const concernSubjectSchema = z.discriminatedUnion("kind", [
  z.strictObject({ kind: z.literal("fleet") }),
  z.strictObject({ kind: z.literal("host"), id: z.string() }),
  z.strictObject({ kind: z.literal("pool"), id: z.string() }),
]);

export const concernSchema = z.strictObject({
  key: z.string().min(1),
  severity: severitySchema,
  title: z.string(),
  detail: z.string(),
  subject: concernSubjectSchema,
  link: z.string().nullable(),
  since: millis.nullable(),
});
export type Concern = z.infer<typeof concernSchema>;

export const scoreComponentSchema = z.strictObject({
  key: z.string().min(1),
  label: z.string(),
  weight: z.number().nonnegative(),
  score,
  deduction: z.string().nullable(),
});
export type ScoreComponent = z.infer<typeof scoreComponentSchema>;

export const categoryUsageSchema = z.strictObject({
  category: jobCategorySchema,
  jobs: count,
  cpuHours: z.number().nonnegative(),
  medianDurationMs: millis.nullable(),
  p90QueueWaitMs: millis.nullable(),
});
export type CategoryUsage = z.infer<typeof categoryUsageSchema>;

export const hourlyUsageSchema = z.strictObject({
  hour: millis,
  cpuHours: byCategory(z.number().nonnegative()),
});

export const trendPointSchema = z.strictObject({ at: millis, score });

export const fleetSummarySchema = z.strictObject({
  score,
  components: z.array(scoreComponentSchema),
  trend: z.array(trendPointSchema),
  categories: z.array(categoryUsageSchema),
});

export const speedKeySchema = z.strictObject({
  key: z.string(),
  hostMedianMs: millis,
  fleetMedianMs: millis,
  hostRuns: count,
  fleetRuns: count,
});

export const hostSpeedSchema = z.strictObject({
  cpuMillis: count,
  index: z.number().positive().nullable(),
  topKeys: z.array(speedKeySchema),
});
export type HostSpeed = z.infer<typeof hostSpeedSchema>;

export const hostLiveStatsSchema = z.strictObject({
  hostId: z.string(),
  online: z.boolean(),
  lastSeenAt: millis.nullable(),
  runningByCategory: byCategory(count),
  cpuReservedMillis: count,
  memoryReservedMib: count,
  cpuUsedMillis: count.nullable(),
  memoryUsedMib: count.nullable(),
  diskFreeBytes: count.nullable(),
  startFailuresLastHour: count,
});
export type HostLiveStats = z.infer<typeof hostLiveStatsSchema>;

export const fleetLiveStatsSchema = z.strictObject({
  queuedTotal: count,
  queuedByCategory: byCategory(count),
  queuedByPool: z.array(z.strictObject({ poolId: z.string(), poolName: z.string(), queued: count })),
  queuedUnmatched: count,
  oldestQueued: z.strictObject({
    jobId: z.number().int(),
    jobName: z.string(),
    repository: z.string(),
    queuedAt: millis,
    waitMs: millis,
  }).nullable(),
  runningTotal: count,
  runningByCategory: byCategory(count),
});
export type FleetLiveStats = z.infer<typeof fleetLiveStatsSchema>;

export const hostHealthSchema = z.strictObject({
  id: z.string(),
  name: z.string(),
  cpuBudgetMillis: count,
  memoryBudgetMib: count,
  diskTotalBytes: count.nullable(),
  score,
  components: z.array(scoreComponentSchema),
  speed: z.array(hostSpeedSchema),
  categories: z.array(categoryUsageSchema),
  usage: z.array(hourlyUsageSchema),
  live: hostLiveStatsSchema.nullable(),
});
export type HostHealth = z.infer<typeof hostHealthSchema>;

export const fleetHealthSchema = z.strictObject({
  window: healthWindowSchema,
  generatedAt: millis,
  stale: z.boolean(),
  fleet: fleetSummarySchema,
  live: fleetLiveStatsSchema,
  hosts: z.array(hostHealthSchema),
  concerns: z.array(concernSchema),
});
export type FleetHealth = z.infer<typeof fleetHealthSchema>;

/** What every fleet health card takes: the parsed response and the window it was asked for. */
export type FleetHealthViewProps = { data: FleetHealth; window: HealthWindow };
