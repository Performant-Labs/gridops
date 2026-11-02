// @vitest-environment jsdom
/** The live-page refresh hook follows the per-browser Refresh preference, including Off. */
import { act, createElement, useEffect } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const router = vi.hoisted(() => ({
  invalidate: vi.fn(async () => undefined),
  refetchQueries: vi.fn(async () => undefined),
  updatedAt: 0,
}));

vi.mock("@tanstack/react-router", () => ({
  useRouter: () => ({
    invalidate: async () => {
      await router.invalidate();
      router.updatedAt = Date.now();
    },
    options: { context: { queryClient: { refetchQueries: router.refetchQueries } } },
  }),
  useMatch: ({ select }: { select: (match: { updatedAt: number }) => unknown }) => select({ updatedAt: router.updatedAt }),
}));

import { type LiveRouteRefresh, useLiveRouteRefresh } from "~/lib/use-live-route-refresh";
import { resetRefreshPreferenceStore, setRefreshPreference, useRefreshSignal } from "~/lib/use-refresh-interval";

import { installMemoryStorage } from "./fixtures/memory-storage";

declare global {
  var IS_REACT_ACT_ENVIRONMENT: boolean | undefined;
}

let container: HTMLDivElement;
let root: Root;
/** What the hook returned on the latest render, recorded after each commit. */
const seen: { live?: LiveRouteRefresh; signal: number } = { signal: 0 };
let visibility: DocumentVisibilityState = "visible";

function record(live: LiveRouteRefresh, signal: number) {
  seen.live = live;
  seen.signal = signal;
}

function Harness({ base, enabled, onCommit }: { base: number; enabled?: boolean; onCommit: typeof record }) {
  const live = useLiveRouteRefresh(base, enabled);
  const signal = useRefreshSignal();
  useEffect(() => onCommit(live, signal));
  return null;
}

function render(base: number, enabled?: boolean) {
  act(() => root.render(createElement(Harness, { base, enabled, onCommit: record })));
}

function setVisibility(state: DocumentVisibilityState) {
  visibility = state;
  act(() => { document.dispatchEvent(new Event("visibilitychange")); });
}

async function advance(milliseconds: number) {
  await act(async () => { await vi.advanceTimersByTimeAsync(milliseconds); });
}

beforeEach(() => {
  vi.useFakeTimers({ now: new Date("2026-11-02T17:00:00.000Z") });
  globalThis.IS_REACT_ACT_ENVIRONMENT = true;
  installMemoryStorage();
  resetRefreshPreferenceStore();
  router.invalidate.mockClear();
  router.refetchQueries.mockClear();
  router.updatedAt = Date.now();
  visibility = "visible";
  Object.defineProperty(document, "visibilityState", { configurable: true, get: () => visibility });
  container = document.createElement("div");
  document.body.append(container);
  root = createRoot(container);
  seen.live = undefined;
  seen.signal = 0;
});

afterEach(() => {
  act(() => root.unmount());
  document.body.innerHTML = "";
  vi.useRealTimers();
});

describe("useLiveRouteRefresh", () => {
  it("keeps each page's own interval at the default setting", async () => {
    render(5_000);
    await advance(4_999);
    expect(router.invalidate).not.toHaveBeenCalled();
    await advance(1);
    expect(router.invalidate).toHaveBeenCalledTimes(1);
    await advance(5_000);
    expect(router.invalidate).toHaveBeenCalledTimes(2);
    expect(seen.live?.off).toBe(false);
  });

  it("scales the page's interval by the chosen setting", async () => {
    setRefreshPreference("30s");
    render(5_000);
    await advance(14_999);
    expect(router.invalidate).not.toHaveBeenCalled();
    await advance(1);
    expect(router.invalidate).toHaveBeenCalledTimes(1);
  });

  it("follows a change of setting while the page is open", async () => {
    render(10_000);
    act(() => setRefreshPreference("1m"));
    await advance(59_999);
    expect(router.invalidate).not.toHaveBeenCalled();
    await advance(1);
    expect(router.invalidate).toHaveBeenCalledTimes(1);
  });

  it("never refreshes on its own when the setting is Off", async () => {
    setRefreshPreference("off");
    render(3_000);
    await advance(10 * 60_000);
    setVisibility("hidden");
    setVisibility("visible");
    await advance(1_000);
    expect(router.invalidate).not.toHaveBeenCalled();
    expect(seen.live?.off).toBe(true);
  });

  it("stops as soon as the setting is switched to Off", async () => {
    render(5_000);
    await advance(5_000);
    expect(router.invalidate).toHaveBeenCalledTimes(1);
    act(() => setRefreshPreference("off"));
    await advance(60_000);
    expect(router.invalidate).toHaveBeenCalledTimes(1);
  });

  it("skips refreshes while the tab is hidden and refreshes once when it becomes visible", async () => {
    render(5_000);
    setVisibility("hidden");
    await advance(20_000);
    expect(router.invalidate).not.toHaveBeenCalled();
    setVisibility("visible");
    await advance(0);
    expect(router.invalidate).toHaveBeenCalledTimes(1);
  });

  it("does nothing while disabled", async () => {
    render(5_000, false);
    await advance(60_000);
    setVisibility("visible");
    expect(router.invalidate).not.toHaveBeenCalled();
    expect(seen.live?.enabled).toBe(false);
  });

  it("refreshes on demand, reloading queries and sections, and reports the time of the update", async () => {
    setRefreshPreference("off");
    render(10_000);
    const loadedAt = Date.now();
    expect(seen.live?.lastUpdatedAt).toBe(loadedAt);
    const signalBefore = seen.signal;

    await advance(90_000);
    await act(async () => { await seen.live?.refresh(); });

    expect(router.invalidate).toHaveBeenCalledTimes(1);
    expect(router.refetchQueries).toHaveBeenCalledWith({ type: "active" });
    expect(seen.signal).toBe(signalBefore + 1);
    expect(seen.live?.lastUpdatedAt).toBe(loadedAt + 90_000);
    expect(seen.live?.refreshing).toBe(false);
  });
});
