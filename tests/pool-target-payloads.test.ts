// @vitest-environment jsdom
/**
 * Payload equality for the one Targets row (#145): for the same selection, the `data` object the
 * new forms pass to createRunnerPoolAction / updateRunnerPoolAction is deep-equal to what the old
 * forms built. The expected objects are frozen constants captured from the OLD forms at commit
 * 5550cf5 (capture harness, command and output: docs/handoffs/145/handoff-T-red.md). They are
 * never computed by, or imported from, the new implementation, and are not edited to match it.
 */
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { act, createElement, type ComponentType } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const loaderData = vi.hoisted(() => ({ current: undefined as unknown }));
const recorded = vi.hoisted(() => ({ create: [] as unknown[], update: [] as unknown[] }));

vi.mock("@tanstack/react-router", () => ({
  createFileRoute: () => (options: Record<string, unknown>) => ({ options, useLoaderData: () => loaderData.current }),
  getRouteApi: () => ({ useLoaderData: () => loaderData.current }),
  Link: ({ children, to }: { children?: unknown; to?: string }) => createElement("a", { href: String(to) }, children as never),
  useNavigate: () => vi.fn(),
}));
vi.mock("sonner", () => ({ toast: { success: vi.fn(), error: vi.fn() } }));
// Wraps the real actions so the `data` argument is recorded and the request still goes through `api()`.
vi.mock("../src/features/runner-pools/runner-pools.functions", async (importOriginal) => {
  const actual = await importOriginal<typeof import("../src/features/runner-pools/runner-pools.functions")>();
  return {
    ...actual,
    createRunnerPoolAction: (args: Parameters<typeof actual.createRunnerPoolAction>[0]) => { recorded.create.push(args.data); return actual.createRunnerPoolAction(args); },
    updateRunnerPoolAction: (args: Parameters<typeof actual.updateRunnerPoolAction>[0]) => { recorded.update.push(args.data); return actual.updateRunnerPoolAction(args); },
  };
});

import { TooltipProvider } from "../src/components/ui/tooltip";
import { Route as NewPoolRoute } from "../src/routes/_app/runner-pools.new";
import { Route as PoolSettingsRoute } from "../src/routes/_app/runner-pools.$poolId.settings";

declare global {
  var IS_REACT_ACT_ENVIRONMENT: boolean | undefined;
}

class NoopResizeObserver { observe() {} unobserve() {} disconnect() {} }

