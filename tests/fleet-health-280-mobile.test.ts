// @vitest-environment jsdom
/**
 * #280 (epic #268): Fleet → Health on a phone. jsdom has no layout, so the layout fixes are
 * checked through the classes and the DOM order that make them; the headless mobile check (#269)
 * checks the rendered page.
 */
import { createElement } from "react";
import { describe, expect, it, vi } from "vitest";

import { CategoryLegend } from "~/features/fleet-health/category-legend";
import { LiveStrip } from "~/features/fleet-health/live-strip";
import { MachineDetail } from "~/features/fleet-health/machine-detail";
import { MachineScoreTable } from "~/features/fleet-health/machine-score-table";
import { FleetHealthPage } from "~/features/fleet-health/page";

import { all, fixture, mountEachTest, one, render, settle } from "./fleet-health-dom";

vi.mock("@tanstack/react-router", async () => {
  const { createElement: element } = await import("react");
  return {
    Link: ({ children, to, ...props }: { children?: unknown; to: string } & Record<string, unknown>) =>
      element("a", { ...props, href: to }, children as never),
    useRouterState: <T>({ select }: { select: (state: { location: { hash: string } }) => T }) => select({ location: { hash: "" } }),
  };
});

mountEachTest();

describe("item 1: a machine's score components are readable without hovering", () => {
  it("lists each component with its name, score and deduction in the machine's detail", async () => {
    const data = fixture();
    const host = data.hosts.find((candidate) => candidate.components.some((component) => component.score !== null));
    if (!host) throw new Error("the fixture has a scored machine");
    await render(createElement(MachineDetail, { data, host, window: "24h" }));

    const section = one('section[aria-label="Score"]', one(`[data-machine-detail="${host.id}"]`));
    for (const component of host.components) {
      const row = one(`[data-detail-component="${component.key}"]`, section);
      expect(row.textContent).toContain(component.label);
      expect(row.textContent).toContain(component.score === null ? "—" : String(Math.round(component.score)));
      const reason = component.deduction ?? (component.score === null ? "Not enough data" : "On target");
      expect(row.textContent).toContain(reason);
    }
  });

  it("says what the small bars on each row are", async () => {
    await render(createElement(MachineScoreTable, { data: fixture(), window: "24h" }));
    expect(one('section[aria-label="Machines"]').textContent).toMatch(/bars are the score's parts/i);
  });
});

describe("item 2: category descriptions wrap on a phone", () => {
  it("truncates a description only from the sm breakpoint up", async () => {
    await render(createElement(CategoryLegend, { data: fixture(), window: "24h" }));
    const descriptions = all("[data-category-description]");
    expect(descriptions.length).toBeGreaterThan(0);
    for (const description of descriptions) {
      const classes = description.className.split(/\s+/);
      expect(classes).not.toContain("truncate");
      expect(classes).toContain("sm:truncate");
    }
  });
});

describe("item 3: each side of Right now has its bar under its own line", () => {
  it("puts the queued bar after the Queued line and the running bar after the Running line", async () => {
    const data = fixture();
    await render(createElement(LiveStrip, { data, window: "24h" }));
    const order = all("[data-testid='live-queued-total'], [data-live-bar], [data-testid='live-running-total']").map(
      (element) => element.getAttribute("data-live-bar") ?? element.getAttribute("data-testid"),
    );
    expect(order).toEqual(["live-queued-total", "queued", "live-running-total", "running"]);
  });

  it("sizes each category's segment by its count, against the busier side", async () => {
    const data = fixture();
    data.live.queuedByCategory = { test: 2, build: 2 };
    data.live.queuedTotal = 4;
    data.live.runningByCategory = { test: 1 };
    data.live.runningTotal = 1;
    await render(createElement(LiveStrip, { data, window: "24h" }));
    const width = (side: string, category: string) =>
      one(`[data-live-bar="${side}"] [data-live-segment="${category}"]`).style.width;
    expect(width("queued", "test")).toBe("50%");
    expect(width("queued", "build")).toBe("50%");
    expect(width("running", "test")).toBe("25%");
    expect(all('[data-live-bar="running"] [data-live-segment]')).toHaveLength(1);
  });
});

describe("item 4: links are at least 24 px tall", () => {
  it("gives the strip's and the legend's links a 24 px minimum height", async () => {
    const data = fixture();
    data.live.queuedUnmatched = 2;
    await render(createElement("div", null, createElement(LiveStrip, { data, window: "24h" }), createElement(CategoryLegend, { data, window: "24h" })));
    const links = all("a");
    expect(links.length).toBeGreaterThanOrEqual(3);
    for (const link of links) expect(link.className.split(/\s+/), link.textContent ?? "").toContain("min-h-6");
  });
});

describe("item 5: the machine detail card shows on two columns only", () => {
  it("hides the detail slot below the lg breakpoint, where the detail is in its row", async () => {
    vi.stubGlobal("fetch", vi.fn(() => Promise.resolve(Response.json({ ...fixture(), window: "24h" }))));
    await render(createElement(FleetHealthPage));
    await settle();
    const classes = one('[data-health-section="machine-detail"]').className.split(/\s+/);
    expect(classes).toContain("hidden");
    expect(classes).toContain("lg:block");
    for (const name of ["machine-table", "live-strip", "category-legend"]) {
      expect(one(`[data-health-section="${name}"]`).className.split(/\s+/)).not.toContain("hidden");
    }
  });
});
