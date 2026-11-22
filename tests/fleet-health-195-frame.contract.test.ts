// @vitest-environment jsdom
/**
 * Contract tests for #195 (page frame, Areas of concern, fleet score card), written by #197.
 *
 * #195 turns each `it.skip("[contract: #195] …")` into `it("…")` and makes it pass; it changes
 * nothing else here (`Fleet health guard`). The components take `{ data, window }` (the parsed
 * `FleetHealth` and the selected window) and mark what the tests read:
 * - Areas of concern: one row per concern with `data-concern-key`, in the order the API gives
 *   (worst first, then oldest), at most 5 until "Show all N"; "No areas of concern" when empty;
 *   rows keyed by `key`, so a refresh keeps each row's element; a concern that wasn't in the
 *   previous data is marked `data-new="true"` (nothing is on the first render).
 * - Fleet score card: the score's element carries `data-band` (`scoreBand`); each component row
 *   has `data-component-key`, and `data-insufficient="true"` when it has no score.
 * - The page remembers the window: `saveHealthWindow` / `loadHealthWindow` (default `24h`, and
 *   `24h` when storage fails).
 */
import { createElement } from "react";
import { afterEach, describe, expect, it, vi } from "vitest";

import { AreasOfConcern } from "~/features/fleet-health/areas-of-concern";
import { scoreBand } from "~/features/fleet-health/chart-theme";
import { FleetScoreCard } from "~/features/fleet-health/fleet-score-card";
import { loadHealthWindow, saveHealthWindow } from "~/features/fleet-health/page";
import type { FleetHealth } from "~/features/fleet-health/types";

import { all, button, click, fixture, one, render, mountEachTest } from "./fleet-health-dom";

vi.mock("@tanstack/react-router", async () => {
  const { createElement: element } = await import("react");
  return {
    Link: ({ children, to, ...props }: { children?: unknown; to: string } & Record<string, unknown>) =>
      element("a", { ...props, href: to }, children as never),
  };
});

mountEachTest();

const keys = () => all("[data-concern-key]").map((row) => row.getAttribute("data-concern-key"));

const concerns = (data: FleetHealth) => createElement(AreasOfConcern, { data, window: "24h" });

describe("Areas of concern", () => {
  it.skip("[contract: #195] lists the first five concerns in the API's order, then all of them on Show all", async () => {
    const data = fixture();
    const total = data.concerns.length;
    expect(total).toBeGreaterThan(5);
    await render(concerns(data));
    expect(keys()).toEqual(data.concerns.slice(0, 5).map((concern) => concern.key));
    await click(button(`Show all ${total}`));
    expect(keys()).toEqual(data.concerns.map((concern) => concern.key));
  });

  it.skip("[contract: #195] says so when there is nothing to show", async () => {
    await render(concerns({ ...fixture(), concerns: [] }));
    expect(keys()).toEqual([]);
    expect(document.body.textContent).toContain("No areas of concern");
  });

  it.skip("[contract: #195] keeps each row by key across a refresh and highlights only the new concern", async () => {
    const data = fixture();
    await render(concerns(data));
    expect(all('[data-new="true"]')).toHaveLength(0);
    const kept = data.concerns[1];
    if (!kept) throw new Error("the fixture has a second concern");
    const before = one(`[data-concern-key="${kept.key}"]`);

    const first = data.concerns[0];
    if (!first) throw new Error("the fixture has a first concern");
    const added = { ...first, key: "host-offline:new-machine", title: "New machine is offline", since: null };
    await render(concerns({ ...data, concerns: [added, ...data.concerns] }));
    expect(keys()[0]).toBe("host-offline:new-machine");
    expect(one(`[data-concern-key="${kept.key}"]`)).toBe(before);
    expect(all('[data-new="true"]').map((row) => row.getAttribute("data-concern-key"))).toEqual(["host-offline:new-machine"]);
  });
});

describe("Fleet score card", () => {
  it.skip("[contract: #195] colours the score by its band", async () => {
    for (const score of [95, 90, 75, 40, null]) {
      const data = fixture();
      data.fleet.score = score;
      await render(createElement(FleetScoreCard, { data, window: "24h" }));
      expect(one("[data-band]").getAttribute("data-band")).toBe(scoreBand(score));
    }
  });

  it.skip("[contract: #195] greys out a component that has not enough data", async () => {
    const data = fixture();
    const [first, ...rest] = data.fleet.components;
    if (!first) throw new Error("the fixture's fleet has components");
    first.score = null;
    for (const component of rest) component.score = 80;
    await render(createElement(FleetScoreCard, { data, window: "24h" }));
    expect(all("[data-component-key]").map((row) => row.getAttribute("data-component-key"))).toEqual(
      data.fleet.components.map((component) => component.key),
    );
    expect(all('[data-insufficient="true"]').map((row) => row.getAttribute("data-component-key"))).toEqual([first.key]);
  });
});

describe("the remembered window", () => {
  afterEach(() => window.localStorage.clear());

  it.skip("[contract: #195] remembers the chosen window, 24h by default and when storage fails", () => {
    expect(loadHealthWindow()).toBe("24h");
    saveHealthWindow("7d");
    expect(loadHealthWindow()).toBe("7d");
    saveHealthWindow("1h");
    expect(loadHealthWindow()).toBe("1h");
    window.localStorage.setItem("gridops.fleetHealth.window", "30d");
    expect(loadHealthWindow()).toBe("24h");
    vi.spyOn(Storage.prototype, "getItem").mockImplementation(() => {
      throw new Error("storage is off");
    });
    expect(loadHealthWindow()).toBe("24h");
  });
});