// ---- Fixed inputs: identical to the capture harness ----
const INSTALLATIONS = [
  { id: 1, accountLogin: "octo-org", accountType: "Organization" },
  { id: 2, accountLogin: "acme", accountType: "Organization" },
  { id: 3, accountLogin: "jdoe", accountType: "User" },
];
const REPOSITORIES = [
  { id: 10, installationId: 1, accountLogin: "octo-org", accountType: "Organization", fullName: "octo-org/app", private: true },
  { id: 11, installationId: 1, accountLogin: "octo-org", accountType: "Organization", fullName: "octo-org/api", private: false },
  { id: 20, installationId: 2, accountLogin: "acme", accountType: "Organization", fullName: "acme/web", private: true },
];
const RUNNER_GROUPS: Record<number, Array<{ id: number; name: string; visibility: string; isDefault: boolean }>> = {
  1: [{ id: 1, name: "Default", visibility: "all", isDefault: true }, { id: 4, name: "Linux builds", visibility: "selected", isDefault: false }],
  2: [{ id: 1, name: "Default", visibility: "all", isDefault: true }, { id: 7, name: "Release", visibility: "private", isDefault: false }],
};
const BITBUCKET = [
  { id: "6a1c1b2e-0000-4000-8000-000000000001", name: "Acme Bitbucket", workspace: "acme", workspaceUuid: "{acme}" },
  { id: "6a1c1b2e-0000-4000-8000-000000000002", name: "Mobile delivery", workspace: "acme-mobile", workspaceUuid: "{acme-mobile}" },
];
const createOptions = () => ({
  authenticated: true,
  installUrl: "https://github.com/apps/gridops/installations/new",
  builtinRunnerHostEnabled: true,
  installations: INSTALLATIONS,
  repositories: REPOSITORIES.map(({ id, installationId, fullName, private: isPrivate }) => ({ id, installationId, fullName, private: isPrivate })),
  runnerGroups: Object.entries(RUNNER_GROUPS).flatMap(([installationId, groups]) => groups.map((group) => ({ installationId: Number(installationId), ...group }))),
  bitbucketConnections: BITBUCKET,
  defaults: {
    provider: "docker", providers: ["docker"], image: "runner:default", dockerImage: "runner:default", tartImage: "fixture-default-tahoe",
    macosRuntime: "vm", labels: ["gridops"], cpuLimit: 2, memoryLimitMb: 2048, desiredCount: 0, minCount: 0, maxCount: 10,
    autoscalingEnabled: true, queueScaleFactor: 1, idleTimeoutMinutes: 5, runnerGroupId: 1, maxCpuLimit: 16, maxMemoryLimitMb: 65_536,
  },
});
const settingsPool = (overrides: Record<string, unknown>) => ({
  id: "pool-1", installationId: 1, repositoryId: null, repository: null, repositoryIds: [], repositories: [],
  priorityRepositoryIds: [], priorityWaitMinutes: 10, bitbucketConnections: [], bitbucketConnectionIds: [],
  accountLogin: "octo-org", name: "linux-app", scope: "organization", mode: "ephemeral", provider: "docker",
  providers: ["docker"], labels: ["build"], image: "runner:acme", dockerImage: "runner:acme", tartImage: "",
  macosRuntime: "vm", desiredCount: 1, minCount: 0, maxCount: 4, cpuLimit: 4, memoryLimitMb: 8192,
  maxCpuLimit: 16, maxMemoryLimitMb: 65_536, builtinRunnerHostEnabled: true, runnerGroupId: 1, paused: false,
  state: "active", autoscalingEnabled: true, queueScaleFactor: 1, idleTimeoutMinutes: 5, configurationVersion: 3,
  provisionFailureCount: 0, provisionRetryAt: null, provisionCircuitOpen: false, canManage: true,
  ...overrides,
});
const REPOSITORY_POOL = settingsPool({
  name: "macos-app", scope: "repository", installationId: 1, repositoryIds: [10], repositories: [{ ...REPOSITORIES[0], prioritised: true }],
  priorityRepositoryIds: [10], priorityWaitMinutes: 15, bitbucketConnections: [BITBUCKET[0]], bitbucketConnectionIds: [BITBUCKET[0]?.id],
  provider: "tart", providers: ["tart"], labels: ["build"], image: "acme-xcode-26", dockerImage: "", tartImage: "acme-xcode-26",
});
const ORGANIZATION_POOL = settingsPool({ name: "acme-linux", scope: "organization", installationId: 2, accountLogin: "acme", runnerGroupId: 1 });

function fetchStub(url: string, init?: RequestInit) {
  const method = init?.method ?? "GET";
  if (method === "GET" && url === "/api/v1/runner-pools/repositories") return Response.json({ items: REPOSITORIES });
  const repos = /^\/api\/v1\/installations\/(\d+)\/repositories$/.exec(url);
  if (method === "GET" && repos) return Response.json({ items: REPOSITORIES.filter((repository) => repository.installationId === Number(repos[1])) });
  const groups = /^\/api\/v1\/installations\/(\d+)\/runner-groups$/.exec(url);
  if (method === "GET" && groups) {
    const items = RUNNER_GROUPS[Number(groups[1])];
    return items ? Response.json({ items }) : Response.json({ error: "Runner groups are listed for organizations only." }, { status: 404 });
  }
  if (method === "GET" && /^\/api\/v1\/platform-connections\/bitbucket$/.test(url)) return Response.json({ canManage: true, items: [] });
  if (method === "POST" && url === "/api/v1/runner-pools") return Response.json({ id: "pool-new" }, { status: 201 });
  if (method === "PUT" && url === "/api/v1/runner-pools/pool-1") return Response.json({ ok: true, configurationVersion: 4, rollingReplacement: false });
  return Response.json({ error: `not in this test: ${method} ${url}` }, { status: 404 });
}

