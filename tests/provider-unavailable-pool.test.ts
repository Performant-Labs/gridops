/**
 * A pool the reconciler skips because the runner manager could not ask any provider it runs on
 * (the Mac's Tart agent sleeps, is down, or refuses the manager, #122): the server shows it in the
 * state `provider-unavailable`, and the UI shows it as unable to run, in the pools that need
 * attention, rather than as whatever state it last had.
 */
import { describe, expect, it } from "vitest";

import { healthTone, statusLabel } from "../src/components/status-icon";
import { poolGroup } from "../src/features/runner-pools/pool-group";
import { provisioningNotice } from "../src/features/runner-pools/waiting-notice";

describe("a pool whose runner provider is unavailable", () => {
  it("is shown as unable to run, as a danger state, not left looking healthy or merely waiting", () => {
    expect(statusLabel("provider-unavailable")).toBe("Cannot run: runner provider unavailable");
    expect(healthTone("provider-unavailable")).toBe("danger");
  });

  it("is listed under the pools that need attention", () => {
    const pool = { paused: false, state: "provider-unavailable", provisionCircuitOpen: false, failedRunners: 0, outdatedRunners: 0 };
    expect(poolGroup(pool)).toBe("attention");
    // An open circuit or a failed runner does not move it to another group either.
    expect(poolGroup({ ...pool, provisionCircuitOpen: true })).toBe("attention");
    expect(poolGroup({ ...pool, failedRunners: 1 })).toBe("attention");
    // A paused pool stays paused: pausing is the operator's own block and comes first.
    expect(poolGroup({ ...pool, paused: true })).toBe("paused");
  });
});

/**
 * Round 3 of PR #123, finding 1c: the pool page explains the state with a callout, as it does a
 * wait, giving the server's reason (`provisionWaitReason`): which provider, and whether its agent
 * cannot be reached or answers with an error. It is a danger, as the state's tone is.
 */
describe("the pool page of a pool whose runner provider is unavailable", () => {
  const pool = { state: "provider-unavailable", provisionCircuitOpen: false, provisionFailureCount: 0 };
  const reason = "macOS (Tart) provider unavailable: its agent cannot be reached.";

  it("shows a danger callout that names the state and gives the server's reason", () => {
    const notice = provisioningNotice({ ...pool, provisionWaitReason: reason }, "shortly");
    expect(notice).not.toBeNull();
    expect(notice?.title.toLowerCase()).toContain("provider unavailable");
    expect(notice?.body).toContain(reason);
    expect(notice?.tone).toBe("danger");
  });

  it("still shows the callout when the server gives no reason, and before an open circuit", () => {
    const notice = provisioningNotice({ ...pool, provisionWaitReason: null, provisionCircuitOpen: true, provisionFailureCount: 3 }, "at 10:30");
    expect(notice?.title.toLowerCase()).toContain("provider unavailable");
    expect(notice?.tone).toBe("danger");
  });
});
