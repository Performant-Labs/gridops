import { describe, expect, it } from "vitest";

import { hostResourceWarning, runnerHostBudgetNote, runnerHostLabel } from "~/features/runner-pools/resource-risk";

describe("hostResourceWarning", () => {
  it("stays quiet when the current target fits the host budget", () => {
    expect(hostResourceWarning({
      runnerCount: 1,
      cpuLimit: 2,
      memoryLimitMb: 2048,
      cpuBudget: 9,
      memoryBudgetMb: 7604,
    })).toBeNull();
  });

  it("warns when the current target can exceed CPU or memory capacity", () => {
    expect(hostResourceWarning({
      runnerCount: 3,
      cpuLimit: 4,
      memoryLimitMb: 4096,
      cpuBudget: 9,
      memoryBudgetMb: 7604,
    })).toMatchObject({ cpuRequested: 12, memoryRequestedMb: 12_288 });
  });
});

describe("runnerHostLabel", () => {
  it("names the built-in runner host and, when known, which machine it is", () => {
    expect(runnerHostLabel({ name: "build-box", cpus: 16, memoryMb: 64_301 })).toBe("the built-in runner host (build-box)");
    expect(runnerHostLabel({ name: null, cpus: 16, memoryMb: 64_301 })).toBe("the built-in runner host");
    expect(runnerHostLabel(null)).toBe("the built-in runner host");
  });
});

describe("runnerHostBudgetNote", () => {
  it("says what share of the host the budget is, when the host's size is known", () => {
    expect(runnerHostBudgetNote({ name: "build-box", cpus: 16, memoryMb: 64_301 }, 15, 61_085))
      .toBe("That is 94% of its 16 cores and 95% of its 64,301 MB.");
  });

  it("says nothing it does not know", () => {
    expect(runnerHostBudgetNote(null, 15, 61_085)).toBeNull();
    expect(runnerHostBudgetNote({ name: null, cpus: null, memoryMb: null }, 15, 61_085)).toBeNull();
  });
});
