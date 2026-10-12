/**
 * Version-one read-only fleet query contracts.
 *
 * These schemas are the browser boundary for the eight fleet GET routes. They
 * keep physical host identity separate from backend execution identity and
 * reject fields that are outside the authorized projection.
 */
import { z } from "zod";
import { fleetCounterSchema, fleetIdSchema, fleetTimestampSchema } from "./contracts";

const safeInteger = z.number().int().min(0).max(Number.MAX_SAFE_INTEGER);
const counter = fleetCounterSchema;
const hasLoneSurrogate = (value: string) => {
  for (let index = 0; index < value.length; index += 1) {
    const code = value.charCodeAt(index);
    if (code >= 0xd800 && code <= 0xdbff) {
      const next = value.charCodeAt(index + 1);
      if (!(next >= 0xdc00 && next <= 0xdfff)) return true;
      index += 1;
    } else if (code >= 0xdc00 && code <= 0xdfff) {
      return true;
    }
  }
  return false;
};
const hasControlOrLoneSurrogate = (value: string) => /[\p{Cc}]/u.test(value) || hasLoneSurrogate(value);
const text = z.string().min(1).refine((value) => [...value].length <= 128, "Text exceeds 128 Unicode scalars")
  .refine((value) => !hasControlOrLoneSurrogate(value), "Text contains an invalid character");
const boundedUtf8Bytes = (value: string) => new TextEncoder().encode(value).byteLength <= 128;
const tagValue = z.string().refine(boundedUtf8Bytes, "Tag value exceeds 128 UTF-8 bytes")
  .refine((value) => !hasControlOrLoneSurrogate(value), "Tag value contains an invalid character");
const os = z.enum(["linux", "macos", "windows"]);
const architecture = z.enum(["x64", "arm64"]);
const runtimeKind = z.enum(["docker", "tart_vm", "native_process"]);

// Query cursors are sealed browser envelopes. The payload is opaque and may
// contain only the URL-safe alphabet; its signed contents never enter this API.
export const fleetQueryCursorSchema = z.string()
  .min(4)
  .max(4096)
  .regex(/^q1\.[A-Za-z0-9_-]+$/, "Invalid fleet query cursor");

const digest = z.string().regex(/^[A-Za-z0-9_-]{42}[AEIMQUYcgkosw048]$/, "Invalid SHA-256 digest");
const checkedTags = z.record(z.string(), tagValue).superRefine((value, context) => {
  if (Object.keys(value).length > 64) {
    context.addIssue({ code: "too_big", maximum: 64, origin: "array", inclusive: true });
  }
  for (const key of Object.keys(value)) {
    if (key.length === 0 || !boundedUtf8Bytes(key) || hasControlOrLoneSurrogate(key)) {
      context.addIssue({ code: "custom", message: "Invalid tag key" });
    }
  }
});

export const fleetResourceAmountSchema = z.strictObject({
  cpuMillis: safeInteger,
  memoryMib: safeInteger,
  diskBytes: safeInteger,
});

export const fleetAllocationSchema = z.strictObject({
  charged: fleetResourceAmountSchema,
  uncertain: fleetResourceAmountSchema,
});

export const fleetRecordCountsSchema = z.strictObject({
  scope: z.enum(["host", "visible_targets"]),
  knownManagedRunnerCount: safeInteger,
  chargedCiPlacementCount: safeInteger,
  chargedFixPlacementCount: safeInteger,
  runningCiPlacementCount: safeInteger,
  runningFixPlacementCount: safeInteger,
  uncertainPlacementCount: safeInteger,
});
const visibleTargetCountsSchema = fleetRecordCountsSchema.extend({ scope: z.literal("visible_targets") });

export const fleetMeasurementSchema = z.strictObject({
  observedAt: fleetTimestampSchema,
  receivedAt: fleetTimestampSchema,
  coverage: z.enum(["complete", "partial", "unknown"]),
  provenance: z.enum(["current", "historical", "legacy_unknown"]),
  cpuUsedMillis: safeInteger.nullable(),
  memoryUsedMib: safeInteger.nullable(),
  diskFreeBytes: safeInteger.nullable(),
  uptimeSeconds: safeInteger.nullable(),
});

export const fleetInventoryEvidenceSchema = z.discriminatedUnion("state", [
  z.strictObject({
    state: z.literal("current"),
    inventoryRevision: counter,
    digest,
    receivedAt: fleetTimestampSchema,
  }),
  z.strictObject({
    state: z.literal("stale"),
    inventoryRevision: counter.nullable(),
    digest: digest.nullable(),
    receivedAt: fleetTimestampSchema.nullable(),
  }),
  z.strictObject({ state: z.literal("missing") }),
]);

