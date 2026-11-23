// @vitest-environment jsdom
/**
 * #197 (S finding 1): the sidebar marks exactly one item as the current page.
 *
 * `isActivePath` (`~/lib/navigation`, tested in `navigation.test.ts`) decides which item is
 * highlighted, but TanStack Router's own `Link` also adds `aria-current="page"` and
 * `data-status="active"` to any link whose path is a prefix of the location, after the app's
 * props. On `/fleet/health` that made Machines (`/fleet`) and Health both "current" to a screen
 * reader, while only Health was highlighted. The pure function can't see that, so this test
 * renders the real shell under a router and reads the attributes a screen reader reads.
 */
import {
  RouterProvider, createMemoryHistory, createRootRoute, createRouter,
} from "@tanstack/react-router";
import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it } from "vitest";

import { AppShell } from "~/components/app-shell";
import { ThemeProvider } from "~/components/theme-provider";
import { TooltipProvider } from "~/components/ui/tooltip";

declare global {
  var IS_REACT_ACT_ENVIRONMENT: boolean | undefined;
}

let container: HTMLDivElement;
let root: Root;

beforeEach(() => {
  globalThis.IS_REACT_ACT_ENVIRONMENT = true;
  // jsdom has no matchMedia; the theme provider and the shell's drawer both ask for it.
  if (typeof window.matchMedia !== "function") {
    Object.defineProperty(window, "matchMedia", {
      configurable: true,
      value: (query: string) => ({
        matches: false,
        media: query,
        addEventListener: () => undefined,
        removeEventListener: () => undefined,
      }),
    });
  }
  container = document.createElement("div");
  document.body.append(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
});

/** Renders the real shell at `path`, as the app does for every signed-in page. */
async function renderShellAt(path: string) {
  const rootRoute = createRootRoute({
    // `AppShell` reads the viewer from the root route's loader data.
    loader: () => null,
    notFoundComponent: () => null,
    component: () => createElement(
      ThemeProvider,
      null,
      createElement(TooltipProvider, null, createElement(AppShell, null, null)),
    ),
  });
  const router = createRouter({
    history: createMemoryHistory({ initialEntries: [path] }),
    routeTree: rootRoute,
  });
  await act(async () => {
    await router.load();
    root.render(createElement(RouterProvider, { router: router as never }));
  });
}

/** The `href` of every sidebar link carrying `attribute="value"`. */
function marked(attribute: string, value: string) {
  return [...container.querySelectorAll<HTMLAnchorElement>(`aside a[${attribute}="${value}"]`)]
    .map((link) => link.getAttribute("href"))
    .filter((href, index, all) => all.indexOf(href) === index);
}

describe("the sidebar's current page", () => {
  it.each([
    ["/fleet/health", "/fleet/health"],
    ["/fleet", "/fleet"],
    ["/fleet/machines/abc", "/fleet"],
    ["/queued-jobs", "/queued-jobs"],
    ["/", "/"],
  ])("on %s only %s is the current page", async (path, current) => {
    await renderShellAt(path);
    // Both Health and Machines links exist, so a prefix match would have something to mark twice.
    const hrefs = [...container.querySelectorAll("aside a")].map((link) => link.getAttribute("href"));
    expect(hrefs).toEqual(expect.arrayContaining(["/fleet", "/fleet/health"]));

    expect(marked("aria-current", "page"), "what a screen reader announces").toEqual([current]);
    expect(marked("data-status", "active"), "the router's own marker").toEqual([current]);
  });
});
