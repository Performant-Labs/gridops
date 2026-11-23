// @vitest-environment jsdom
/**
 * #197 (S advisory, the same defect class as S finding 1): TanStack Router's `Link` adds
 * `aria-current="page"` and `data-status="active"` to any link whose path is a prefix of the
 * location. A breadcrumb names an ancestor, so on `/fleet/machines/<id>` the "Machines" crumb
 * (`/fleet`) must not be announced as the current page; and a view tab the app calls inactive
 * must not be either. This renders the real `PageHeader` and `ViewTabLink` under a router and
 * reads the attributes a screen reader reads.
 */
import {
  RouterProvider, createMemoryHistory, createRootRoute, createRouter,
} from "@tanstack/react-router";
import { act, createElement, type ReactNode } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it } from "vitest";

import { PageHeader, ViewTabLink } from "~/components/page";

declare global {
  var IS_REACT_ACT_ENVIRONMENT: boolean | undefined;
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

async function renderAt(path: string, view: () => ReactNode) {
  const rootRoute = createRootRoute({ notFoundComponent: () => null, component: view });
  const router = createRouter({ history: createMemoryHistory({ initialEntries: [path] }), routeTree: rootRoute });
  await act(async () => {
    await router.load();
    root.render(createElement(RouterProvider, { router: router as never }));
  });
}

/** The `href` of every link inside `scope` carrying `attribute="value"`. */
function marked(scope: string, attribute: string, value: string) {
  return [...container.querySelectorAll<HTMLAnchorElement>(`${scope} a[${attribute}="${value}"]`)]
    .map((link) => link.getAttribute("href"));
}

describe("the page header's breadcrumbs", () => {
  it.each(["/fleet/machines/abc", "/fleet/health"])("on %s no breadcrumb is the current page", async (path) => {
    await renderAt(path, () => createElement(PageHeader, {
      title: "abc",
      breadcrumbs: [{ label: "Machines", to: "/fleet" }, { label: "abc" }],
    }));
    const hrefs = [...container.querySelectorAll('nav[aria-label="Breadcrumb"] a')].map((link) => link.getAttribute("href"));
    expect(hrefs).toEqual(["/fleet"]);

    expect(marked('nav[aria-label="Breadcrumb"]', "aria-current", "page"), "what a screen reader announces").toEqual([]);
    expect(marked('nav[aria-label="Breadcrumb"]', "data-status", "active"), "the router's own marker").toEqual([]);
  });
});

describe("a view tab", () => {
  it("only the tab the app calls active is the current page", async () => {
    await renderAt("/fleet/health", () => createElement("div", { id: "tabs" },
      createElement(ViewTabLink, { active: false, to: "/fleet", children: "Machines" }),
      createElement(ViewTabLink, { active: true, to: "/fleet/health", children: "Health" }),
    ));
    expect(marked("#tabs", "aria-current", "page")).toEqual(["/fleet/health"]);
    expect(marked("#tabs", "data-status", "active")).toEqual(["/fleet/health"]);
  });
});
