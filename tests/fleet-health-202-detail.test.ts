// @vitest-environment jsdom
/**
 * #202's own tests for what the contract leaves open: the "Now" readings in an expanded detail,
 * the per-category numbers against the fixture, the slowest job keys, the speed-index lines and
 * the machine-detail slot's state when no machine is expanded.
 */
import { createElement } from "react";
import { describe, expect, it, vi } from "vitest";

import { MachineDetail } from "~/features/fleet-health/machine-detail";
import { slowestKeys, usagePoints } from "~/features/fleet-health/machine-detail-window";
import { MachineScoreTable } from "~/features/fleet-health/machine-score-table";
import type { FleetHealth } from "~/features/fleet-health/types";

import { all, click, fixture, one, render, mountEachTest } from "./fleet-health-dom";

vi.mock("@tanstack/react-router", async () => {
  const { createElement: element } = await import("react");
  return {
    Link: ({ children, to, ...props }: { children?: unknown; to: string } & Record<string, unknown>) =>
      element("a", { ...props, href: to }, children as never),
  };
});

mountEachTest();

const table = (data: FleetHealth) => createElement(MachineScoreTable, { data, window: "24h" });
const toggle = (hostId: string) => one<HTMLButtonElement>("button[aria-expanded]", one(`#machine-${hostId}`));
const hostByName = (data: FleetHealth, name: string) => {
  const host = data.hosts.find((candidate) => candidate.name === name);
  if (!host) throw new Error(`the fixture has a machine named ${name}`);
  return host;
};

/** Renders the table, expands the machine and returns its detail element. */
async function openDetail(data: FleetHealth, hostId: string) {
  await render(table(data));
  await click(toggle(hostId));
  return one(`[data-machine-detail="${hostId}"]`);
}

describe("machine summary rows", () => {
  it("labels each machine's band and live status", async () => {
    const data = fixture();
    await render(table(data));
    expect(one(`#machine-${hostByName(data, "io").id}`).textContent).toContain("Good");
    expect(one(`#machine-${hostByName(data, "Jupiter").id}`).textContent).toContain("Poor");
    expect(one(`#machine-${hostByName(data, "callisto").id}`).textContent).toContain("Fair");
    expect(one(`#machine-${hostByName(data, "MacBookPro").id}`).textContent).toContain("No score");
    expect(one(`#machine-${hostByName(data, "io").id}`).textContent).toContain("Online");
    expect(one(`#machine-${hostByName(data, "europa").id}`).textContent).toContain("Offline");
  });

  it("writes one speed-index line per runner size", async () => {
    const data = fixture();
    const io = hostByName(data, "io");
    await render(table(data));
    expect(one('[data-speed-cpu="1500"]', one(`#machine-${io.id}`)).textContent).toContain("Not enough data at 1.5 CPU");
    expect(one('[data-speed-cpu="2000"]', one(`#machine-${io.id}`)).textContent).toContain("1.01× fleet median at 2 CPU");
    expect(one('[data-speed-cpu="2000"]', one(`#machine-${hostByName(data, "callisto").id}`)).textContent).toContain("1.42× fleet median at 2 CPU");
  });
});

describe("machine detail — now", () => {
  it("shows CPU and memory reserved and used against the budget, and disk free", async () => {
    const data = fixture();
    const detail = await openDetail(data, hostByName(data, "io").id);
    expect(detail.textContent).toContain("9 cores reserved · 7.4 cores used of 22.8 cores");
    expect(detail.textContent).toContain("18 GB reserved · 20.5 GB used of 64 GB");
    expect(detail.textContent).toContain("Disk free: 300 GB of 500 GB");
  });

  it("reports start failures and the runners running by category", async () => {
    const data = fixture();
    const detail = await openDetail(data, hostByName(data, "io").id);
    expect(detail.textContent).toContain("No start failures in the last hour.");
    expect(detail.textContent).toContain("Fix agent · 1");
    const jupiter = await openDetail(data, hostByName(data, "Jupiter").id);
    expect(jupiter.textContent).toContain("3 start failures in the last hour.");
  });
});

describe("machine detail — in the window", () => {
  it("matches the per-category table to the fixture", async () => {
    const data = fixture();
    const detail = await openDetail(data, hostByName(data, "io").id);
    const rows = all("[data-category]", detail);
    expect(rows.map((row) => row.getAttribute("data-category"))).toEqual(["test", "build", "check", "other"]);
    const [test, build] = rows;
    expect(test?.textContent).toContain("150");
    expect(test?.textContent).toContain("5 min");
    expect(test?.textContent).toContain("13 min 20 s");
    expect(build?.textContent).toContain("60");
    expect(build?.textContent).toContain("6 min 15 s");
  });

  it("caps the slowest job keys at five, the ones pushing the index up most first", () => {
    const data = fixture();
    const callisto = slowestKeys(hostByName(data, "callisto"));
    expect(callisto.map((entry) => entry.key)).toEqual(["acme/app / CI / e2e", "acme/app / CI / build", "acme/app / CI / unit"]);
    expect(callisto[0]).toMatchObject({ cpuMillis: 2000, hostMedianMs: 870000, fleetMedianMs: 600000, hostRuns: 14, fleetRuns: 98 });
    const io = slowestKeys(hostByName(data, "io"));
    expect(io).toHaveLength(4);
    expect(io[0]).toMatchObject({ key: "acme/app / CI / lint", cpuMillis: 1500 });
  });

  it("maps the hourly usage onto one point per hour per category", () => {
    const data = fixture();
    const io = usagePoints(hostByName(data, "io"));
    expect(io.categories).toEqual(["test", "build", "check", "other"]);
    expect(io.points).toHaveLength(24);
    expect(io.points[0]).toEqual({ build: 0.63, check: 0.52, hour: 1795129200000, other: 0.84, test: 1.57 });
    const macBook = usagePoints(hostByName(data, "MacBookPro"));
    expect(macBook.categories).toEqual([]);
    expect(macBook.points).toHaveLength(24);
    expect(macBook.points[0]).toEqual({ hour: 1795129200000 });
  });

  it("tells a machine with no data what is missing", async () => {
    const data = fixture();
    const detail = await openDetail(data, hostByName(data, "MacBookPro").id);
    expect(detail.textContent).toContain("No CPU-hours recorded in the window.");
    expect(detail.textContent).toContain("No jobs completed on this machine in the window.");
    expect(detail.textContent).toContain("Not enough data: fewer than 3 job keys qualify.");
  });
});

describe("machine detail slot", () => {
  it("explains itself when no machine is expanded", async () => {
    await render(createElement(MachineDetail, { data: fixture(), window: "24h" }));
    expect(all("[data-machine-detail]")).toHaveLength(0);
    expect(document.body.textContent).toContain("Expand a machine above");
  });
});
