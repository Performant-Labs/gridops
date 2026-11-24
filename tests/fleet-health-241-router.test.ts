// @vitest-environment jsdom
/**
 * #241 items 2 and 6 on the page's own path: once the machine expansion lives in the page frame
 * and follows the router's hash, a deep link opened on load still expands its machine and the
 * detail slot names it, and a hash that names no machine (or is malformed) expands nothing and
 * leaves the slot's prompt. The in-page concern link itself is
 * `fleet-health-202-detail.test.ts`'s "opens the machine when an in-page concern link is clicked".
 *
 * Rendered under a real router (`./fleet-health-router`), so this file has no router `vi.mock`.
 */
import { describe, expect, it } from "vitest";

import type { FleetHealth } from "~/features/fleet-health/types";

import { all, fixture, mountEachTest, one } from "./fleet-health-dom";
import { renderHealthPageAt, resetLocationEachTest } from "./fleet-health-router";

mountEachTest();
resetLocationEachTest();

const PROMPT = "Expand a machine above to see what it is doing now and what it ran in the window.";
const slot = () => one('[data-health-section="machine-detail"]');
const details = () => all("[data-machine-detail]").map((detail) => detail.getAttribute("data-machine-detail"));
const hostByName = (data: FleetHealth, name: string) => {
  const host = data.hosts.find((candidate) => candidate.name === name);
  if (!host) throw new Error(`the fixture has a machine named ${name}`);
  return host;
};

describe("the page's machine expansion", () => {
  it("opens a deep-linked machine on load, and the detail slot names it", async () => {
    const data = fixture();
    const callisto = hostByName(data, "callisto");
    await renderHealthPageAt(`/fleet/health#machine-${callisto.id}`, data);

    expect(one("button[aria-expanded]", one(`#machine-${callisto.id}`)).getAttribute("aria-expanded")).toBe("true");
    expect(details()).toEqual([callisto.id]);
    expect(slot().textContent).toContain(callisto.name);
    expect(slot().textContent).not.toContain(PROMPT);
  });

  it.each([
    ["names no machine", "#machine-not-a-machine"],
    ["is malformed", "#machine-%E0%A4"],
  ])("expands nothing when the hash %s, and the slot keeps its prompt", async (_case, hash) => {
    const data = fixture();
    await renderHealthPageAt(`/fleet/health${hash}`, data);

    expect(all('[id^="machine-"]')).toHaveLength(data.hosts.length);
    expect(all('button[aria-expanded="true"]')).toHaveLength(0);
    expect(details()).toEqual([]);
    expect(slot().textContent).toContain(PROMPT);
  });
});