// ---- Frozen old-form output (captured, never hand-edited) ----

// Captured at commit 5550cf5 from src/routes/_app/runner-pools.new.tsx lines 233-263 (old submit):
// repositories acme/web (installation 2) then octo-org/app (installation 1), i.e. clicked in reverse list order.
const REPOSITORY_CREATE = {
  installationId: 1,
  repositoryIds: [20, 10],
  bitbucketConnectionIds: [],
  name: "linux-app",
  scope: "repository",
  mode: "ephemeral",
  provider: "docker",
  providers: ["docker"],
  labels: ["gridops"],
  image: "runner:default",
  dockerImage: "runner:default",
  tartImage: "",
  macosRuntime: "vm",
  desiredCount: 0,
  minCount: 0,
  maxCount: 10,
  autoscalingEnabled: true,
  queueScaleFactor: 1,
  idleTimeoutMinutes: 5,
  cpuLimit: 2,
  memoryLimitMb: 2048,
  runnerGroupId: 1,
};

// Captured at commit 5550cf5 from src/routes/_app/runner-pools.new.tsx lines 233-263 (old submit):
// scope Organization, installation acme (2), runner group Release (7).
const RUNNER_GROUP_CREATE = {
  installationId: 2,
  repositoryIds: [],
  bitbucketConnectionIds: [],
  name: "acme-release",
  scope: "organization",
  mode: "ephemeral",
  provider: "docker",
  providers: ["docker"],
  labels: ["gridops"],
  image: "runner:default",
  dockerImage: "runner:default",
  tartImage: "",
  macosRuntime: "vm",
  desiredCount: 0,
  minCount: 0,
  maxCount: 10,
  autoscalingEnabled: true,
  queueScaleFactor: 1,
  idleTimeoutMinutes: 5,
  cpuLimit: 2,
  memoryLimitMb: 2048,
  runnerGroupId: 7,
};

// Captured at commit 5550cf5 from src/routes/_app/runner-pools.new.tsx lines 233-263 (old submit):
// repository octo-org/app, then workspaces Mobile delivery and Acme Bitbucket (reverse list order);
// macOS · Tart was added by the workspace pick.
const REPOSITORIES_AND_WORKSPACES_CREATE = {
  installationId: 1,
  repositoryIds: [10],
  bitbucketConnectionIds: ["6a1c1b2e-0000-4000-8000-000000000002", "6a1c1b2e-0000-4000-8000-000000000001"],
  name: "mobile-app",
  scope: "repository",
  mode: "ephemeral",
  provider: "docker",
  providers: ["docker", "tart"],
  labels: ["gridops"],
  image: "runner:default",
  dockerImage: "runner:default",
  tartImage: "fixture-default-tahoe",
  macosRuntime: "vm",
  desiredCount: 0,
  minCount: 0,
  maxCount: 10,
  autoscalingEnabled: true,
  queueScaleFactor: 1,
  idleTimeoutMinutes: 5,
  cpuLimit: 2,
  memoryLimitMb: 2048,
  runnerGroupId: 1,
};

// Captured at commit 5550cf5 from src/routes/_app/runner-pools.$poolId.settings.tsx lines 122-144 (old submit):
// repository pool with octo-org/app (prioritised) and one Bitbucket workspace; octo-org/api added.
const REPOSITORY_POOL_UPDATE = {
  poolId: "pool-1",
  repositoryIds: [10, 11],
  priorityRepositoryIds: [10],
  priorityWaitMinutes: 15,
  name: "macos-app",
  mode: "ephemeral",
  provider: "tart",
  providers: ["tart"],
  labels: ["build"],
  image: "acme-xcode-26",
  dockerImage: "",
  tartImage: "acme-xcode-26",
  macosRuntime: "vm",
  desiredCount: 1,
  minCount: 0,
  maxCount: 4,
  autoscalingEnabled: true,
  queueScaleFactor: 1,
  idleTimeoutMinutes: 5,
  cpuLimit: 4,
  memoryLimitMb: 8192,
  runnerGroupId: 1,
};

