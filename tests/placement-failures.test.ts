import { describe, expect, it } from "vitest";

import fixture from "./fixtures/fleet-query-contracts.json";
import { hostQueryResultSchema } from "../src/features/fleet/query-contracts";
import {
  failureReasonLabel,
  failureSummary,
  failuresLastHourLabel,
  startGateLabel,
} from "../src/features/fleet/placement-failures";

describe("placement failures on a machine", () => {
  const host = hostQueryResultSchema.parse(fixture.hostResults[0]).item;

  it("reads the recorded failures and the last-hour count from the machine", () => {
    expect(host.projection).toBe("host");
    if (host.projection !== "host") return;
    expect(host.failedPlacementsLastHour).toBe(3);
    expect(host.recentPlacementFailures.map((failure) => [failure.reason, failure.count])).toEqual([
      ["agent_session_lost", 2],
      ["start_deadline_expired", 1],
    ]);
  });

  it("refuses a reason outside the closed set", () => {
    const body = structuredClone(fixture.hostResults[0]) as unknown as { item: { recentPlacementFailures: Array<{ reason: string }> } };
    const first = body.item.recentPlacementFailures[0];
    if (!first) throw new Error("fixture has no recorded failure");
    first.reason = "made_up";
    expect(() => hostQueryResultSchema.parse(body)).toThrow();
  });

  it("names every reason in words an operator can act on", () => {
    expect(failureReasonLabel("start_deadline_expired")).toBe("Start timed out");
    expect(failureReasonLabel("agent_session_lost")).toBe("Agent session dropped");
    expect(failureReasonLabel("unknown")).toBe("Cause not recorded");
  });

  it("summarises a group as reason, message and how many", () => {
    expect(failureSummary({ reason: "agent_reported", message: "It failed.", count: 1 })).toBe("Agent reported a failure: It failed.");
    expect(failureSummary({ reason: "agent_reported", message: "", count: 4 })).toBe("Agent reported a failure (4 times)");
  });

  it("says plainly how many failed in the last hour", () => {
    expect(failuresLastHourLabel(0)).toBeNull();
    expect(failuresLastHourLabel(1)).toBe("1 failed in the last hour");
    expect(failuresLastHourLabel(17)).toBe("17 failed in the last hour");
  });

  it("reads why a machine takes no new runners, and when it is reconsidered", () => {
    if (host.projection !== "host") throw new Error("fixture is not a host");
    expect(host.startGate).toEqual({
      failures: 3,
      windowMinutes: 15,
      since: "2026-10-09T11:50:00.000Z",
      until: "2026-10-09T12:05:00.000Z",
      message: "Not accepting new runners: 3 start failures in the last 15 minutes.",
    });
    expect(startGateLabel(host.startGate, (iso) => `<${iso}>`)).toBe(
      "Not accepting new runners: 3 start failures in the last 15 minutes. Reconsidered at <2026-10-09T12:05:00.000Z>, or sooner if a start on it succeeds.",
    );
    expect(startGateLabel(null, (iso) => iso)).toBeNull();
  });

  it("requires the gate field, which is null when the machine takes new runners", () => {
    const body = structuredClone(fixture.hostResults[0]) as unknown as { item: Record<string, unknown> };
    body.item.startGate = null;
    expect(hostQueryResultSchema.parse(body).item).toMatchObject({ startGate: null });
    delete body.item.startGate;
    expect(() => hostQueryResultSchema.parse(body)).toThrow();
  });
});
