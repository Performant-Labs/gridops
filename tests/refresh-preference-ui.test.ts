// @vitest-environment jsdom
/** The Refresh row in Settings -> Preferences and the notice live pages show when refresh is Off. */
import { act, createElement, type ReactElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { LiveRefreshNotice, formatUpdatedTime } from "~/components/live-refresh-notice";
import { RefreshPreferenceRow } from "~/features/settings/refresh-preference-row";
import { REFRESH_STORAGE_KEY } from "~/lib/refresh-interval";
import { getRefreshPreference, resetRefreshPreferenceStore } from "~/lib/use-refresh-interval";

import { installMemoryStorage, type MemoryStorage } from "./fixtures/memory-storage";

declare global {
  var IS_REACT_ACT_ENVIRONMENT: boolean | undefined;
}

let container: HTMLDivElement;
let storage: MemoryStorage;
let root: Root;

beforeEach(() => {
  globalThis.IS_REACT_ACT_ENVIRONMENT = true;
  storage = installMemoryStorage();
  resetRefreshPreferenceStore();
  container = document.createElement("div");
  document.body.append(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  document.body.innerHTML = "";
  vi.restoreAllMocks();
});

function render(element: ReactElement) {
  act(() => root.render(element));
}

const radios = () => [...container.querySelectorAll<HTMLInputElement>('input[type="radio"]')];
const radio = (label: string) => {
  const found = radios().find((input) => input.labels?.[0]?.textContent?.trim() === label);
  if (!found) throw new Error(`No "${label}" choice`);
  return found;
};
const button = (name: string) => {
  const found = [...container.querySelectorAll("button")].find((element) => element.textContent?.trim() === name);
  if (!found) throw new Error(`No "${name}" button`);
  return found;
};

describe("Refresh preference row", () => {
  it("offers the five choices as a labelled radio group with 10 seconds selected by default", () => {
    render(createElement(RefreshPreferenceRow));
    const group = container.querySelector('[role="radiogroup"]');
    expect(group?.getAttribute("aria-label")).toBe("Refresh interval");
    expect(container.textContent).toContain("Refresh");
    expect(radios().map((input) => input.labels?.[0]?.textContent?.trim())).toEqual(["5 seconds", "10 seconds", "30 seconds", "1 minute", "Off"]);
    expect(radios().every((input) => input.name === radios()[0]?.name)).toBe(true);
    expect(radio("10 seconds").checked).toBe(true);
  });

  it("selects and remembers a choice in this browser", () => {
    render(createElement(RefreshPreferenceRow));
    act(() => radio("Off").click());
    expect(radio("Off").checked).toBe(true);
    expect(getRefreshPreference()).toBe("off");
    expect(storage.getItem(REFRESH_STORAGE_KEY)).toBe("off");

    act(() => radio("1 minute").click());
    expect(storage.getItem(REFRESH_STORAGE_KEY)).toBe("1m");
  });

  it("shows a stored choice and explains Off", () => {
    storage.setItem(REFRESH_STORAGE_KEY, "30s");
    resetRefreshPreferenceStore();
    render(createElement(RefreshPreferenceRow));
    expect(radio("30 seconds").checked).toBe(true);
    expect(container.textContent).toMatch(/Off.*Refresh button/);
  });

  it("follows a change made in another tab", () => {
    render(createElement(RefreshPreferenceRow));
    act(() => { window.dispatchEvent(new StorageEvent("storage", { key: REFRESH_STORAGE_KEY, newValue: "1m" })); });
    expect(radio("1 minute").checked).toBe(true);
    act(() => { window.dispatchEvent(new StorageEvent("storage", { key: REFRESH_STORAGE_KEY, newValue: "nonsense" })); });
    expect(radio("10 seconds").checked).toBe(true);
  });

  it("still switches when browser storage is unavailable", () => {
    vi.spyOn(storage, "getItem").mockImplementation(() => { throw new Error("blocked"); });
    vi.spyOn(storage, "setItem").mockImplementation(() => { throw new Error("blocked"); });
    resetRefreshPreferenceStore();
    render(createElement(RefreshPreferenceRow));
    expect(radio("10 seconds").checked).toBe(true);
    act(() => radio("5 seconds").click());
    expect(radio("5 seconds").checked).toBe(true);
    expect(getRefreshPreference()).toBe("5s");
  });

  it("works when the browser refuses access to storage altogether", () => {
    Object.defineProperty(window, "localStorage", { configurable: true, get: () => { throw new DOMException("denied", "SecurityError"); } });
    resetRefreshPreferenceStore();
    render(createElement(RefreshPreferenceRow));
    expect(radio("10 seconds").checked).toBe(true);
    act(() => radio("Off").click());
    expect(getRefreshPreference()).toBe("off");
  });
});

describe("LiveRefreshNotice", () => {
  const updatedAt = new Date("2026-11-02T17:04:05.000Z").getTime();

  it("shows nothing while automatic refresh is on", () => {
    render(createElement(LiveRefreshNotice, { off: false, lastUpdatedAt: updatedAt, refreshing: false, onRefresh: () => undefined }));
    expect(container.textContent).toBe("");
  });

  it("says refresh is off, shows the time of the last update and offers a Refresh button", () => {
    const onRefresh = vi.fn();
    render(createElement(LiveRefreshNotice, { off: true, lastUpdatedAt: updatedAt, refreshing: false, onRefresh }));
    expect(container.textContent).toContain("Automatic refresh is off");
    expect(container.textContent).toContain(`Updated ${formatUpdatedTime(updatedAt)}`);
    expect(container.querySelector("time")?.getAttribute("datetime")).toBe("2026-11-02T17:04:05.000Z");
    const refresh = button("Refresh");
    expect(refresh.getAttribute("type")).toBe("button");
    act(() => refresh.click());
    expect(onRefresh).toHaveBeenCalledTimes(1);
  });

  it("can explain what is not updating and marks a refresh in progress", () => {
    render(createElement(LiveRefreshNotice, { off: true, lastUpdatedAt: updatedAt, refreshing: true, onRefresh: () => undefined, message: "This run is still in progress but won't update on its own." }));
    expect(container.textContent).toContain("This run is still in progress but won't update on its own.");
    expect(button("Refresh").disabled).toBe(true);
  });

  it("formats the update time with seconds", () => {
    expect(formatUpdatedTime(updatedAt)).toMatch(/\d{1,2}:\d{2}:\d{2}/);
  });
});