// Captured at commit 5550cf5 from src/routes/_app/runner-pools.$poolId.settings.tsx lines 122-144 (old submit):
// organization pool on installation acme (2), switched from Default (1) to the Release runner group (7).
const ORGANIZATION_POOL_UPDATE = {
  poolId: "pool-1",
  repositoryIds: undefined,
  priorityRepositoryIds: undefined,
  priorityWaitMinutes: undefined,
  name: "acme-linux",
  mode: "ephemeral",
  provider: "docker",
  providers: ["docker"],
  labels: ["build"],
  image: "runner:acme",
  dockerImage: "runner:acme",
  tartImage: "",
  macosRuntime: "vm",
  desiredCount: 1,
  minCount: 0,
  maxCount: 4,
  autoscalingEnabled: true,
  queueScaleFactor: 1,
  idleTimeoutMinutes: 5,
  cpuLimit: 4,
  memoryLimitMb: 8192,
  runnerGroupId: 7,
};

// Additional case (manual runner group entry, handoff-A finding 3).
// Captured at commit 5550cf5 from src/routes/_app/runner-pools.new.tsx lines 233-263 (old submit):
// scope Organization, user-account installation jdoe (3), runner group ID typed as 42.
const MANUAL_RUNNER_GROUP_CREATE = {
  installationId: 3,
  repositoryIds: [],
  bitbucketConnectionIds: [],
  name: "jdoe-runners",
  scope: "organization",
  mode: "ephemeral",
  provider: "docker",
  providers: ["docker"],
  labels: ["gridops"],
  image: "runner:default",
  dockerImage: "runner:default",
  tartImage: "",
  macosRuntime: "vm",
  desiredCount: 0,
  minCount: 0,
  maxCount: 10,
  autoscalingEnabled: true,
  queueScaleFactor: 1,
  idleTimeoutMinutes: 5,
  cpuLimit: 2,
  memoryLimitMb: 2048,
  runnerGroupId: 42,
};

// ---- Driving the new forms ----
let container: HTMLDivElement;
let root: Root;
let fetcher: ReturnType<typeof vi.fn>;

