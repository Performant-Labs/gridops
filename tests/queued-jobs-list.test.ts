// @vitest-environment jsdom
/** Renders the queued-jobs table in a DOM: rows, the no-pool marking, the empty, loading and error states. */
import { createElement, act, type ComponentProps, type ReactElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { QueuedJobsList, formatWait } from "../src/features/dashboard/queued-jobs";
import type { QueuedJob } from "../src/features/dashboard/types";

declare global {
  var IS_REACT_ACT_ENVIRONMENT: boolean | undefined;
}

// The list only needs links to render as anchors, not a router.
vi.mock("@tanstack/react-router", () => ({
  Link: ({ children, params, to, ...props }: { children?: unknown; params?: Record<string, string>; to: string } & Record<string, unknown>) =>
    createElement("a", { ...props, href: Object.entries(params ?? {}).reduce((path, [key, value]) => path.replace(`$${key}`, value), to) }, children as never),
}));

const job = (overrides: Partial<QueuedJob> & Pick<QueuedJob, "id">): QueuedJob => ({
  name: "build",
  repository: "octo-org/app",
  workflow: "CI",
  runId: 100,
  runUrl: "https://github.com/octo-org/app/actions/runs/100",
  htmlUrl: "https://github.com/octo-org/app/actions/runs/100/job/1",
  labels: ["self-hosted", "shared"],
  matchedPool: { id: "pool-a", name: "linux-pool" },
  waitingReason: "waiting_for_runner",
  queuedAt: "2026-11-01T20:00:00.000Z",
  ageSeconds: 125,
  ...overrides,
});

const listProps = (overrides: Partial<ComponentProps<typeof QueuedJobsList>> = {}): ComponentProps<typeof QueuedJobsList> => ({
  items: [],
  page: 1,
  perPage: 25,
  total: 0,
  ...overrides,
});

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

function render(element: ReactElement) {
  act(() => root.render(element));
}

const rows = () => [...container.querySelectorAll("tbody tr")];

describe("queued jobs list", () => {
  it("says nothing is queued instead of showing an empty table", () => {
    render(createElement(QueuedJobsList, listProps()));
    expect(container.textContent).toContain("No queued jobs");
    expect(container.querySelector("table")).toBeNull();
  });

  it("lists each job with its repository, workflow, labels, pool and wait, in the order given", () => {
    render(createElement(QueuedJobsList, listProps({
      items: [
        job({ id: 1, name: "test", workflow: "CI", ageSeconds: 7_320 }),
        job({ id: 2, name: "package", repository: "octo-org/site", workflow: "Release", labels: ["self-hosted", "gridops-ci"], matchedPool: { id: "pool-b", name: "mac-pool" }, waitingReason: "pool_at_capacity", ageSeconds: 30 }),
      ],
      total: 2,
    })));
    const [first, second] = rows();
    expect(rows()).toHaveLength(2);
    expect(first?.textContent).toContain("test");
    expect(first?.textContent).toContain("CI");
    expect(first?.textContent).toContain("octo-org/app");
    expect(first?.textContent).toContain("shared");
    expect(first?.textContent).toContain("linux-pool");
    expect(first?.textContent).toContain("2h 2m");
    expect(first?.textContent).toContain("Waiting for a runner");
    expect(first?.querySelector("a[href='/runner-pools/pool-a']")).not.toBeNull();
    expect(first?.querySelector("a[href='/workflow-runs/100']")).not.toBeNull();
    expect(second?.textContent).toContain("Release");
    expect(second?.textContent).toContain("gridops-ci");
    expect(second?.textContent).toContain("mac-pool");
    expect(second?.textContent).toContain("30s");
    expect(second?.textContent).toContain("Pool is at its runner limit");
    expect(second?.querySelector("a[href='/runner-pools/pool-b']")).not.toBeNull();
  });

  it("marks a job that no pool can run", () => {
    render(createElement(QueuedJobsList, listProps({
      items: [job({ id: 3, labels: ["self-hosted", "gridops-ci"], matchedPool: null, waitingReason: "no_matching_pool", ageSeconds: 26_520 })],
      total: 1,
    })));
    const [row] = rows();
    expect(row?.getAttribute("data-reason")).toBe("no_matching_pool");
    expect(row?.textContent).toContain("No matching pool");
    expect(row?.textContent).toContain("No pool offers these labels");
    expect(row?.textContent).toContain("gridops-ci");
    expect(row?.textContent).toContain("7h 22m");
    expect(row?.querySelector("a[href^='/runner-pools']")).toBeNull();
  });

  it("names the labels no pool provides for a job that can never run", () => {
    render(createElement(QueuedJobsList, listProps({
      items: [job({ id: 5, labels: ["self-hosted", "self-hosted-macos", "xcode"], missingLabels: ["self-hosted-macos", "xcode"], matchedPool: null, waitingReason: "no_matching_pool" })],
      total: 1,
    })));
    const [row] = rows();
    expect(row?.textContent).toContain("No matching pool");
    expect(row?.textContent).toContain("No pool provides self-hosted-macos, xcode");
    expect(row?.textContent).not.toContain("No pool offers these labels");
    const missing = [...(row?.querySelectorAll("[data-missing='true']") ?? [])].map((badge) => badge.textContent);
    expect(missing).toEqual(["self-hosted-macos", "xcode"]);
  });

  it("says no pool is available when every label exists but no pool can take the job", () => {
    render(createElement(QueuedJobsList, listProps({
      items: [job({ id: 6, labels: ["self-hosted", "paused-only"], missingLabels: [], matchedPool: null, waitingReason: "no_matching_pool" })],
      total: 1,
    })));
    const [row] = rows();
    expect(row?.textContent).toContain("No matching pool");
    expect(row?.textContent).toContain("No available pool");
    expect(row?.querySelector("[data-missing='true']")).toBeNull();
  });

  it("shows an unknown reason as plain text rather than failing", () => {
    render(createElement(QueuedJobsList, listProps({ items: [job({ id: 4, waitingReason: "unmatched_labels" })], total: 1 })));
    expect(rows()[0]?.textContent).toContain("Unmatched labels");
  });

  it("shows a loading state before the first page arrives", () => {
    render(createElement(QueuedJobsList, listProps({ loading: true })));
    expect(container.querySelector("[aria-busy='true']")).not.toBeNull();
    expect(container.textContent).not.toContain("No queued jobs");
  });

  it("shows a failed load with a retry, not an empty queue", () => {
    const onRetry = vi.fn();
    render(createElement(QueuedJobsList, listProps({ error: "GridOps request failed (500).", onRetry })));
    expect(container.querySelector("[role='alert']")?.textContent).toContain("GridOps request failed (500).");
    expect(container.textContent).not.toContain("No queued jobs");
    const retry = [...container.querySelectorAll("button")].find((button) => button.textContent?.includes("Retry"));
    act(() => retry?.click());
    expect(onRetry).toHaveBeenCalledOnce();
  });

  it("keeps the jobs on screen when a refresh fails", () => {
    render(createElement(QueuedJobsList, listProps({ error: "offline", items: [job({ id: 1 })], total: 1 })));
    expect(rows()).toHaveLength(1);
    expect(container.querySelector("[role='alert']")?.textContent).toContain("offline");
  });

  it("pages through a long queue", () => {
    const onPageChange = vi.fn();
    render(createElement(QueuedJobsList, listProps({ items: [job({ id: 1 }), job({ id: 2 })], onPageChange, perPage: 2, total: 5 })));
    expect(container.textContent).toContain("1–2 of 5 queued jobs");
    const next = container.querySelector<HTMLButtonElement>("button[aria-label='Next queued jobs page']");
    act(() => next?.click());
    expect(onPageChange).toHaveBeenCalledWith(2);
  });

  it("leaves pagination out when no page handler is given", () => {
    render(createElement(QueuedJobsList, listProps({ items: [job({ id: 1 })], perPage: 1, total: 9 })));
    expect(container.querySelector("nav")).toBeNull();
  });
});

describe("formatWait", () => {
  it("writes seconds, minutes, hours and days", () => {
    expect(formatWait(0)).toBe("0s");
    expect(formatWait(59)).toBe("59s");
    expect(formatWait(300)).toBe("5m");
    expect(formatWait(26_520)).toBe("7h 22m");
    expect(formatWait(90_000)).toBe("1d 1h");
    expect(formatWait(-4)).toBe("0s");
  });
});
