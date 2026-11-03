// @vitest-environment jsdom
/**
 * The Overview's Tart host card (#95 phase 2): the Mac's macOS VM slots, who holds each, the pools
 * waiting for one, and the Tart agent's cap warning. The bodies come from the contract fixture the
 * API's tests produce (`crates/gridops-api/tests/fleet_http/macos_vm_slots.rs`), so the card is
 * rendered from exactly what the API answers.
 */
import {
  RouterProvider, createMemoryHistory, createRootRoute, createRouter,
} from "@tanstack/react-router";
import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it } from "vitest";

import fixtures from "./fixtures/tart-host-contracts.json";
import { TartHostCard } from "../src/features/runner-pools/tart-host-card";
import {
  formatWaited, showsTartHost, slotRows, tartHostSchema, type TartHost,
} from "../src/features/runner-pools/tart-host";

declare global {
  var IS_REACT_ACT_ENVIRONMENT: boolean | undefined;
}

/** When the fixture's slots were read: five minutes after its first runner started. */
const NOW = Date.parse("2026-11-03T16:05:00Z");

function example(name: string): TartHost {
  const found = fixtures.find((item) => item.name.startsWith(name));
  if (!found) throw new Error(`no fixture example "${name}"`);
  return tartHostSchema.parse(found.body);
}

let container: HTMLDivElement;
let root: Root;

