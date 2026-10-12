/** Verifies strict version-one fleet query projections and bounded filters. */
import { describe, expect, it } from "vitest";
import fixture from "./fixtures/fleet-query-contracts.json";
import {
  backendQueryPageSchema,
  backendQueryResultSchema,
  domainQueryPageSchema,
  domainQueryResultSchema,
  eventQueryPageSchema,
  fleetCatalogQuerySchema,
  fleetDomainCatalogQuerySchema,
  fleetEventQuerySchema,
  fleetObservedCapacitySchema,
  fleetSampleQuerySchema,
  hostQueryPageSchema,
  hostQueryResultSchema,
  sampleQueryPageSchema,
} from "../src/features/fleet/query-contracts";

describe("fleet query browser contracts", () => {
  it("round trips every version-one fixture response", () => {
    for (const body of fixture.hostResults) {
      expect(hostQueryResultSchema.parse(body)).toEqual(body);
    }
    for (const body of fixture.hostPages) {
      expect(hostQueryPageSchema.parse(body)).toEqual(body);
    }
    for (const body of fixture.backendResults) {
      expect(backendQueryResultSchema.parse(body)).toEqual(body);
    }
    for (const body of fixture.backendPages) {
      expect(backendQueryPageSchema.parse(body)).toEqual(body);
    }
    for (const body of fixture.domainResults) {
      expect(domainQueryResultSchema.parse(body)).toEqual(body);
    }
    for (const body of fixture.domainPages) {
      expect(domainQueryPageSchema.parse(body)).toEqual(body);
    }
    for (const body of fixture.samplePages) {
      expect(sampleQueryPageSchema.parse(body)).toEqual(body);
    }
    for (const body of fixture.eventPages) {
      expect(eventQueryPageSchema.parse(body)).toEqual(body);
    }
  });

  it("keeps target projections free of host-only policy and evidence fields", () => {
    const target = fixture.hostResults[1]?.item;
    expect(target).toBeDefined();
    expect(hostQueryResultSchema.parse(fixture.hostResults[1]).item).toEqual(target);
    for (const field of ["policyRevision", "tags", "physicalBudget", "physicalAllocation", "latestRootMeasurement", "inventoryEvidence", "approvalEvidence"]) {
      expect(field in target!).toBe(false);
    }

    const backend = fixture.backendResults[1]?.item;
    expect(backend).toBeDefined();
    for (const field of ["domainId", "enabled", "policyRevision", "observedReadiness", "probeFreshness", "safeReason", "allocation"]) {
      expect(field in backend!).toBe(false);
    }
  });

  it("rejects unknown fields, invalid schema versions, unsafe numbers and cursors", () => {
    expect(hostQueryResultSchema.safeParse({ ...fixture.hostResults[0], schemaVersion: 2 }).success).toBe(false);
    expect(hostQueryResultSchema.safeParse({ ...fixture.hostResults[0], extra: true }).success).toBe(false);
    expect(hostQueryResultSchema.safeParse({
      ...fixture.hostResults[0],
      item: { ...fixture.hostResults[0]?.item, hardwareFingerprint: "secret" },
    }).success).toBe(false);
    expect(fleetObservedCapacitySchema.safeParse({ state: "missing", cpuMillis: 0 }).success).toBe(false);
    expect(fleetObservedCapacitySchema.safeParse({
      state: "current", receivedAt: "2026-10-09T12:00:00.000Z", cpuMillis: Number.MAX_SAFE_INTEGER + 1,
      memoryMib: null, diskBytes: null,
    }).success).toBe(false);
    expect(fleetCatalogQuerySchema.safeParse({ limit: 0 }).success).toBe(false);
    expect(fleetCatalogQuerySchema.safeParse({ limit: 101 }).success).toBe(false);
    expect(fleetCatalogQuerySchema.safeParse({ cursor: "forged cursor" }).success).toBe(false);
    expect(fleetCatalogQuerySchema.safeParse({ unknown: 1 }).success).toBe(false);
  });

  it("preserves independent null, zero and partial measurement semantics", () => {
    const sample = fixture.samplePages[0]?.items[0];
    expect(sampleQueryPageSchema.parse(fixture.samplePages[0]).items[0]).toEqual(sample);
    expect(sample?.coverage).toBe("partial");
    expect(sample?.cpuUsedMillis).toBeNull();
    expect(sample?.memoryUsedMib).toBe(0);
    expect(fleetObservedCapacitySchema.parse(fixture.domainResults[2]?.item.observedCapacity)).toEqual({ state: "missing" });
  });

  it("matches the server UTF-8 byte limits for tags and bounded text", () => {
    const host = fixture.hostResults[0];
    const loneSurrogate = String.fromCharCode(0xd800);
    expect(host).toBeDefined();
    if (!host || host.item.projection !== "host") return;
    expect(hostQueryResultSchema.safeParse({
      ...host,
      item: { ...host.item, name: "é".repeat(64), agentVersion: "é".repeat(64), tags: { value: "" } },
    }).success).toBe(true);
    expect(hostQueryResultSchema.safeParse({
      ...host,
      item: { ...host.item, name: "é".repeat(65), agentVersion: "é".repeat(65) },
    }).success).toBe(true);
    expect(hostQueryResultSchema.safeParse({
      ...host,
      item: { ...host.item, tags: { value: "é".repeat(65) } },
    }).success).toBe(false);
    expect(hostQueryResultSchema.safeParse({
      ...host,
      item: { ...host.item, tags: { "": "value" } },
    }).success).toBe(false);
    const loneName = hostQueryResultSchema.safeParse({
      ...host,
      item: { ...host.item, name: loneSurrogate },
    });
    expect(loneName.success).toBe(false);
    expect(hostQueryResultSchema.safeParse({
      ...host,
      item: { ...host.item, tags: { value: loneSurrogate } },
    }).success).toBe(false);
  });

  it("requires an ordered sample window no longer than 24 hours", () => {
    const valid = { receivedFrom: "2026-10-08T12:00:00.000Z", receivedTo: "2026-10-09T12:00:00.000Z" };
    expect(fleetSampleQuerySchema.safeParse(valid).success).toBe(true);
    expect(fleetSampleQuerySchema.safeParse({ receivedFrom: valid.receivedTo, receivedTo: valid.receivedFrom }).success).toBe(false);
    expect(fleetSampleQuerySchema.safeParse({ receivedFrom: "2026-10-08T11:59:59.000Z", receivedTo: valid.receivedTo }).success).toBe(false);
    expect(fleetSampleQuerySchema.safeParse({ ...valid, receivedTo: "2026-10-09T12:00:00.001Z", duplicate: true }).success).toBe(false);
  });

  it("requires paired event bounds when supplied", () => {
    expect(fleetEventQuerySchema.safeParse({}).success).toBe(true);
    expect(fleetEventQuerySchema.safeParse({ occurredFrom: "2026-10-09T00:00:00.000Z" }).success).toBe(false);
    expect(fleetEventQuerySchema.safeParse({ occurredTo: "2026-10-09T01:00:00.000Z" }).success).toBe(false);
    expect(fleetEventQuerySchema.safeParse({ occurredFrom: "2026-10-09T01:00:00.000Z", occurredTo: "2026-10-09T00:00:00.000Z" }).success).toBe(false);
  });

  it("keeps domain catalog filters target-free", () => {
    expect(fleetDomainCatalogQuerySchema.safeParse({ limit: 2, cursor: "q1.payload" }).success).toBe(true);
    expect(fleetDomainCatalogQuerySchema.safeParse({ targetId: "00000008-0000-4000-8000-000000000001" }).success).toBe(false);
  });
});
