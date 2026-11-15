/**
 * A pool the reconciler skips because the runner manager could not ask any provider it runs on
 * (the Mac's Tart agent sleeps, is down, or refuses the manager, #122): the server shows it in the
 * state `provider-unavailable`, and the UI shows it as unable to run, in the pools that need
 * attention, rather than as whatever state it last had.
 */
import { describe, expect, it } from "vitest";

import { healthTone, statusLabel } from "../src/components/status-icon";
import { poolGroup } from "../src/features/runner-pools/pool-group";

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
