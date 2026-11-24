// @vitest-environment jsdom
/**
 * #203's own tests, beyond the contract (`tests/fleet-health-203-live.contract.test.ts`): the
 * oldest wait's link, the sides that have no jobs, the empty strip, the legend's descriptions,
 * shares and settings link, and when the Fleet health tile exists at all.
 */
import { createElement } from "react";
import { describe, expect, it, onTestFinished, vi } from "vitest";

import { fleetHealthTile, metricTiles } from "~/features/dashboard/metric-tiles";
import type { DashboardOverview } from "~/features/dashboard/types";
import { CategoryLegend } from "~/features/fleet-health/category-legend";
import { LiveStrip, formatOldestWait } from "~/features/fleet-health/live-strip";

import { all, fixture, mountEachTest, one, render } from "./fleet-health-dom";

vi.mock("@tanstack/react-router", async () => {
  const { createElement: element } = await import("react");
  return {
    Link: ({ children, to, ...props }: { children?: unknown; to: string } & Record<string, unknown>) =>
      element("a", { ...props, href: to }, children as never),
  };
});

mountEachTest();

describe("live strip", () => {
  it("links the oldest queued job's wait to the queue", async () => {
    // #241 item 7 reads the wait against the clock: pin it to when the fixture was built, where
    // the clock's wait and `waitMs` agree (7 min).
    const data = fixture();
    vi.useFakeTimers({ now: data.generatedAt, toFake: ["Date"] });
    onTestFinished(() => {
      vi.useRealTimers();
    });
    await render(createElement(LiveStrip, { data, window: "24h" }));
    const link = one('a[href="/queued-jobs"]');
    expect(link.textContent).toContain("7 min");
    expect(link.textContent).toContain("acme/app");
    expect(link.textContent).toContain("integration tests (2/4)");
  });

  it("renders chips only for categories with jobs, and a zero total for an empty side", async () => {
    const data = fixture();
    data.live.queuedByCategory = { test: 2, other: 1 };
    data.live.queuedTotal = 3;
    data.live.runningByCategory = {};
    data.live.runningTotal = 0;
    await render(createElement(LiveStrip, { data, window: "24h" }));
    expect(all("[data-queued-category]").map((chip) => chip.getAttribute("data-queued-category"))).toEqual(["test", "other"]);
    expect(all("[data-running-category]")).toEqual([]);
    expect(one('[data-testid="live-running-total"]').textContent?.trim()).toBe("0");
  });

  it("says nothing is queued or running when both sides are empty", async () => {
    const data = fixture();
    data.live = { ...data.live, queuedTotal: 0, queuedByCategory: {}, runningTotal: 0, runningByCategory: {}, oldestQueued: null };
    await render(createElement(LiveStrip, { data, window: "24h" }));
    expect(document.body.textContent).toContain("Nothing is queued or running right now.");
    expect(one('[data-testid="live-queued-total"]').textContent?.trim()).toBe("0");
    expect(one('[data-testid="live-running-total"]').textContent?.trim()).toBe("0");
  });

  it("formats a wait that is only hours without minutes", () => {
    expect(formatOldestWait(0)).toBe("0 s");
    expect(formatOldestWait(2 * 3_600_000)).toBe("2 h");
    expect(formatOldestWait((2 * 60 + 1) * 60_000)).toBe("2 h 1 min");
  });
});

describe("category legend", () => {
  it("lists every category with its description and its share of the window's CPU time", async () => {
    await render(createElement(CategoryLegend, { data: fixture(), window: "24h" }));
    for (const label of ["Test", "Build", "Check", "Deploy", "Fix agent", "Other"]) {
      expect(document.body.textContent).toContain(label);
    }
    // The fixture's hours: test 217.5 of 530.4 → 41%; fix-agent never carries window hours.
    expect(document.body.textContent).toContain("41% of CPU time");
    expect(document.body.textContent).toContain("0% of CPU time");
  });

  it("links to the job category rules in Settings (#199)", async () => {
    await render(createElement(CategoryLegend, { data: fixture(), window: "24h" }));
    expect(one('a[href="/settings/general"]').textContent).toContain("Edit job categories");
  });

  it("shows no share when the window has no CPU hours", async () => {
    const data = fixture();
    data.fleet.categories = [];
    await render(createElement(CategoryLegend, { data, window: "24h" }));
    expect(document.body.textContent).not.toContain("% of CPU time");
    for (const label of ["Test", "Build", "Check", "Deploy", "Fix agent", "Other"]) {
      expect(document.body.textContent).toContain(label);
    }
  });
});

describe("the Fleet health tile", () => {
  const metrics: DashboardOverview["metrics"] = {
    runners: 0, online: 0, busy: 0, idle: 0, starting: 0, unavailable: 0,
    queuedJobs: 0, queuedWaiting: 0, queuedUnmatched: 0, successRate: null,
  };

  it("appears only once the viewer's health has loaded", () => {
    expect(metricTiles(metrics).map((tile) => tile.label)).not.toContain("Fleet health");
    expect(metricTiles(metrics, fixture()).map((tile) => tile.label)).toContain("Fleet health");
  });

  it("keeps the tiles that were there before first in the strip", () => {
    const labels = metricTiles(metrics, fixture()).map((tile) => tile.label);
    expect(labels.slice(0, 5)).toEqual(["Runners", "Busy", "Idle", "Queued jobs", "Success rate"]);
    expect(labels).toEqual(["Runners", "Busy", "Idle", "Queued jobs", "Success rate", "Fleet health"]);
  });

  it("pluralizes the critical concerns past one", () => {
    const data = fixture();
    const template = data.concerns[0];
    if (!template) throw new Error("the fixture has concerns");
    data.concerns = Array.from({ length: 3 }, (_, index) => ({ ...template, key: `critical-${index}`, severity: "critical" as const }));
    expect(fleetHealthTile(data).hint).toBe("3 critical concerns");
  });

  it("shows a dash and a neutral tone when there is no score", () => {
    const data = fixture();
    data.fleet.score = null;
    data.concerns = data.concerns.filter((concern) => concern.severity !== "critical");
    const tile = fleetHealthTile(data);
    expect(tile.value).toBe("—");
    expect(tile.tone).toBe("neutral");
    expect(tile.hint).toBe("No critical concerns");
  });
});
