// @vitest-environment jsdom
/**
 * Settings → Connections replaces the GitHub and Bitbucket pages (#145): the three old routes are
 * redirect-only and replace-redirect to it, the `?appError=` redirect points straight at it, and
 * every in-app link that pointed at an old route points at the new one (no redirect chains).
 */
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { act, createElement, type ComponentType, type ReactElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const loaderData = vi.hoisted(() => ({ current: undefined as unknown }));
const toast = vi.hoisted(() => ({ success: vi.fn(), error: vi.fn(), promise: vi.fn() }));

// The real `redirect` is kept, so the thrown value is the router's own redirect.
vi.mock("@tanstack/react-router", async (importOriginal) => ({
  redirect: (await importOriginal<typeof import("@tanstack/react-router")>()).redirect,
  createFileRoute: () => (options: Record<string, unknown>) => ({ options, useLoaderData: () => loaderData.current, fullPath: "/test" }),
  getRouteApi: () => ({ useLoaderData: () => loaderData.current }),
  Link: ({ children, to }: { children?: unknown; to?: string }) => createElement("a", { href: String(to) }, children as never),
  Outlet: () => null,
  useNavigate: () => vi.fn(),
  useRouter: () => ({ invalidate: vi.fn() }),
  useMatch: (options: { select: (match: { updatedAt: number }) => unknown }) => options.select({ updatedAt: Date.now() }),
  useRouterState: (options: { select: (state: { location: { pathname: string } }) => unknown }) => options.select({ location: { pathname: "/settings/connections" } }),
}));
vi.mock("sonner", () => ({ toast }));

import { ThemeProvider } from "../src/components/theme-provider";
import { TooltipProvider } from "../src/components/ui/tooltip";
import { Route as PlatformConnectionsRoute } from "../src/routes/_app/platform-connections";
import { Route as NewPoolRoute } from "../src/routes/_app/runner-pools.new";
import { Route as WebhooksRoute } from "../src/routes/_app/webhooks";
import { Route as SettingsRoute } from "../src/routes/settings";
import { Route as GitHubSettingsRoute } from "../src/routes/settings.github";
import { Route as SettingsIndexRoute } from "../src/routes/settings.index";
import { Route as IntegrationsRoute } from "../src/routes/settings.integrations";
import { createOptions, openPicker, poolFormFetch, settle } from "./fixtures/pool-forms";

declare global {
  var IS_REACT_ACT_ENVIRONMENT: boolean | undefined;
}

class NoopResizeObserver { observe() {} unobserve() {} disconnect() {} }

type RouteOptions = { beforeLoad?: (context: unknown) => unknown; component?: unknown; loader?: unknown; pendingComponent?: ComponentType };
const optionsOf = (route: unknown) => (route as { options: RouteOptions }).options;

/** Runs a route's beforeLoad and returns the redirect it throws. */
function redirectFrom(route: unknown, context: unknown = {}) {
  const beforeLoad = optionsOf(route).beforeLoad;
  expect(typeof beforeLoad).toBe("function");
  try {
    beforeLoad?.(context);
  } catch (thrown) {
    return (thrown as { options: { to?: string; replace?: boolean; search?: unknown } }).options;
  }
  throw new Error("beforeLoad did not redirect");
}

describe("old routes are redirect-only and point at Settings → Connections", () => {
  it.each([
    ["/settings/integrations", IntegrationsRoute],
    ["/settings/github", GitHubSettingsRoute],
    ["/platform-connections", PlatformConnectionsRoute],
  ])("%s replace-redirects to /settings/connections, with no page or loader of its own", (_path, route) => {
    expect(redirectFrom(route)).toMatchObject({ to: "/settings/connections", replace: true });
    expect(optionsOf(route).component).toBeUndefined();
    expect(optionsOf(route).loader).toBeUndefined();
  });
});

describe("/settings?appError=", () => {
  beforeEach(() => toast.error.mockReset());

  it("toasts the error once and replace-redirects to /settings/connections with no search", () => {
    const target = redirectFrom(SettingsIndexRoute, { location: { search: { appError: "GitHub App setup was cancelled." } }, preload: false });
    expect(toast.error).toHaveBeenCalledTimes(1);
    expect(toast.error).toHaveBeenCalledWith("GitHub App setup was cancelled.");
    expect(target).toMatchObject({ to: "/settings/connections", replace: true });
    expect(target.search).toBeUndefined();
  });

  it("redirects the same way without toasting on preload", () => {
    const target = redirectFrom(SettingsIndexRoute, { location: { search: { appError: "GitHub App setup was cancelled." } }, preload: true });
    expect(toast.error).not.toHaveBeenCalled();
    expect(target).toMatchObject({ to: "/settings/connections", replace: true });
    expect(target.search).toBeUndefined();
  });

  it("still sends /settings without appError to /settings/general", () => {
    expect(redirectFrom(SettingsIndexRoute, { location: { search: {} }, preload: false })).toMatchObject({ to: "/settings/general", replace: true });
    expect(toast.error).not.toHaveBeenCalled();
  });
});

describe("in-app links point straight at /settings/connections", () => {
  let container: HTMLDivElement;
  let root: Root;

  beforeEach(() => {
    globalThis.IS_REACT_ACT_ENVIRONMENT = true;
    vi.stubGlobal("ResizeObserver", NoopResizeObserver);
    vi.stubGlobal("matchMedia", () => ({ matches: false, addEventListener() {}, removeEventListener() {} }));
    vi.stubGlobal("fetch", vi.fn(async (url: string, init?: RequestInit) => poolFormFetch(url, init)));
    container = document.createElement("div");
    document.body.append(container);
    root = createRoot(container);
  });

  afterEach(() => {
    act(() => root.unmount());
    document.body.innerHTML = "";
    vi.unstubAllGlobals();
  });

  async function render(element: ReactElement) {
    const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
    await act(async () => root.render(createElement(QueryClientProvider, { client }, createElement(TooltipProvider, null, element))));
    await settle();
  }

  const hrefs = (selector = "a") => [...document.querySelectorAll(selector)].map((element) => element.getAttribute("href"));
  const OLD = ["/settings/github", "/settings/integrations", "/platform-connections"];

  it("the settings navigation has one Connections item", async () => {
    loaderData.current = { authenticated: true, viewer: null };
    const pending = optionsOf(SettingsRoute).pendingComponent;
    if (!pending) throw new Error("No settings shell");
    await render(createElement(ThemeProvider, null, createElement(pending)));
    const nav = [...document.querySelectorAll<HTMLAnchorElement>('nav[aria-label="Settings"] a')]
      .map((link) => [link.getAttribute("href"), link.textContent?.trim()]);
    expect(nav).toContainEqual(["/settings/connections", "Connections"]);
    expect(nav.filter(([href]) => OLD.includes(href ?? ""))).toEqual([]);
  });

  it("the webhooks empty state links to GitHub settings on the Connections page", async () => {
    loaderData.current = { authenticated: true, items: [], total: 0, page: 1, perPage: 25, failedTotal: 0 };
    const page = optionsOf(WebhooksRoute).component as ComponentType;
    await render(createElement(page));
    const links = [...document.querySelectorAll("a")].map((link) => [link.getAttribute("href"), link.textContent?.trim()]);
    expect(links).toContainEqual(["/settings/connections", "GitHub settings"]);
    expect(hrefs().filter((href) => OLD.includes(href ?? ""))).toEqual([]);
  });

  it("the new-pool form's connect-a-workspace link goes to the Connections page", async () => {
    loaderData.current = createOptions({ bitbucketConnections: [] });
    const page = optionsOf(NewPoolRoute).component as ComponentType;
    await render(createElement(page));
    openPicker("Targets");
    expect(hrefs()).toContain("/settings/connections");
    expect(hrefs().filter((href) => OLD.includes(href ?? ""))).toEqual([]);
  });
});