export const fleetApprovalEvidenceSchema = z.discriminatedUnion("state", [
  z.strictObject({
    state: z.literal("available"),
    expectedRevision: counter,
    expectedInventoryRevision: counter,
    expectedInventoryDigest: digest,
    inventoryReceivedAt: fleetTimestampSchema,
    observationLeaseExpiresAt: fleetTimestampSchema,
  }),
  z.strictObject({
    state: z.literal("unavailable"),
    reason: z.enum([
      "requires_system_admin",
      "pending_host",
      "retired_host",
      "missing_control_plane",
      "missing_observation",
      "observation_expired",
      "observation_conflicted",
      "stale_inventory",
      "invalid_inventory_receipt",
    ]),
  }),
]);

const hostReadV1HostSchema = z.strictObject({
  projection: z.literal("host"),
  id: fleetIdSchema,
  name: text,
  hostOs: os,
  architecture,
  enrollment: z.enum(["pending", "approved", "revoked"]),
  lifecycle: z.enum(["active", "paused", "draining", "maintenance", "retired"]),
  integrity: z.enum(["unverified", "verified", "quarantined"]),
  heartbeatFreshness: z.enum(["fresh", "stale", "offline", "unknown"]),
  pressure: z.enum(["normal", "pressured", "unknown"]),
  policyRevision: counter,
  agentVersion: text.nullable(),
  lastHeartbeatAt: fleetTimestampSchema.nullable(),
  lastInventoryAt: fleetTimestampSchema.nullable(),
  backendCount: safeInteger,
  counts: fleetRecordCountsSchema,
  tags: checkedTags,
  physicalBudget: fleetResourceAmountSchema,
  physicalAllocation: fleetAllocationSchema,
  latestRootMeasurement: fleetMeasurementSchema.nullable(),
  inventoryEvidence: fleetInventoryEvidenceSchema,
  approvalEvidence: fleetApprovalEvidenceSchema,
  duplicateHardware: z.boolean(),
  credentialGeneration: z.number().int().min(0).max(Number.MAX_SAFE_INTEGER),
  enrollmentTargetIds: z.array(fleetIdSchema).max(64),
});

const hostReadV1TargetSchema = z.strictObject({
  projection: z.literal("target"),
  id: fleetIdSchema,
  name: text,
  hostOs: os,
  architecture,
  counts: visibleTargetCountsSchema,
});

export const hostReadV1Schema = z.discriminatedUnion("projection", [
  hostReadV1HostSchema,
  hostReadV1TargetSchema,
]);

const backendReadV1HostSchema = z.strictObject({
  projection: z.literal("host"),
  id: fleetIdSchema,
  hostId: fleetIdSchema,
  domainId: fleetIdSchema,
  runtimeKind,
  executionOs: os,
  architecture,
  enabled: z.boolean(),
  policyRevision: counter,
  observedReadiness: z.enum(["ready", "reconciling", "unavailable", "unsupported", "unknown"]),
  probeFreshness: z.enum(["fresh", "stale", "offline", "unknown"]),
  safeReason: z.enum([
    "capability_unverified",
    "incompatible_runtime",
    "incompatible_os",
    "incompatible_architecture",
    "offline",
    "pressure_unknown",
    "resource_pressure",
    "unsupported",
    "unknown",
  ]).nullable(),
  counts: fleetRecordCountsSchema,
  allocation: fleetAllocationSchema,
});

const backendReadV1TargetSchema = z.strictObject({
  projection: z.literal("target"),
  id: fleetIdSchema,
  hostId: fleetIdSchema,
  runtimeKind,
  executionOs: os,
  architecture,
  counts: visibleTargetCountsSchema,
});

export const backendReadV1Schema = z.discriminatedUnion("projection", [
  backendReadV1HostSchema,
  backendReadV1TargetSchema,
]);

const observedCapacityDimensions = {
  receivedAt: fleetTimestampSchema,
  cpuMillis: safeInteger.nullable(),
  memoryMib: safeInteger.nullable(),
  diskBytes: safeInteger.nullable(),
} as const;

export const fleetObservedCapacitySchema = z.discriminatedUnion("state", [
  z.strictObject({ state: z.literal("current"), ...observedCapacityDimensions }),
  z.strictObject({ state: z.literal("historical"), ...observedCapacityDimensions }),
  z.strictObject({ state: z.literal("missing") }),
]);

export const domainReadV1Schema = z.strictObject({
  id: fleetIdSchema,
  hostId: fleetIdSchema,
  parentId: fleetIdSchema.nullable(),
  name: text,
  kind: z.enum(["physical", "virtual_machine", "container"]).nullable(),
  policyRevision: counter,
  budget: fleetResourceAmountSchema,
  allocation: fleetAllocationSchema,
  observedCapacity: fleetObservedCapacitySchema,
  latestMeasurement: fleetMeasurementSchema.nullable(),
});

export const sampleReadV1Schema = z.strictObject({
  domainId: fleetIdSchema.nullable(),
  observedAt: fleetTimestampSchema,
  receivedAt: fleetTimestampSchema,
  coverage: z.enum(["complete", "partial", "unknown"]),
  provenance: z.enum(["current", "historical", "legacy_unknown"]),
  cpuUsedMillis: safeInteger.nullable(),
  memoryUsedMib: safeInteger.nullable(),
  diskFreeBytes: safeInteger.nullable(),
  uptimeSeconds: safeInteger.nullable(),
});

