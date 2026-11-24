/**
 * #241: mounts the fleet health page under a real TanStack router, for the tests that need a real
 * same-route navigation (a concern `Link` to `/fleet/health#machine-<host>`). Not a test file
 * itself (`vitest.config.ts` collects `*.test.ts` only).
 *
 * The router runs on a browser history, so jsdom's own `window.location` follows each
 * navigation, as it does in a browser. A test that re-rendered the page, or navigated with
 * `router.navigate`, would hide the #241 bug (a same-route navigation re-renders nothing that
 * reads the hash), so the helpers here do neither: a test clicks the real `Link`.
 *
 * A file using this helper must not `vi.mock("@tanstack/react-router")`: a factory mock without
 * `createRouter` and `RouterProvider` throws, and a mocked `Link` never reaches the router.
 */
import { RouterProvider, createBrowserHistory, createRootRoute, createRouter } from "@tanstack/react-router";
import { act, createElement } from "react";
import { afterEach, vi } from "vitest";

import { FleetHealthPage } from "~/features/fleet-health/page";
import type { FleetHealth } from "~/features/fleet-health/types";

import { render, settle } from "./fleet-health-dom";

let destroyHistory: (() => void) | null = null;

/** Puts the location back at `/` and drops the history's listeners after each test. */
export function resetLocationEachTest() {
  afterEach(() => {
    destroyHistory?.();
    destroyHistory = null;
    window.history.replaceState(null, "", "/");
  });
}

/** Opens `/fleet/health` at `path` (its hash included) and waits for the health GET to answer `data`. */
export async function renderHealthPageAt(path: string, data: FleetHealth) {
  vi.stubGlobal("fetch", vi.fn(() => Promise.resolve(Response.json(data))));
  window.history.replaceState(null, "", path);
  const history = createBrowserHistory();
  destroyHistory = () => history.destroy();
  const router = createRouter({
    history,
    routeTree: createRootRoute({ notFoundComponent: () => null, component: FleetHealthPage }),
  });
  await act(async () => {
    await router.load();
  });
  await render(createElement(RouterProvider, { router: router as never }));
  await settle();
  return router;
}
