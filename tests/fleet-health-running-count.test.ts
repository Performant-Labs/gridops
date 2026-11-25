// @vitest-environment jsdom
/**
 * Each machine's summary row says how many runners it is running right now, so the owner can
 * see every machine's load without expanding the rows one by one. The count is the sum of the
 * live `runningByCategory`; a machine with no live part (unknown) or offline shows none.
 */
import { createElement } from "react";
import { describe, expect, it } from "vitest";

import { MachineScoreTable } from "~/features/fleet-health/machine-score-table";
import type { FleetHealth, HostHealth } from "~/features/fleet-health/types";

import { fixture, mountEachTest, one, render } from "./fleet-health-dom";

mountEachTest();

const table = (data: FleetHealth) => createElement(MachineScoreTable, { data, window: "24h" });
const liveHost = (data: FleetHealth): HostHealth & { live: NonNullable<HostHealth["live"]> } => {
  const host = data.hosts.find((candidate) => candidate.live !== null);
  if (!host || host.live === null) throw new Error("the fixture has a machine with a live part");
  return host as HostHealth & { live: NonNullable<HostHealth["live"]> };
};
const runningText = (hostId: string) => one(`#machine-${hostId}`).querySelector("[data-running-count]");

describe("the running count on each machine row", () => {
  it("sums the runners running by category on an online machine", async () => {
    const data = fixture();
    const host = liveHost(data);
    host.live.online = true;
    host.live.runningByCategory = { test: 3, build: 0, check: 0, deploy: 1, "fix-agent": 0, other: 2 };
    await render(table(data));

    const count = runningText(host.id);
    expect(count?.getAttribute("data-running-count")).toBe("6");
    expect(count?.textContent).toBe("6 running");
  });

  it("says 0 running on an online machine with nothing running", async () => {
    const data = fixture();
    const host = liveHost(data);
    host.live.online = true;
    host.live.runningByCategory = { test: 0, build: 0, check: 0, deploy: 0, "fix-agent": 0, other: 0 };
    await render(table(data));

    expect(runningText(host.id)?.textContent).toBe("0 running");
  });

  it("shows no count on an offline machine or one with no live part", async () => {
    const data = fixture();
    const host = liveHost(data);
    host.live.online = false;
    const unknown = data.hosts.find((candidate) => candidate.id !== host.id);
    if (!unknown) throw new Error("the fixture has a second machine");
    unknown.live = null;
    await render(table(data));

    expect(runningText(host.id)).toBeNull();
    expect(runningText(unknown.id)).toBeNull();
  });
});
