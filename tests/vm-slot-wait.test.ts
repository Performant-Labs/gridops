/**
 * A pool waiting for one of the Tart host's macOS VM slots (#95): the pool page
 * says so, with the runner manager's reason (its place in the queue and who
 * holds the slots), instead of the generic "host capacity" notice.
 */
import { describe, expect, it } from "vitest";

import { poolGroup } from "../src/features/runner-pools/pool-group";
import { provisioningNotice, waitingNotice } from "../src/features/runner-pools/waiting-notice";

const reason = "Waiting for a macOS VM slot: position 2 of 2 in the queue. 2 of 2 slots in use (GridOps runners: 1; other software: 1 — runner-warm-1 (ci)).";

describe("waitingNotice", () => {
  it("names the VM slot wait and shows the queue position and who holds the slots", () => {
    const notice = waitingNotice({ provisionWaitReason: reason }, "shortly");
    expect(notice.title).toBe("Waiting for a macOS VM slot");
    expect(notice.body).toContain("position 2 of 2 in the queue");
    expect(notice.body).toContain("runner-warm-1 (ci)");
    expect(notice.body).toContain("not a failure");
  });

  it("shows another wait's reason with the host-capacity notice", () => {
    const notice = waitingNotice({ provisionWaitReason: "Host runner limit reached (5/4 including reservations)." }, "at 10:30");
    expect(notice.title).toBe("Waiting for host capacity");
    expect(notice.body).toContain("Host runner limit reached");
    expect(notice.body).toContain("next check at 10:30");
  });

  it("keeps the generic notice for a server that gives no reason", () => {
    for (const pool of [{}, { provisionWaitReason: null }]) {
      const notice = waitingNotice(pool, "shortly");
      expect(notice.title).toBe("Waiting for host capacity");
      expect(notice.body).toBe("The runner host is at its safe limit. GridOps starts more runners as capacity frees up, next check shortly.");
    }
  });
});

/**
 * A pool waiting in the Mac's slot queue is shown waiting even with earlier failures or an open
 * provisioning circuit (#102): the wait comes first, and the circuit is still named.
 */
describe("provisioningNotice", () => {
  const pool = { state: "waiting", provisionCircuitOpen: false, provisionFailureCount: 0, provisionWaitReason: reason };

  it("shows the slot wait for a waiting pool whose circuit is open, and says the circuit is open", () => {
    const notice = provisioningNotice({ ...pool, provisionCircuitOpen: true, provisionFailureCount: 3 }, "shortly");
    expect(notice?.title).toBe("Waiting for a macOS VM slot");
    expect(notice?.body).toContain("position 2 of 2 in the queue");
    expect(notice?.body).toContain("3 earlier failures");
    expect(notice?.tone).toBe("warning");
  });

  it("shows the slot wait for a waiting pool with earlier failures", () => {
    const notice = provisioningNotice({ ...pool, provisionFailureCount: 1 }, "shortly");
    expect(notice?.title).toBe("Waiting for a macOS VM slot");
    expect(notice?.body).toContain("The last attempt failed before this wait");
    expect(notice?.tone).toBe("neutral");
  });

  it("keeps the failure notices for a pool that is not waiting", () => {
    expect(provisioningNotice({ ...pool, state: "blocked", provisionCircuitOpen: true, provisionFailureCount: 3 }, "at 10:30")?.title)
      .toBe("Provisioning paused after 3 consecutive failures");
    expect(provisioningNotice({ ...pool, state: "backoff", provisionFailureCount: 2 }, "at 10:30")?.title).toBe("Provisioning is backing off");
    expect(provisioningNotice({ ...pool, state: "active", provisionWaitReason: null }, "shortly")).toBeNull();
  });
});

describe("poolGroup", () => {
  const pool = { paused: false, state: "waiting", provisionCircuitOpen: false, failedRunners: 0, outdatedRunners: 0 };

  it("lists a waiting pool under waiting even with an open circuit or a failed runner", () => {
    expect(poolGroup(pool)).toBe("waiting");
    expect(poolGroup({ ...pool, provisionCircuitOpen: true })).toBe("waiting");
    expect(poolGroup({ ...pool, failedRunners: 1 })).toBe("waiting");
  });

  it("does not hide a real block or a failure behind a wait", () => {
    expect(poolGroup({ ...pool, paused: true })).toBe("paused");
    expect(poolGroup({ ...pool, state: "provisioning-paused" })).toBe("paused");
    expect(poolGroup({ ...pool, cannotRun: { code: "builtin_host_disabled", message: "off" } })).toBe("attention");
    expect(poolGroup({ ...pool, state: "blocked", provisionCircuitOpen: true })).toBe("attention");
    expect(poolGroup({ ...pool, state: "backoff" })).toBe("waiting");
    expect(poolGroup({ ...pool, state: "active", failedRunners: 1 })).toBe("attention");
    expect(poolGroup({ ...pool, state: "active" })).toBe("active");
  });
});
