// @vitest-environment jsdom
/**
 * #241: the fixes to the wave-1 fleet health sections that the Opus review of #227, #231 and
 * #232 found (`docs/handoffs/241-brief.md`). One `describe` per brief item; items 2 and 6 (the
 * in-page link and the detail slot) need a real router and live in
 * `fleet-health-202-detail.test.ts` and `fleet-health-241-router.test.ts`.
 */
import { act, createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { afterEach, describe, expect, it, vi } from "vitest";

import { RunStatusIcon, type RunState } from "~/components/status-icon";
import { AreasOfConcern } from "~/features/fleet-health/areas-of-concern";
import { FleetScoreCard } from "~/features/fleet-health/fleet-score-card";
import { LiveStrip } from "~/features/fleet-health/live-strip";
import * as machineDetail from "~/features/fleet-health/machine-detail";
import { MachineScoreTable } from "~/features/fleet-health/machine-score-table";
import type { FleetHealth, ScoreComponent, Severity } from "~/features/fleet-health/types";

import { all, button, click, fixture, mountEachTest, one, render, settle } from "./fleet-health-dom";

vi.mock("@tanstack/react-router", async () => {
  const { createElement: element } = await import("react");
  return {
    Link: ({ children, to, ...props }: { children?: unknown; to: string } & Record<string, unknown>) =>
      element("a", { ...props, href: to }, children as never),
  };
});

mountEachTest();

afterEach(() => {
  vi.useRealTimers();
  window.location.hash = "";
});

const hostByName = (data: FleetHealth, name: string) => {
  const host = data.hosts.find((candidate) => candidate.name === name);
  if (!host) throw new Error(`the fixture has a machine named ${name}`);
  return host;
};

/** The text of every element in `scope` with no element children: what each leaf shows. */
const leafTexts = (scope: Element) =>
  all("*", scope).filter((element) => element.childElementCount === 0).map((element) => element.textContent?.trim() ?? "");

describe("item 1: a concern's severity is not colour alone", () => {
  // The brief's map: the same red, amber and grey the rows used, as three different glyphs.
  const SEVERITY_STATES: Record<Severity, RunState> = { critical: "failure", warning: "waiting", info: "skipped" };
  const SEVERITY_TEXT: Record<Severity, string> = { critical: "Critical", warning: "Warning", info: "Info" };

  /** The children of the `svg` `RunStatusIcon` draws for `status`, rendered on its own. */
  const glyph = (status: RunState) => {
    const holder = document.createElement("div");
    holder.innerHTML = renderToStaticMarkup(createElement(RunStatusIcon, { status }));
    return one("svg", holder).innerHTML;
  };

  it("reads each row's severity out, and draws its severity's glyph", async () => {
    const data = fixture();
    expect(new Set(data.concerns.map((concern) => concern.severity))).toEqual(new Set(["critical", "warning", "info"]));
    await render(createElement(AreasOfConcern, { data, window: "24h" }));
    await click(button(`Show all ${data.concerns.length}`));

    const rows = all("[data-concern-key]");
    expect(rows).toHaveLength(data.concerns.length);
    for (const row of rows) {
      const concern = data.concerns.find((candidate) => candidate.key === row.getAttribute("data-concern-key"));
      if (!concern) throw new Error(`no concern for row ${row.getAttribute("data-concern-key")}`);
      const where = `row ${concern.key} (${concern.severity})`;

      // (a) The primary criterion: what a screen reader announces for the row's severity.
      expect(all(".sr-only", row).map((hidden) => hidden.textContent), where).toEqual([SEVERITY_TEXT[concern.severity]]);

      // (b) The secondary check: the row draws the glyph its severity maps to. This pins the
      // visual severity mapping, not the SVG's structure; the children are compared because the
      // `svg`'s own width, height and class come from the props the row passes.
      const icons = all("svg", row);
      expect(icons, where).toHaveLength(1);
      const [icon] = icons;
      expect(icon?.innerHTML, where).toBe(glyph(SEVERITY_STATES[concern.severity]));
      expect(icon?.getAttribute("aria-hidden"), where).toBe("true");
    }
  });
});

describe("items 3 and 4: the score card's weights and no-data reasons", () => {
  const INSUFFICIENT_REASON = "Not enough data in this window: fewer than 20 jobs, or no samples.";
  const NO_DATA_REASON = "Not enough data: only 12 jobs in this window (needs 20).";

  const component = (key: string, weight: number, score: number | null, deduction: string | null): ScoreComponent => ({
    key,
    label: `Label ${key}`,
    weight,
    score,
    deduction,
  });

  async function renderCard(components: ScoreComponent[]) {
    const data = fixture();
    data.fleet.components = components;
    await render(createElement(FleetScoreCard, { data, window: "24h" }));
  }

  const row = (key: string) => one(`[data-component-key="${key}"]`);

  it("shows each weight as a whole percent, and a dash for a component with no score", async () => {
    await renderCard([
      component("fractional", 47.05882352941177, 61, "Queue p90 was 4 min 30 s against a 1 min target: −16."),
      component("whole", 25, 87, null),
      component("no-score", 27.94117647058823, null, NO_DATA_REASON),
    ]);
    expect(leafTexts(row("fractional"))).toContain("47%");
    expect(row("fractional").textContent).not.toContain("47.0588");
    expect(leafTexts(row("whole"))).toContain("25%");
    // Exactly U+2014, with no "%", whatever the weight.
    expect(leafTexts(row("no-score"))).toContain("—");
    expect(row("no-score").textContent).not.toContain("%");
  });

  it("gives the API's reason for a component with no score, on its line and in its tooltip", async () => {
    await renderCard([component("no-score", 15, null, NO_DATA_REASON), component("scored", 85, 90, null)]);
    expect(row("no-score").textContent).toContain(NO_DATA_REASON);
    expect(row("no-score").textContent).not.toContain(INSUFFICIENT_REASON);

    // The row is the tooltip's trigger and takes focus for keyboard users.
    await act(async () => {
      row("no-score").focus();
    });
    await settle();
    const tooltip = one('[role="tooltip"]');
    expect(tooltip.textContent).toBe(NO_DATA_REASON);
  });

  it("falls back to the card's own text when the API gives no reason", async () => {
    await renderCard([component("null-reason", 50, null, null), component("empty-reason", 50, null, "")]);
    expect(row("null-reason").textContent).toContain(INSUFFICIENT_REASON);
    expect(row("empty-reason").textContent).toContain(INSUFFICIENT_REASON);
  });
});

describe("item 4: the machine table's no-data bar titles", () => {
  it("titles a no-data bar with the API's reason, and falls back to the table's own text", async () => {
    const data = fixture();
    const macBook = hostByName(data, "MacBookPro");
    const [withReason, nullReason, emptyReason] = macBook.components;
    if (!withReason || !nullReason || !emptyReason) throw new Error("MacBookPro has five components");
    expect([withReason.score, nullReason.score, emptyReason.score]).toEqual([null, null, null]);
    expect(withReason.deduction).toMatch(/^Not enough data: /);
    nullReason.deduction = null;
    emptyReason.deduction = "";
    await render(createElement(MachineScoreTable, { data, window: "24h" }));

    const bar = (hostId: string, key: string) => one(`[data-component-key="${key}"]`, one(`#machine-${hostId}`));
    expect(bar(macBook.id, withReason.key).getAttribute("title")).toBe(`${withReason.label}: ${withReason.deduction}`);
    expect(bar(macBook.id, nullReason.key).getAttribute("title")).toBe(`${nullReason.label}: not enough data`);
    expect(bar(macBook.id, emptyReason.key).getAttribute("title")).toBe(`${emptyReason.label}: not enough data`);

    // A scored bar keeps its score.
    const io = hostByName(data, "io");
    const [scored] = io.components;
    if (!scored || scored.score === null) throw new Error("io's first component has a score");
    expect(bar(io.id, scored.key).getAttribute("title")).toBe(`${scored.label}: ${scored.score}`);
  });
});

describe("item 5: CPU and memory as stacked bars", () => {
  type Segments = { usedWithin: number; usedOver: number; reservedUnused: number; free: number; scale: number };
  type Resource = { budget: number; reserved: number; used: number | null };

  /** `resourceSegments`, read through the module so a missing export fails as an assertion. */
  const segments = (resource: Resource): Segments => {
    const exported = (machineDetail as Record<string, unknown>).resourceSegments;
    expect(typeof exported, "machine-detail exports resourceSegments").toBe("function");
    return (exported as (resource: Resource) => Segments)(resource);
  };

  it("splits reserved, used and budget into segments that never overlap", () => {
    // Used inside its reservation: used, the rest of the reservation, the free budget.
    expect(segments({ budget: 10_000, reserved: 6_000, used: 2_500 })).toEqual({ usedWithin: 2_500, usedOver: 0, reservedUnused: 3_500, free: 4_000, scale: 10_000 });
    // Used past its reservation (the bug case): all of the reservation, then the excess.
    expect(segments({ budget: 16_384, reserved: 4_096, used: 6_144 })).toEqual({ usedWithin: 4_096, usedOver: 2_048, reservedUnused: 0, free: 10_240, scale: 16_384 });
    // No used reading: the whole reservation is unused.
    expect(segments({ budget: 10_000, reserved: 6_000, used: null })).toEqual({ usedWithin: 0, usedOver: 0, reservedUnused: 6_000, free: 4_000, scale: 10_000 });
    // Demand past the budget: no free budget, and the axis grows to the demand.
    expect(segments({ budget: 10_000, reserved: 12_000, used: 11_000 })).toEqual({ usedWithin: 11_000, usedOver: 0, reservedUnused: 1_000, free: 0, scale: 12_000 });
    // Nothing at all (a machine with no budget): an axis of 1, so nothing divides by zero.
    expect(segments({ budget: 0, reserved: 0, used: null })).toEqual({ usedWithin: 0, usedOver: 0, reservedUnused: 0, free: 0, scale: 1 });

    // The reservation is always readable as a width, and the segments add up to the demand.
    const cases: Resource[] = [
      { budget: 10_000, reserved: 6_000, used: 2_500 },
      { budget: 16_384, reserved: 4_096, used: 6_144 },
      { budget: 10_000, reserved: 6_000, used: null },
      { budget: 10_000, reserved: 12_000, used: 11_000 },
      { budget: 8_000, reserved: 4_000, used: 4_000 },
    ];
    for (const resource of cases) {
      const split = segments(resource);
      expect(split.usedWithin + split.reservedUnused, JSON.stringify(resource)).toBe(resource.reserved);
      expect(split.usedWithin + split.usedOver + split.reservedUnused + split.free, JSON.stringify(resource))
        .toBe(Math.max(resource.budget, resource.reserved, resource.used ?? 0));
    }
  });

  // jsdom has no layout, so a chart only draws once its container reports a size.
  class SizedResizeObserver {
    constructor(private readonly callback: ResizeObserverCallback) {}
    observe(target: Element) {
      this.callback([{ target, contentRect: { width: 400, height: 40 } } as unknown as ResizeObserverEntry], this as unknown as ResizeObserver);
    }
    unobserve() {}
    disconnect() {}
  }

  it("draws each resource as one row of stacked, non-overlapping segments", async () => {
    vi.stubGlobal("ResizeObserver", SizedResizeObserver);
    const data = fixture();
    const io = hostByName(data, "io");
    // io's memory is used past its reservation (21000 of 18432 MiB), the case that hid reserved.
    expect(io.live?.memoryUsedMib).toBeGreaterThan(io.live?.memoryReservedMib ?? Infinity);
    await render(createElement(MachineScoreTable, { data, window: "24h" }));
    await click(one<HTMLButtonElement>("button[aria-expanded]", one(`#machine-${io.id}`)));
    await settle();
    const now = one('section[aria-label="Now"]', one(`[data-machine-detail="${io.id}"]`));

    // One bar row per resource (CPU, memory), whether the charts are one or two.
    const barRows = all(".recharts-wrapper", now).flatMap((chart) => {
      const byRow = new Map<string, { x: number; width: number }[]>();
      for (const rectangle of all(".recharts-bar-rectangle .recharts-rectangle", chart)) {
        const width = Number(rectangle.getAttribute("width"));
        if (!(width > 0)) continue;
        const key = rectangle.getAttribute("y") ?? "";
        byRow.set(key, [...(byRow.get(key) ?? []), { x: Number(rectangle.getAttribute("x")), width }]);
      }
      return [...byRow.values()];
    });
    expect(barRows, "a stacked bar for CPU and one for memory").toHaveLength(2);
    for (const row of barRows) {
      expect(row.length, "at least two segments").toBeGreaterThanOrEqual(2);
      const sorted = [...row].sort((a, b) => a.x - b.x);
      sorted.slice(1).forEach((segment, index) => {
        const previous = sorted[index];
        if (!previous) return;
        expect(segment.x, "a segment starts where the previous one ends").toBeGreaterThanOrEqual(previous.x + previous.width - 0.01);
      });
    }

    // A hatch is optional; when one is drawn it takes theme colours only, and each chart's
    // pattern has its own id (CPU and memory render in the same detail).
    const patterns = all("pattern", now);
    expect(new Set(patterns.map((pattern) => pattern.id)).size).toBe(patterns.length);
    for (const pattern of patterns) {
      for (const painted of [pattern, ...all("*", pattern)]) {
        for (const attribute of ["fill", "stroke"]) {
          const value = painted.getAttribute(attribute);
          if (value !== null) expect(value, `${attribute} in the hatch`).toMatch(/^(var\(--[\w-]+\)|none)$/);
        }
      }
    }

    // Each bar is one image read out by its label: nothing inside it is a tab stop or
    // claims a role of its own (Recharts' accessibility layer would add a `tabindex="0"`
    // `role="application"` svg per bar).
    const images = all('[role="img"]', now);
    expect(images, "one labelled image per resource").toHaveLength(2);
    for (const image of images) {
      expect(all(`[tabindex]:not([tabindex^="-"]), [role]`, image).map((inner) => `${inner.tagName} ${inner.getAttribute("role")} ${inner.getAttribute("tabindex")}`), `inside ${image.getAttribute("aria-label")}`).toEqual([]);
    }
  });
});

describe("item 7: the oldest wait is read against the clock", () => {
  const MINUTE = 60_000;

  async function renderStripAt(now: number, oldest: { queuedAt: number; waitMs: number }) {
    vi.useFakeTimers({ now, toFake: ["Date"] });
    const data = fixture();
    const queued = data.live.oldestQueued;
    if (!queued) throw new Error("the fixture has an oldest queued job");
    data.live.oldestQueued = { ...queued, ...oldest };
    await render(createElement(LiveStrip, { data, window: "24h" }));
    return one('a[href="/queued-jobs"]').textContent ?? "";
  }

  it("shows how long the job has waited by now, never less than the response said", async () => {
    const queuedAt = Date.UTC(2026, 10, 24, 12, 0, 0);
    // A cached or stale response: `waitMs` was 45 s when it was built, but the job has now
    // waited 8 minutes.
    expect(await renderStripAt(queuedAt + 8 * MINUTE, { queuedAt, waitMs: 45_000 })).toContain("Oldest job waited 8 min");
    // The viewer's clock behind the server's (queued "in the future"): the response's wait stands.
    expect(await renderStripAt(queuedAt - MINUTE, { queuedAt, waitMs: 45_000 })).toContain("Oldest job waited 45 s");
  });
});
