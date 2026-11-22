// @vitest-environment jsdom
/**
 * Contract tests for #203 (live strip, category legend, Overview tile), written by #197.
 *
 * #203 turns each `it.skip("[contract: #203] …")` into `it("…")` and makes it pass; it changes
 * nothing else here (`Fleet health guard`). The existing `metric-tiles` tests stay as they are.
 * - `LiveStrip` takes `{ data, window }`: the totals in `data-testid="live-queued-total"` and
 *   `"live-running-total"`, and one element per category with jobs, `data-queued-category` /
 *   `data-running-category="<category>"`, whose text leads with its count;
 * - `formatOldestWait(ms)`: whole seconds under a minute ("45 s"), whole minutes under an hour
 *   ("12 min"), hours and minutes after ("3 h 5 min", "1 h");
 * - `metricTiles(metrics, health)` adds a "Fleet health" tile to `/fleet/health`: the fleet
 *   score, a tone per band (good `success`, fair `warning`, poor `danger`, none `neutral`), and a
 *   hint counting critical concerns.
 */
import { createElement } from "react";
import { describe, expect, it, vi } from "vitest";

import { metricTiles } from "~/features/dashboard/metric-tiles";
import type { DashboardOverview } from "~/features/dashboard/types";
import { LiveStrip, formatOldestWait } from "~/features/fleet-health/live-strip";
import type { FleetHealth } from "~/features/fleet-health/types";

import { all, fixture, one, render, mountEachTest } from "./fleet-health-dom";

vi.mock("@tanstack/react-router", async () => {
  const { createElement: element } = await import("react");
  return {
    Link: ({ children, to, ...props }: { children?: unknown; to: string } & Record<string, unknown>) =>
      element("a", { ...props, href: to }, children as never),
  };
});

mountEachTest();

const leadingNumber = (element: Element) => Number.parseInt((element.textContent ?? "").replace(/[^\d\s]/g, " ").trim(), 10);
const sum = (values: number[]) => values.reduce((total, value) => total + value, 0);

describe("live strip", () => {
  it.skip("[contract: #203] shows category counts that add up to the totals", async () => {
    const data = fixture();
    await render(createElement(LiveStrip, { data, window: "24h" }));
    const queued = leadingNumber(one('[data-testid="live-queued-total"]'));
    const running = leadingNumber(one('[data-testid="live-running-total"]'));
    expect(queued).toBe(data.live.queuedTotal);
    expect(running).toBe(data.live.runningTotal);
    expect(sum(all("[data-queued-category]").map(leadingNumber))).toBe(queued);
    expect(sum(all("[data-running-category]").map(leadingNumber))).toBe(running);
    for (const [category, count] of Object.entries(data.live.runningByCategory)) {
      if ((count ?? 0) > 0) expect(leadingNumber(one(`[data-running-category="${category}"]`))).toBe(count);
    }
  });

  it.skip("[contract: #203] formats the oldest wait in seconds, minutes and hours", () => {
    expect(formatOldestWait(45_000)).toBe("45 s");
    expect(formatOldestWait(59_999)).toBe("59 s");
    expect(formatOldestWait(60_000)).toBe("1 min");
    expect(formatOldestWait(12 * 60_000 + 30_000)).toBe("12 min");
    expect(formatOldestWait(60 * 60_000)).toBe("1 h");
    expect(formatOldestWait((3 * 60 + 5) * 60_000)).toBe("3 h 5 min");
  });
});

describe("the Overview's Fleet health tile", () => {
  const metrics: DashboardOverview["metrics"] = {
    runners: 2, online: 2, busy: 1, idle: 1, starting: 0, unavailable: 0,
    queuedJobs: 0, queuedWaiting: 0, queuedUnmatched: 0, successRate: null,
  };

  const health = (score: number | null, critical: number): FleetHealth => {
    const data = fixture();
    data.fleet.score = score;
    const template = data.concerns[0];
    if (!template) throw new Error("the fixture has concerns");
    data.concerns = [
      ...Array.from({ length: critical }, (_, index) => ({ ...template, key: `critical-${index}`, severity: "critical" as const })),
      { ...template, key: "a-warning", severity: "warning" as const },
    ];
    return data;
  };

  const tile = (data: FleetHealth) => {
    const found = metricTiles(metrics, data).find((candidate) => candidate.label === "Fleet health");
    if (!found) throw new Error("no Fleet health tile");
    return found;
  };

  it.skip("[contract: #203] takes its tone from the score's band", () => {
    expect(tile(health(95, 0)).tone).toBe("success");
    expect(tile(health(75, 0)).tone).toBe("warning");
    expect(tile(health(40, 0)).tone).toBe("danger");
    expect(tile(health(null, 0)).tone).toBe("neutral");
    expect(tile(health(95, 0)).value).toBe(95);
    expect(tile(health(95, 0)).to).toBe("/fleet/health");
  });

  it.skip("[contract: #203] counts the critical concerns in its hint", () => {
    expect(tile(health(95, 0)).hint).toBe("No critical concerns");
    expect(tile(health(80, 1)).hint).toBe("1 critical concern");
    expect(tile(health(60, 2)).hint).toBe("2 critical concerns");
  });
});
