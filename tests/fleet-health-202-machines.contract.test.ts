// @vitest-environment jsdom
/**
 * Contract tests for #202 (machine score table and detail), written by #197.
 *
 * #202 turns each `it.skip("[contract: #202] …")` into `it("…")` and makes it pass; it changes
 * nothing else here (`Fleet health guard`). `MachineScoreTable` takes `{ data, window }` and marks:
 * - one row per machine, `id="machine-<hostId>"`, worst score first (no score last, then by
 *   name), with a button carrying `aria-expanded` that opens its detail;
 * - each runner size's speed index in an element with `data-speed-cpu="<cpuMillis>"`, reading
 *   `<index to 2 places>×`, or "Not enough data" without an index;
 * - the open detail, `data-machine-detail="<hostId>"`, with the per-category table: one
 *   `data-category="<category>"` row per entry of the host's `categories`, showing its job count;
 * - `#machine-<hostId>` in the URL on load opens that machine's detail.
 */
import { createElement } from "react";
import { afterEach, describe, expect, it, vi } from "vitest";

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

afterEach(() => {
  window.location.hash = "";
});

type Host = FleetHealth["hosts"][number];

/** Worst first; a machine with no score last; ties by name. */
const worstFirst = (hosts: Host[]) =>
  [...hosts].sort((a, b) => {
    if (a.score === null || b.score === null) {
      if (a.score === b.score) return a.name.localeCompare(b.name);
      return a.score === null ? 1 : -1;
    }
    return a.score - b.score || a.name.localeCompare(b.name);
  });

const rowIds = () => all('[id^="machine-"]').map((row) => row.id);
const table = (data: FleetHealth) => createElement(MachineScoreTable, { data, window: "24h" });
const toggle = (hostId: string) => one<HTMLButtonElement>("button[aria-expanded]", one(`#machine-${hostId}`));

describe("machine score table", () => {
  it.skip("[contract: #202] sorts the machines worst first", async () => {
    const data = fixture();
    await render(table(data));
    expect(rowIds()).toEqual(worstFirst(data.hosts).map((host) => `machine-${host.id}`));
  });

  it.skip("[contract: #202] shows each runner size's speed index", async () => {
    const data = fixture();
    await render(table(data));
    for (const host of data.hosts) {
      for (const size of host.speed) {
        const shown = one(`[data-speed-cpu="${size.cpuMillis}"]`, one(`#machine-${host.id}`)).textContent ?? "";
        expect(shown).toContain(size.index === null ? "Not enough data" : `${size.index.toFixed(2)}×`);
      }
    }
  });

  it.skip("[contract: #202] expands a row into its detail with the per-category table", async () => {
    const data = fixture();
    const host = data.hosts.find((candidate) => candidate.categories.length > 0);
    if (!host) throw new Error("the fixture has a machine with categories");
    await render(table(data));
    expect(all("[data-machine-detail]")).toHaveLength(0);
    await click(toggle(host.id));
    expect(toggle(host.id).getAttribute("aria-expanded")).toBe("true");
    const detail = one(`[data-machine-detail="${host.id}"]`);
    const rows = all("[data-category]", detail);
    expect(rows.map((row) => row.getAttribute("data-category"))).toEqual(host.categories.map((entry) => entry.category));
    host.categories.forEach((entry, index) => {
      expect(rows[index]?.textContent ?? "").toContain(String(entry.jobs));
    });
    await click(toggle(host.id));
    expect(all("[data-machine-detail]")).toHaveLength(0);
  });

  it.skip("[contract: #202] opens the machine named in the #machine- link", async () => {
    const data = fixture();
    const target = worstFirst(data.hosts)[2];
    if (!target) throw new Error("the fixture has five machines");
    window.location.hash = `#machine-${target.id}`;
    await render(table(data));
    expect(all("[data-machine-detail]").map((detail) => detail.getAttribute("data-machine-detail"))).toEqual([target.id]);
  });
});
