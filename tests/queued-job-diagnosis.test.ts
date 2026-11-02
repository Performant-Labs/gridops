import { describe, expect, it } from "vitest";

import { diagnosisCandidateLabel } from "~/features/operations/queued-job-diagnosis";

type Candidate = Parameters<typeof diagnosisCandidateLabel>[0];

const candidate = (overrides: Partial<Candidate>): Candidate => ({
  id: "pool-a", name: "linux-pool", status: "ready", provider: "docker", owner: true,
  activeRunners: 1, busyRunners: 0, maximumRunners: 4, availableCapacity: 3,
  ...overrides,
});

describe("queued job diagnosis on a run's page", () => {
  it("gives the slots of the pool that will run the job", () => {
    expect(diagnosisCandidateLabel(candidate({}))).toBe("3 slots available");
    expect(diagnosisCandidateLabel(candidate({ availableCapacity: 1 }))).toBe("1 slot available");
  });

  it("says a manual-mode pool will not start a runner, rather than calling it ready", () => {
    expect(diagnosisCandidateLabel(candidate({ status: "manual" }))).toBe("manual mode: no runner is started for queued jobs");
  });

  it("says plainly why every other pool won't run it", () => {
    expect(diagnosisCandidateLabel(candidate({ status: "standby", owner: false }))).toBe("another pool runs it first");
    expect(diagnosisCandidateLabel(candidate({ status: "at_capacity" }))).toBe("at capacity");
    expect(diagnosisCandidateLabel(candidate({ status: "circuit_open" }))).toBe("circuit open");
    expect(diagnosisCandidateLabel(candidate({ status: "paused" }))).toBe("paused");
    expect(diagnosisCandidateLabel(candidate({ status: "incompatible" }))).toBe("incompatible");
  });
});
