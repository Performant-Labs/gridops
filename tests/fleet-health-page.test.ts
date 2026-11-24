// @vitest-environment jsdom
/**
 * #197: `/fleet/health` renders the fixture through the page frame (diff gate B-2, A warn 28).
 *
 * `page.tsx` is #195's file and every section is filled in by its own wave-1 issue, so this test
 * pins only what survives wave 1: the composition's slots, the window the page asks for, and the
 * members-only and failure states. It never asserts a section's placeholder text ("Coming in
 * #N") or the sections' order, both of which wave 1 changes.
 *
 * The slots: `HealthSections` wraps each section in an element carrying
 * `data-health-section="<slot>"`, one per section, as `docs/design/fleet-health.md` ("The page")
 * lists them. Wave 1 fills a slot's content and may move the slot; it keeps the attribute.
 */
import { createElement } from "react";
import { describe, expect, it, vi } from "vitest";

import { FleetHealthPage } from "~/features/fleet-health/page";

import { all, button, click, fixture, mountEachTest, render, settle } from "./fleet-health-dom";

vi.mock("@tanstack/react-router", async () => {
  const { createElement: element } = await import("react");
  return {
    Link: ({ children, to, ...props }: { children?: unknown; to: string } & Record<string, unknown>) =>
      element("a", { ...props, href: to }, children as never),
    // #241: the frame reads the location's hash; this page has none (scaffolding only).
    useRouterState: <T>({ select }: { select: (state: { location: { hash: string } }) => T }) => select({ location: { hash: "" } }),
  };
});

mountEachTest();

/** Every section the frame composes, by slot (owner in brackets). */
const SLOTS = [
  "areas-of-concern", // #195
  "category-legend", // #203
  "fleet-score", // #195
  "live-strip", // #203
  "machine-detail", // #202
  "machine-table", // #202
];

const slots = () => all("[data-health-section]").map((slot) => slot.getAttribute("data-health-section"));

const errorBody = (code: string, message: string) => ({ error: { code, message, requestId: "req-1" } });

/** Answers each health GET with `respond(window)`, a fresh `Response` per call. */
function serve(respond: (window: string | null) => Response) {
  const fetcher = vi.fn((path: string) => {
    const window = new URL(path, "http://gridops.test").searchParams.get("window");
    return Promise.resolve(respond(window));
  });
  vi.stubGlobal("fetch", fetcher);
  return fetcher;
}

const requestedWindows = (fetcher: ReturnType<typeof serve>) =>
  fetcher.mock.calls.map(([path]) => new URL(path, "http://gridops.test").searchParams.get("window"));

describe("/fleet/health", () => {
  it("renders the fixture into one slot per section, each with content", async () => {
    const fetcher = serve((window) => Response.json({ ...fixture(), window }));
    await render(createElement(FleetHealthPage));
    await settle();

    expect(requestedWindows(fetcher)[0]).toBe("24h");
    expect([...slots()].sort()).toEqual(SLOTS);
    for (const slot of all("[data-health-section]")) {
      expect(slot.childElementCount, `slot ${slot.getAttribute("data-health-section")} is empty`).toBeGreaterThan(0);
    }
  });

  it("asks for the window chosen in the picker", async () => {
    const fetcher = serve((window) => Response.json({ ...fixture(), window }));
    await render(createElement(FleetHealthPage));
    await settle();

    await click(button("7d"));
    await settle();

    expect(requestedWindows(fetcher)).toContain("7d");
    expect(button("7d").getAttribute("aria-pressed")).toBe("true");
    expect(button("24h").getAttribute("aria-pressed")).toBe("false");
    expect([...slots()].sort()).toEqual(SLOTS);
  });

  it("tells a member the page is for administrators, with no sections", async () => {
    serve(() => Response.json(errorBody("forbidden", "Forbidden."), { status: 403 }));
    await render(createElement(FleetHealthPage));
    await settle();

    expect(slots()).toEqual([]);
    expect(document.body.textContent).toContain("Administrators only");
  });

  it("shows a failure with Retry, and renders the sections once a retry succeeds", async () => {
    let failing = true;
    const fetcher = serve((window) =>
      failing
        ? Response.json(errorBody("internal", "Something broke."), { status: 500 })
        : Response.json({ ...fixture(), window }),
    );
    await render(createElement(FleetHealthPage));
    await settle();

    expect(slots()).toEqual([]);
    expect(document.body.textContent).not.toContain("Administrators only");
    const calls = fetcher.mock.calls.length;

    failing = false;
    await click(button("Retry"));
    await settle();

    expect(fetcher.mock.calls.length).toBeGreaterThan(calls);
    expect([...slots()].sort()).toEqual(SLOTS);
  });
});