beforeEach(() => {
  globalThis.IS_REACT_ACT_ENVIRONMENT = true;
  vi.stubGlobal("ResizeObserver", NoopResizeObserver);
  fetcher = vi.fn(async (url: string, init?: RequestInit) => fetchStub(url, init));
  vi.stubGlobal("fetch", fetcher);
  recorded.create.length = 0;
  recorded.update.length = 0;
  container = document.createElement("div");
  document.body.append(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  document.body.innerHTML = "";
  vi.unstubAllGlobals();
});

async function settle() {
  for (let index = 0; index < 6; index += 1) await act(async () => { await new Promise((resolve) => setTimeout(resolve, 0)); });
}

async function renderPage(route: unknown, data: unknown) {
  loaderData.current = data;
  const page = (route as { options: { component: ComponentType } }).options.component;
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  await act(async () => root.render(createElement(QueryClientProvider, { client }, createElement(TooltipProvider, null, createElement(page)))));
  await settle();
}

/** Picks the Targets option whose text contains every part (opening the picker if it is closed). */
async function pickTarget(...parts: string[]) {
  const button = document.querySelector<HTMLButtonElement>('button[aria-label="Targets"]');
  if (!button) throw new Error("No Targets picker on the form");
  if (button.getAttribute("aria-expanded") !== "true") act(() => button.click());
  const scope = document.getElementById(button.getAttribute("aria-controls") ?? "") ?? document;
  const found = [...scope.querySelectorAll<HTMLElement>("[role=option]")].find((element) => parts.every((part) => element.textContent?.includes(part)));
  if (!found) throw new Error(`No Targets option with ${parts.join(" + ")}`);
  act(() => found.click());
  await settle();
}

function type(input: HTMLInputElement | null, value: string) {
  if (!input) throw new Error("No input");
  const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")?.set;
  act(() => { setter?.call(input, value); input.dispatchEvent(new Event("input", { bubbles: true })); });
}

function labelledInput(label: string) {
  const byLabel = [...document.querySelectorAll<HTMLLabelElement>("label[for]")].find((element) => element.textContent?.trim() === label);
  const found = byLabel ? document.getElementById(byLabel.htmlFor) : document.querySelector(`input[aria-label="${label}"]`);
  return found instanceof HTMLInputElement ? found : null;
}

async function submit() {
  const form = document.querySelector("form");
  if (!form) throw new Error("No form");
  await act(async () => { form.dispatchEvent(new Event("submit", { bubbles: true, cancelable: true })); });
  await settle();
}

// toEqual, not toStrictEqual, on purpose: api() sends JSON.stringify(data), which drops a key whose
// value is undefined, so { repositoryIds: undefined } and {} send the same request. toStrictEqual
// would treat them as different. Any key added or dropped with a defined value still fails.
describe("New pool: the Targets row sends what the old form sent", () => {
  it("GitHub repositories from two installations: click order kept, installation from the loaded list's first selected repository", async () => {
    await renderPage(NewPoolRoute, createOptions());
    type(document.querySelector("#pool-name"), "linux-app");
    await pickTarget("acme/web");
    await pickTarget("octo-org/app");
    await submit();
    expect(recorded.create).toHaveLength(1);
    expect(recorded.create[0]).toEqual(REPOSITORY_CREATE);
  });

  it("GitHub organization runner group: installation and group come from the picked group", async () => {
    await renderPage(NewPoolRoute, createOptions());
    type(document.querySelector("#pool-name"), "acme-release");
    await pickTarget("acme", "Release");
    await submit();
    expect(recorded.create).toHaveLength(1);
    expect(recorded.create[0]).toEqual(RUNNER_GROUP_CREATE);
  });

  it("GitHub repository plus Bitbucket workspaces", async () => {
    await renderPage(NewPoolRoute, createOptions());
    type(document.querySelector("#pool-name"), "mobile-app");
    await pickTarget("octo-org/app");
    await pickTarget("Mobile delivery");
    await pickTarget("Acme Bitbucket");
    await submit();
    expect(recorded.create).toHaveLength(1);
    expect(recorded.create[0]).toEqual(REPOSITORIES_AND_WORKSPACES_CREATE);
  });

  it("manual runner group ID for a user-account installation", async () => {
    await renderPage(NewPoolRoute, createOptions());
    type(document.querySelector("#pool-name"), "jdoe-runners");
    await pickTarget("jdoe", "Enter runner group ID");
    type(labelledInput("Runner group ID"), "42");
    await submit();
    expect(recorded.create).toHaveLength(1);
    expect(recorded.create[0]).toEqual(MANUAL_RUNNER_GROUP_CREATE);
  });
});

describe("Pool settings: the Targets row saves what the old form saved", () => {
  it("repository pool: repositories, priority and fair-share wait; the Bitbucket workspace is not sent", async () => {
    await renderPage(PoolSettingsRoute, REPOSITORY_POOL);
    await pickTarget("octo-org/api");
    await submit();
    expect(recorded.update).toHaveLength(1);
    expect(recorded.update[0]).toEqual(REPOSITORY_POOL_UPDATE);
  });

  it("organization pool: the picked runner group, and no repository or priority fields", async () => {
    await renderPage(PoolSettingsRoute, ORGANIZATION_POOL);
    await pickTarget("acme", "Release");
    await submit();
    expect(recorded.update).toHaveLength(1);
    expect(recorded.update[0]).toEqual(ORGANIZATION_POOL_UPDATE);
  });
});
