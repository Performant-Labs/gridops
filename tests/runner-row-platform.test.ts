// @vitest-environment jsdom
/** The runner row's CI-platform badge comes from the platform registry (#145), fallback included. */
import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@tanstack/react-router", () => ({
  Link: ({ children, to }: { children?: unknown; to?: string }) => createElement("a", { href: String(to) }, children as never),
  useRouter: () => ({ invalidate: vi.fn() }),
}));

import { TooltipProvider } from "../src/components/ui/tooltip";
import type { Runner } from "../src/features/operations/operations.functions";
import { RunnerRow } from "../src/features/runners/runner-row";

declare global {
  var IS_REACT_ACT_ENVIRONMENT: boolean | undefined;
}

const runner = (platform: string): Runner => ({
  id: "r-1", name: "mac-runner-1", provider: "tart", platform: platform as Runner["platform"], status: "idle", busy: false, ephemeral: true, os: "macos",
  architecture: "ARM64", containerId: "c-1", githubRunnerId: null, bitbucketRunnerUuid: null, failureReason: null, registeredAt: null,
  lastHeartbeatAt: null, createdAt: "2026-11-20T20:00:00.000Z", poolId: "pool-1", poolName: "mobile-macos", poolPaused: false,
  accountLogin: "acme", repository: "acme/web", currentJobName: null, currentRunId: null, machine: null, canManage: false,
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
  document.body.innerHTML = "";
});

describe("runner row platform badge", () => {
  it.each([
    ["github", "GitHub"],
    ["bitbucket", "Bitbucket"],
    ["gitlab", "gitlab"],
  ])("shows the registry name for %s", (platform, name) => {
    act(() => root.render(createElement(TooltipProvider, null, createElement(RunnerRow, { runner: runner(platform) }))));
    expect(container.textContent).toContain(name);
  });
});

describe("runner details link to the runner's page on its platform", () => {
  it("names the link with the descriptor's runnerLink label", async () => {
    act(() => root.render(createElement(TooltipProvider, null, createElement(RunnerRow, { runner: { ...runner("github"), githubRunnerId: 42 } }))));
    const open = document.querySelector<HTMLButtonElement>('button[aria-label="Show details for mac-runner-1"]');
    if (!open) throw new Error("No details button");
    await act(async () => { open.click(); });
    const link = document.querySelector<HTMLAnchorElement>('a[href="https://github.com/acme/web/settings/actions/runners/42"]');
    expect(link?.textContent).toContain("GitHub");
    expect([link?.getAttribute("title"), link?.getAttribute("aria-label")]).toContain("Open on GitHub");
  });
});
