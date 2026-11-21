// @vitest-environment jsdom
/**
 * #169: the Overview's "Runner pools" rows take each pool's scope text from the platform that owns
 * the scope (the registry's scope-label hook), not the raw scope. GitHub's scopes read as today
 * ("repository" / "organization", which the row's CSS `capitalize` shows as "Repository" /
 * "Organization"); a GitLab pool reads "GitLab", not "Gitlab"; an unknown scope reads "Unknown"
 * and the raw scope is never shown. The "· mode" part is untouched.
 *
 * Driven through the rendered route with the production registry. Copy from
 * docs/handoffs/169/wireframe.html, frame C and the contract table E.
 */
import { act, createElement, type ComponentType } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const loaderData = vi.hoisted(() => ({ current: undefined as unknown }));

vi.mock("@tanstack/react-router", () => ({
  createFileRoute: () => (options: Record<string, unknown>) => ({ options, useLoaderData: () => loaderData.current }),
  Link: ({ children, to }: { children?: unknown; to?: string }) => createElement("a", { href: String(to) }, children as never),
  useMatch: () => undefined,
  useNavigate: () => vi.fn(),
  useRouter: () => ({ invalidate: vi.fn(async () => undefined) }),
}));
vi.mock("sonner", () => ({ toast: { success: vi.fn(), error: vi.fn(), promise: vi.fn() } }));

import { TooltipProvider } from "../src/components/ui/tooltip";
import type { DashboardOverview } from "../src/features/dashboard/types";
import { Route as OverviewRoute } from "../src/routes/_app/index";

declare global {
  var IS_REACT_ACT_ENVIRONMENT: boolean | undefined;
}

type OverviewPool = DashboardOverview["pools"][number];

const pool = (id: string, name: string, scope: string, mode = "ephemeral"): OverviewPool => ({
  id, name, scope, mode, desired: 4, online: 2, busy: 1, queue: 0, status: "healthy",
});

const POOLS = [
  pool("p-repo", "web-ci", "repository"),
  pool("p-org", "org-pool", "organization", "persistent"),
  pool("p-gl", "gl-pool", "gitlab"),
  pool("p-odd", "odd-pool", "azure-devops"),
];

/** A signed-out Overview with complete configuration and nothing else to show but the pools. */
function overview(pools: OverviewPool[]): DashboardOverview & { agentRuns: null } {
  return {
    authenticated: false,
    configuration: {
      githubOAuth: true, githubAppControl: true, webhookActive: false, webhookVerification: true,
      secureStorage: true, runnerManager: true, callbackUrl: "", webhookUrl: "",
    },
    metrics: { runners: 0, online: 0, busy: 0, idle: 0, starting: 0, unavailable: 0, queuedJobs: 0, queuedWaiting: 0, queuedUnmatched: 0, successRate: null },
    queue: { unmatched: 0, otherPools: 0, missingLabels: [] },
    slo: {
      windowHours: 24, queue: { oldestSeconds: null, p95Seconds: null },
      startLatency: { sampleSize: 0, p50Seconds: null, p95Seconds: null }, failures: [], alerts: [],
    },
    pools,
    runs: [],
    machines: null,
    activity: [],
    installations: 0,
    agentRuns: null,
  };
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
  document.body.innerHTML = "";
});

function renderOverview(pools: OverviewPool[]) {
  loaderData.current = overview(pools);
  const page = (OverviewRoute as unknown as { options: { component: ComponentType } }).options.component;
  act(() => root.render(createElement(TooltipProvider, null, createElement(page))));
}

/** The pool's row: the link to its detail page that names it. */
function poolRow(name: string) {
  const row = [...container.querySelectorAll<HTMLAnchorElement>('a[href="/runner-pools/$poolId"]')].find((link) => link.textContent?.includes(name));
  if (!row) throw new Error(`No Overview row for pool ${name}`);
  return row;
}

/** The row's "scope · mode" text, and whether it keeps the row's `capitalize` styling. */
function scopeText(name: string) {
  const span = [...poolRow(name).querySelectorAll("span")].find((candidate) => / · /.test(candidate.textContent ?? "") && !candidate.querySelector("span"));
  if (!span) throw new Error(`No scope · mode text in the row for pool ${name}`);
  return { text: span.textContent, capitalized: span.classList.contains("capitalize") };
}

describe("Overview pool rows show the scope label of the pool's platform (#169)", () => {
  it.each([
    ["web-ci", "repository · ephemeral", "a GitHub repository pool reads as today"],
    ["org-pool", "organization · persistent", "a GitHub organization pool reads as today"],
    ["gl-pool", "GitLab · ephemeral", "a GitLab pool reads GitLab, not Gitlab"],
    ["odd-pool", "Unknown · ephemeral", "an unknown scope reads Unknown, never its raw scope"],
  ])("%s shows %j: %s", (name, text) => {
    renderOverview(POOLS);
    // The row's CSS `capitalize` stays, so GitHub's raw scope still displays as "Repository" / "Organization".
    expect(scopeText(name)).toEqual({ text, capitalized: true });
  });
});