export const hostEventReadV1Schema = z.strictObject({
  id: fleetIdSchema,
  hostId: fleetIdSchema,
  targetId: fleetIdSchema.nullable(),
  placementId: fleetIdSchema.nullable(),
  kind: z.enum([
    "enrollment_issued",
    "enrollment_consumed",
    "enrollment_revoked",
    "host_approved",
    "host_recovered",
    "host_credential_rotated",
    "host_credential_revoked",
    "inventory_received",
    "inventory_conflict",
    "backend_changed",
    "placement_queued",
    "placement_admitted",
    "placement_changed",
    "command_changed",
    "other",
  ]),
  occurredAt: fleetTimestampSchema,
  cursor: fleetQueryCursorSchema,
});

export function queryResultSchema<T extends z.ZodType>(item: T) {
  return z.strictObject({ schemaVersion: z.literal(1), readAt: fleetTimestampSchema, item });
}

export function queryPageSchema<T extends z.ZodType>(item: T) {
  return z.strictObject({
    schemaVersion: z.literal(1),
    readAt: fleetTimestampSchema,
    items: z.array(item).max(100),
    nextCursor: fleetQueryCursorSchema.nullable(),
  });
}

export const hostQueryPageSchema = queryPageSchema(hostReadV1Schema);
export const backendQueryPageSchema = queryPageSchema(backendReadV1Schema);
export const domainQueryPageSchema = queryPageSchema(domainReadV1Schema);
export const sampleQueryPageSchema = queryPageSchema(sampleReadV1Schema);
export const eventQueryPageSchema = queryPageSchema(hostEventReadV1Schema);
export const hostQueryResultSchema = queryResultSchema(hostReadV1Schema);
export const backendQueryResultSchema = queryResultSchema(backendReadV1Schema);
export const domainQueryResultSchema = queryResultSchema(domainReadV1Schema);

const queryOptions = {
  limit: safeInteger.int().min(1).max(100).optional(),
  cursor: fleetQueryCursorSchema.optional(),
  targetId: fleetIdSchema.optional(),
} as const;

export const fleetCatalogQuerySchema = z.strictObject(queryOptions);
const domainQueryOptions = { limit: queryOptions.limit, cursor: queryOptions.cursor } as const;
export const fleetDomainCatalogQuerySchema = z.strictObject(domainQueryOptions);

export const fleetSampleQuerySchema = z.strictObject({
  limit: queryOptions.limit,
  cursor: queryOptions.cursor,
  domainId: fleetIdSchema.optional(),
  receivedFrom: fleetTimestampSchema,
  receivedTo: fleetTimestampSchema,
}).superRefine((value, context) => {
  const from = Date.parse(value.receivedFrom);
  const to = Date.parse(value.receivedTo);
  if (!(from < to)) {
    context.addIssue({ code: "custom", path: ["receivedTo"], message: "receivedTo must be after receivedFrom" });
  } else if (to - from > 24 * 60 * 60 * 1000) {
    context.addIssue({ code: "custom", path: ["receivedTo"], message: "Sample window must be at most 24 hours" });
  }
});

export const fleetEventQuerySchema = z.strictObject({
  ...queryOptions,
  occurredFrom: fleetTimestampSchema.optional(),
  occurredTo: fleetTimestampSchema.optional(),
}).superRefine((value, context) => {
  const hasFrom = value.occurredFrom !== undefined;
  const hasTo = value.occurredTo !== undefined;
  if (hasFrom !== hasTo) {
    context.addIssue({ code: "custom", path: [hasFrom ? "occurredTo" : "occurredFrom"], message: "Event bounds must be supplied together" });
  } else if (hasFrom && hasTo) {
    const { occurredFrom, occurredTo } = value;
    if (occurredFrom !== undefined && occurredTo !== undefined && !(Date.parse(occurredFrom) < Date.parse(occurredTo))) {
      context.addIssue({ code: "custom", path: ["occurredTo"], message: "occurredTo must be after occurredFrom" });
    }
  }
});

export type HostReadV1 = z.infer<typeof hostReadV1Schema>;
export type BackendReadV1 = z.infer<typeof backendReadV1Schema>;
export type DomainReadV1 = z.infer<typeof domainReadV1Schema>;
export type SampleReadV1 = z.infer<typeof sampleReadV1Schema>;
export type HostEventReadV1 = z.infer<typeof hostEventReadV1Schema>;
export type FleetQueryResult<T> = { schemaVersion: 1; readAt: string; item: T };
export type FleetQueryPage<T> = { schemaVersion: 1; readAt: string; items: T[]; nextCursor: string | null };
export type FleetCatalogQuery = z.input<typeof fleetCatalogQuerySchema>;
export type FleetDomainCatalogQuery = z.input<typeof fleetDomainCatalogQuerySchema>;
export type FleetSampleQuery = z.input<typeof fleetSampleQuerySchema>;
export type FleetEventQuery = z.input<typeof fleetEventQuerySchema>;