beforeEach(() => {
  globalThis.IS_REACT_ACT_ENVIRONMENT = true;
  container = document.createElement("div");
  document.body.append(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
});

/** Renders the card under a router, as the Overview does, so its pool and runner links resolve. */
async function render(host: TartHost | null) {
  const router = createRouter({
    history: createMemoryHistory({ initialEntries: ["/"] }),
    routeTree: createRootRoute({ component: () => createElement(TartHostCard, { host, now: NOW }) }),
  });
  await act(async () => {
    await router.load();
    root.render(createElement(RouterProvider, { router: router as never }));
  });
}

function rows(label: string) {
  return [...container.querySelectorAll(`[aria-label="${label}"] > li`)].map((row) => row.textContent ?? "");
}

function link(text: string) {
  return [...container.querySelectorAll("a")].find((anchor) => anchor.textContent === text);
}

describe("the Tart host contract", () => {
  it("parses every body the API's tests produce, keeping every field", () => {
    expect(fixtures.length).toBeGreaterThanOrEqual(5);
    for (const item of fixtures) {
      expect(tartHostSchema.parse(item.body), item.name).toEqual(item.body);
    }
  });

  it("refuses a holder of a kind it does not know", () => {
    const body = structuredClone(fixtures[0]!.body) as { slots: { holders: unknown[] } };
    body.slots.holders.push({ kind: "suspended", name: "x" });
    expect(tartHostSchema.safeParse(body).success).toBe(false);
  });
});

describe("TartHostCard", () => {
  it("shows the limit, each holder with its pool, runner and since when, and the queue in order", async () => {
    await render(example("slots held by a GridOps runner"));
    expect(container.querySelector('[aria-label="Tart host"]')).not.toBeNull();
    expect(container.textContent).toContain("macOS VM slots on the Tart host2");
    expect(container.textContent).toContain("2 of 2 in use");

    const slots = rows("Slots");
    expect(slots).toHaveLength(2);
    expect(slots[0]).toContain("GridOps runner");
    expect(slots[0]).toContain("macos-release · macos-release-1a2b3c4d");
    expect(slots[0]).toContain("5m 0s");
    expect(slots[1]).toContain("Started outside GridOps");
    expect(slots[1]).toContain("runner-warm-1 (ci) · seen as a tart run process");
    expect(link("macos-release")?.getAttribute("href")).toBe("/runner-pools/mac-pool");
    expect(link("macos-release-1a2b3c4d")?.getAttribute("href")).toBe("/live-logs?target=runner-mac-1");

    const queue = rows("Slot queue");
    expect(queue).toEqual([
      expect.stringMatching(/^1macos-release.*waiting 3m 0s$/),
      expect.stringMatching(/^2A pool you can't see.*waiting 1m 30s$/),
    ]);
    expect(container.textContent).not.toContain("The Tart agent allows");
  });

  it("shows a start in progress, a pool the viewer cannot see, and the agent cap warning", async () => {
    await render(example("a runner of a pool the caller cannot see"));
    const slots = rows("Slots");
    expect(slots).toHaveLength(2);
    expect(slots[0]).toContain("GridOps runner");
    expect(slots[0]).toContain("In a pool you can't see");
    expect(slots[0]).toContain("7m 0s");
    expect(slots[1]).toContain("Starting a GridOps runner");
    expect(slots[1]).toContain("macos-release · macos-release-5e6f7a8b");
    expect(slots[1]).toContain("1m 0s");
    expect(container.querySelector('[data-slot="starting"] .live-pulse')).not.toBeNull();
    expect(container.textContent).toContain("The Tart agent allows 1 runner; slots are set to 2");
    expect(container.textContent).toContain("No pool is waiting for a slot.");
    expect(container.querySelector('[aria-label="Slot queue"]')).toBeNull();
  });

  it("lists a runner the agent does not name, and free slots", async () => {
    await render(example("an agent that does not name its runners"));
    const slots = rows("Slots");
    expect(slots).toHaveLength(2);
    expect(slots[0]).toContain("Not identified");
    expect(slots[1]).toBe("Free");
    expect(container.textContent).toContain("1 of 2 in use");
  });

  it("says the Tart agent is unreachable, with the slot setting, and shows no holders", async () => {
    await render(example("the Tart agent is unreachable"));
    expect(container.textContent).toContain("The Tart agent is unreachable");
    expect(container.textContent).toContain("Tart agent is unavailable.");
    expect(container.textContent).toContain("macOS VM slots on the Tart host2");
    expect(container.querySelector('[aria-label="Slots"]')).toBeNull();
    expect(container.textContent).not.toContain("in use");
  });

  it("is hidden without a Tart agent, when the manager cannot be asked, and before data arrives", async () => {
    for (const host of [
      example("no Tart agent is configured"),
      tartHostSchema.parse({ tartAgent: "unknown", message: "The runner manager could not be reached.", slotSetting: 2, slots: null }),
      tartHostSchema.parse({ authenticated: false, tartAgent: "unknown", message: null, slotSetting: null, slots: null }),
      null,
    ]) {
      await render(host);
      expect(container.querySelector('[aria-label="Tart host"]')).toBeNull();
      expect(container.textContent).toBe("");
    }
  });

  it("says so when the agent answered but its slots could not be read", async () => {
    await render(tartHostSchema.parse({ tartAgent: "available", message: null, slotSetting: 1, slots: null }));
    expect(container.textContent).toContain("could not read the Tart agent's VM slots");
    expect(container.textContent).toContain("macOS VM slots on the Tart host1");
  });
});

describe("the card's helpers", () => {
  it("shows the card only for a Tart agent that is there", () => {
    expect(showsTartHost(example("slots held by a GridOps runner"))).toBe(true);
    expect(showsTartHost(example("the Tart agent is unreachable"))).toBe(true);
    expect(showsTartHost(example("no Tart agent is configured"))).toBe(false);
    expect(showsTartHost(null)).toBe(false);
  });

  it("lists every holder even past the limit, and a free row for each slot left", () => {
    const slots = example("slots held by a GridOps runner").slots!;
    expect(slotRows({ ...slots, limit: 1 }).map((row) => row.kind)).toEqual(["runner", "external"]);
    expect(slotRows({ ...slots, holders: [] }).map((row) => row.kind)).toEqual(["free", "free"]);
  });

  it("formats how long a slot has been held or a pool has waited", () => {
    const at = (seconds: number) => new Date(NOW - seconds * 1_000).toISOString();
    expect(formatWaited(at(0), NOW)).toBe("0s");
    expect(formatWaited(at(45), NOW)).toBe("45s");
    expect(formatWaited(at(200), NOW)).toBe("3m 20s");
    expect(formatWaited(at(2 * 3_600 + 300), NOW)).toBe("2h 5m");
    expect(formatWaited(at(3 * 86_400 + 4 * 3_600), NOW)).toBe("3d 4h");
    // A reading from a clock slightly ahead of this one is not negative.
    expect(formatWaited(at(-5), NOW)).toBe("0s");
  });
});
