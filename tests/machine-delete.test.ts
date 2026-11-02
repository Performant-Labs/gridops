import { describe, expect, it } from "vitest";

import { FleetRequestError } from "../src/features/fleet/registry-api";
import { blockerSummary, canOfferDelete, deleteConfirmation, refusalMessage } from "../src/features/fleet/machine-delete";
import { fleetErrorSchema, hostDeleteBlockers } from "../src/features/fleet/contracts";

type Freshness = "fresh" | "stale" | "offline" | "unknown";
const host = (enrollment: "pending" | "approved" | "revoked", lifecycle: string, heartbeatFreshness: Freshness) =>
  ({ enrollment, lifecycle, heartbeatFreshness }) as Parameters<typeof canOfferDelete>[0];

describe("when Delete is offered on a machine", () => {
  it("is offered for a retired machine, however it last reported", () => {
    for (const freshness of ["fresh", "stale", "offline", "unknown"] as const) {
      expect(canOfferDelete(host("approved", "retired", freshness))).toBe(true);
    }
  });

  it("is never offered for an approved machine that is still in service", () => {
    for (const lifecycle of ["active", "paused", "draining", "maintenance"]) {
      for (const freshness of ["fresh", "stale", "offline", "unknown"] as const) {
        expect(canOfferDelete(host("approved", lifecycle, freshness))).toBe(false);
      }
    }
  });

  it("is offered for a machine that never finished enrolling only once it has gone quiet", () => {
    expect(canOfferDelete(host("pending", "paused", "offline"))).toBe(true);
    expect(canOfferDelete(host("pending", "paused", "unknown"))).toBe(true);
    expect(canOfferDelete(host("pending", "paused", "fresh"))).toBe(false);
    expect(canOfferDelete(host("pending", "paused", "stale"))).toBe(false);
    expect(canOfferDelete(host("revoked", "paused", "offline"))).toBe(true);
  });
});

describe("the delete confirmation", () => {
  it("names the machine and says it cannot be undone", () => {
    const text = deleteConfirmation("build-box-7");
    expect(text.startsWith("Delete build-box-7?")).toBe(true);
    expect(text).toContain("can’t be undone");
  });
});

describe("a refused delete", () => {
  const refusal = (reasons: unknown) => {
    const body = fleetErrorSchema.parse({
      code: "host_not_deletable", message: "Machine cannot be deleted yet",
      requestId: "00000001-0000-4000-8000-000000000001",
      details: { kind: "delete_blocked", reasons },
    });
    return new FleetRequestError(409, body);
  };

  it("has a sentence for every reason the server can send", () => {
    for (const reason of hostDeleteBlockers) {
      expect(blockerSummary([reason]).length).toBeGreaterThan(40);
    }
  });

  it("lists every reason it was refused", () => {
    const message = refusalMessage(refusal(["live_placement", "pending_command"]));
    expect(message).toContain("not cleaned up");
    expect(message).toContain("command");
  });

  it("is ignored for other failures, which keep their usual text", () => {
    expect(refusalMessage(new FleetRequestError(403))).toBeNull();
    expect(refusalMessage(new Error("offline"))).toBeNull();
  });

  it("rejects a reason outside the closed set", () => {
    expect(() => refusal(["made_up"])).toThrow();
  });
});
