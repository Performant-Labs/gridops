// @vitest-environment jsdom
/** Renders the Overview's "Needs attention" list: each alert's words, its link, and when a machine set aside is reconsidered. */
import { createElement, act, type ReactElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { AttentionAlerts, alertDetail } from "../src/features/dashboard/attention-alerts";
import type { OverviewAlert } from "../src/features/dashboard/types";
import { formatDateTime } from "../src/lib/utils";

declare global {
  var IS_REACT_ACT_ENVIRONMENT: boolean | undefined;
}

// The list only needs links to render as anchors, not a router.
vi.mock("@tanstack/react-router", () => ({
  Link: ({ children, params, to, ...props }: { children?: unknown; params?: Record<string, string>; to: string } & Record<string, unknown>) =>
    createElement("a", { ...props, href: Object.entries(params ?? {}).reduce((path, [key, value]) => path.replace(`$${key}`, value), to) }, children as never),
}));

const gated: OverviewAlert = {
  level: "warning",
  title: "Fleet machine flaky-box is not accepting new runners",
  detail: "Not accepting new runners: 3 start failures in the last 15 minutes.",
  until: "2026-11-02T18:45:00.000Z",
  href: "/fleet/machines/0b9a6a0e-4f5d-4c41-9a43-3f1c6f0b2d11",
};

const waiting: OverviewAlert = {
  level: "error",
  title: "1 provisioning circuit is open",
  detail: "GridOps is pausing repeated failed runner starts until the pool is retried.",
  href: "/runner-pools",
};

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

const links = () => [...container.querySelectorAll("a")];

describe("needs attention", () => {
  it("says when a machine set aside is reconsidered, in the reader's time", () => {
    expect(alertDetail(gated, (iso) => `<${iso}>`)).toBe(
      "Not accepting new runners: 3 start failures in the last 15 minutes. Reconsidered at <2026-11-02T18:45:00.000Z>, or sooner if a start on it succeeds.",
    );
    // Alerts without a time read as the server wrote them.
    expect(alertDetail(waiting, () => "never")).toBe(waiting.detail);
  });

  it("links a machine's alert to that machine's page and the rest to their own pages", () => {
    render(createElement(AttentionAlerts, { alerts: [waiting, gated] }));
    const [circuit, machine] = links();
    expect(links()).toHaveLength(2);
    expect(circuit?.getAttribute("href")).toBe("/runner-pools");
    expect(circuit?.textContent).toContain(waiting.detail);
    expect(machine?.getAttribute("href")).toBe("/fleet/machines/0b9a6a0e-4f5d-4c41-9a43-3f1c6f0b2d11");
    expect(machine?.textContent).toContain("Fleet machine flaky-box is not accepting new runners");
    expect(machine?.textContent).toContain(`Reconsidered at ${formatDateTime(gated.until ?? "")}`);
  });

  it("links the disk alarms to the pages the server names", () => {
    render(createElement(AttentionAlerts, {
      alerts: [
        { level: "warning", title: "Fleet machine box is running out of disk", detail: "2.0 GiB free", href: "/fleet" },
        { level: "error", title: "GridOps server disk is running out of space", detail: "1.0 GiB free", href: "/settings/runner-host" },
      ],
    }));
    expect(links().map((link) => link.getAttribute("href"))).toEqual(["/fleet", "/settings/runner-host"]);
  });

  it("says nothing needs attention when there are no alerts", () => {
    render(createElement(AttentionAlerts, { alerts: [] }));
    expect(links()).toHaveLength(0);
    expect(container.textContent).toContain("No active runner-control incidents.");
  });
});
