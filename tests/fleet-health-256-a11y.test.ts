// @vitest-environment jsdom
/**
 * #256 and #258: the accessibility defects #204's walkthrough found on the fleet health page, the
 * Overview's metric strip and the job-category settings, each pinned in the DOM. The browser half
 * (axe in light and dark at 360 and 1280 px, colour contrast, clipped chart labels) is
 * `tests/a11y/fleet-health-axe.spec.ts`; the theme's contrast is `tests/theme-contrast.test.ts`.
 */
import { createElement } from "react";
import { describe, expect, it, vi } from "vitest";

import { bandColor, bandTextColor, scoreBand } from "~/components/charts/chart-theme";
import { MetricStrip } from "~/features/dashboard/metric-strip";
import type { DashboardOverview } from "~/features/dashboard/types";
import { FleetScoreCard } from "~/features/fleet-health/fleet-score-card";
import { FleetHealthPage } from "~/features/fleet-health/page";
import { JobCategoriesSection } from "~/features/settings/job-categories-section";

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

/**
 * A control's accessible name as a narrow screen (below Tailwind's `sm`) exposes it: `aria-label`,
 * else its text without what is hidden there (`hidden sm:inline`) or from assistive technology.
 */
function narrowName(element: Element): string {
  const label = element.getAttribute("aria-label")?.trim();
  if (label) return label;
  const text = (node: Node): string => {
    if (node.nodeType === Node.TEXT_NODE) return node.textContent ?? "";
    if (!(node instanceof Element)) return "";
    if (node.getAttribute("aria-hidden") === "true" || node.classList.contains("hidden")) return "";
    return [...node.childNodes].map(text).join("");
  };
  return text(element).replace(/\s+/g, " ").trim();
}

/** Every health GET answers the fixture for its window; returns the fetch mock. */
function serveHealth() {
  const fetcher = vi.fn((path: string) => {
    const window = new URL(path, "http://gridops.test").searchParams.get("window");
    return Promise.resolve(Response.json({ ...fixture(), window }));
  });
  vi.stubGlobal("fetch", fetcher);
  return fetcher;
}

describe("the fleet health page (#256)", () => {
  it("names every button at a phone's width, Refresh included", async () => {
    serveHealth();
    await render(createElement(FleetHealthPage));
    await settle();

    const buttons = all("button");
    expect(buttons.length).toBeGreaterThan(3);
    expect(buttons.filter((button) => narrowName(button) === "").map((button) => button.outerHTML.slice(0, 120))).toEqual([]);
    expect(buttons.some((button) => narrowName(button) === "Refresh")).toBe(true);
  });

  it("starts each window button's name with the text it shows (Label in Name)", async () => {
    serveHealth();
    await render(createElement(FleetHealthPage));
    await settle();

    const picker = one('[role="group"][aria-label="Window"]');
    const names = all("button", picker).map((button) => ({ shown: button.textContent?.trim() ?? "", name: button.getAttribute("aria-label") ?? "" }));
    expect(names.map(({ shown }) => shown)).toEqual(["1h", "24h", "7d"]);
    for (const { shown, name } of names) expect(name.startsWith(shown), `"${name}" does not start with "${shown}"`).toBe(true);
    expect(names.map(({ name }) => name)).toEqual(["1h, 1 hour", "24h, 24 hours", "7d, 7 days"]);
  });

  it("greys a component with no score without fading its text with opacity", async () => {
    const data = fixture();
    const first = data.fleet.components[0];
    if (!first) throw new Error("the fixture has no components");
    data.fleet.components[0] = { ...first, score: null, deduction: null };
    await render(createElement(FleetScoreCard, { data, window: "24h" }));

    const row = one(`[data-component-key="${first.key}"]`);
    expect(row.getAttribute("data-insufficient")).toBe("true");
    const faded = [row, ...all("*", row)].filter((element) => /\bopacity-/.test(element.getAttribute("class") ?? "") || /opacity/.test(element.getAttribute("style") ?? ""));
    expect(faded.map((element) => element.outerHTML.slice(0, 80))).toEqual([]);
  });

  it("writes the band's name in a text colour, not the band's chart colour", async () => {
    // Green and amber are too light for text in the light theme; red reads as text as it is.
    expect(bandTextColor(scoreBand(95))).not.toBe(bandColor(scoreBand(95)));
    expect(bandTextColor(scoreBand(75))).not.toBe(bandColor(scoreBand(75)));
    expect([bandTextColor("good"), bandTextColor("fair"), bandTextColor("poor")]).toEqual(["var(--success-text)", "var(--warning-text)", "var(--danger)"]);

    const data = fixture();
    data.fleet.score = 95;
    await render(createElement(FleetScoreCard, { data, window: "24h" }));
    const label = all("span").find((span) => span.textContent === "Good");
    expect(label?.style.color).toBe("var(--success-text)");
  });
});

const overview = (): DashboardOverview => ({
  authenticated: true,
  metrics: { runners: 2, online: 2, busy: 1, idle: 1, starting: 0, unavailable: 0, queuedJobs: 0, queuedWaiting: 0, queuedUnmatched: 0, successRate: null },
}) as DashboardOverview;

describe("the Overview's metric strip (#256 item 4)", () => {
  const healthRequests = (fetcher: ReturnType<typeof serveHealth>) =>
    fetcher.mock.calls.filter(([path]) => new URL(path, "http://gridops.test").pathname === "/api/v1/fleet/health");

  it("asks a member's Overview for no fleet health", async () => {
    const fetcher = serveHealth();
    await render(createElement(MetricStrip, { admin: false, data: overview() }));
    await settle();

    expect(healthRequests(fetcher)).toEqual([]);
    expect(all("a").map((link) => link.getAttribute("href"))).not.toContain("/fleet/health");
  });

  it("asks an administrator's, and shows the Fleet health tile", async () => {
    const fetcher = serveHealth();
    await render(createElement(MetricStrip, { admin: true, data: overview() }));
    await settle();

    expect(healthRequests(fetcher).length).toBeGreaterThan(0);
    expect(all("a").map((link) => link.getAttribute("href"))).toContain("/fleet/health");
  });
});

describe("the job-category rules' buttons (#258 item 3)", () => {
  it("name the rule they act on, by its number", async () => {
    const rules = [
      { category: "test", field: "job_name", pattern: "test" },
      { category: "build", field: "job_name", pattern: "build" },
      { category: "deploy", field: "workflow_name", pattern: "deploy" },
    ];
    vi.stubGlobal("fetch", vi.fn((path: string) =>
      Promise.resolve(path.endsWith("/preview") ? Response.json({ rules: rules.map(() => ({ matches: 0 })), other: [] }) : Response.json({ rules })),
    ));
    await render(createElement(JobCategoriesSection));
    await settle();

    const names = all("[data-rule-index]").map((row) => all("button", row).map(narrowName));
    expect(names).toEqual([
      ["Move rule 1 up", "Move rule 1 down", "Delete rule 1"],
      ["Move rule 2 up", "Move rule 2 down", "Delete rule 2"],
      ["Move rule 3 up", "Move rule 3 down", "Delete rule 3"],
    ]);
    const every = names.flat();
    expect(new Set(every).size).toBe(every.length);
  });
});
