// @vitest-environment jsdom
/**
 * The Tart base VM on the runner pool forms (issue #97): the create form asks
 * for it whenever the pool includes macOS · Tart with the VM runtime, prefilled
 * with the server default, and sends what the operator typed; the settings
 * form shows the same row with the same wording.
 */
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { act, createElement, type ComponentType, type ReactElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const loaderData = vi.hoisted(() => ({ current: undefined as unknown }));
const navigate = vi.hoisted(() => vi.fn());

vi.mock("@tanstack/react-router", () => ({
  createFileRoute: () => (options: Record<string, unknown>) => ({ options, useLoaderData: () => loaderData.current }),
  getRouteApi: () => ({ useLoaderData: () => loaderData.current }),
  Link: ({ children, to }: { children?: unknown; to?: string }) => createElement("a", { href: String(to) }, children as never),
  useNavigate: () => navigate,
}));
vi.mock("sonner", () => ({ toast: { success: vi.fn(), error: vi.fn() } }));

import { Route as NewPoolRoute } from "../src/routes/_app/runner-pools.new";
import { Route as PoolSettingsRoute } from "../src/routes/_app/runner-pools.$poolId.settings";

declare global {
  var IS_REACT_ACT_ENVIRONMENT: boolean | undefined;
}

class NoopResizeObserver {
  observe() {}
  unobserve() {}
  disconnect() {}
}

const SERVER_DEFAULT_VM = "fixture-default-tahoe";
const repository = { id: 10, installationId: 1, accountLogin: "octo-org", accountType: "Organization", fullName: "octo-org/app", private: true };

const BITBUCKET = { id: "6a1c1b2e-0000-4000-8000-000000000001", name: "Acme Bitbucket", workspace: "acme", workspaceUuid: "{acme}" };

function createOptions(defaults: { providers: Array<"docker" | "tart">; macosRuntime?: "vm" | "native" }) {
  return {
    authenticated: true,
    installUrl: "https://github.com/apps/gridops/installations/new",
    builtinRunnerHostEnabled: true,
    installations: [{ id: 1, accountLogin: "octo-org", accountType: "Organization" }],
    repositories: [],
    runnerGroups: [],
    bitbucketConnections: [BITBUCKET],
    defaults: {
      provider: defaults.providers[0], providers: defaults.providers, image: "runner:default",
      dockerImage: "runner:default", tartImage: SERVER_DEFAULT_VM, macosRuntime: defaults.macosRuntime ?? "vm",
      labels: ["gridops"], cpuLimit: 2, memoryLimitMb: 2048, desiredCount: 0, minCount: 0, maxCount: 10,
      autoscalingEnabled: true, queueScaleFactor: 1, idleTimeoutMinutes: 5, runnerGroupId: 1,
      maxCpuLimit: 16, maxMemoryLimitMb: 65_536,
    },
  };
}

let container: HTMLDivElement;
let root: Root;
let fetcher: ReturnType<typeof vi.fn>;

beforeEach(() => {
  globalThis.IS_REACT_ACT_ENVIRONMENT = true;
  vi.stubGlobal("ResizeObserver", NoopResizeObserver);
  fetcher = vi.fn(async (url: string, init?: RequestInit) => {
    if (url === "/api/v1/runner-pools/repositories") return Response.json({ items: [repository] });
    if (url === "/api/v1/runner-pools" && init?.method === "POST") return Response.json({ id: "pool-1" }, { status: 201 });
    return Response.json({ error: "not in this test" }, { status: 404 });
  });
  vi.stubGlobal("fetch", fetcher);
  container = document.createElement("div");
  document.body.append(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  document.body.innerHTML = "";
  vi.unstubAllGlobals();
  navigate.mockReset();
});

async function render(element: ReactElement) {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  await act(async () => root.render(createElement(QueryClientProvider, { client }, element)));
  await settle();
}

async function settle() {
  for (let index = 0; index < 5; index += 1) await act(async () => { await new Promise((resolve) => setTimeout(resolve, 0)); });
}

async function renderCreateForm(defaults: Parameters<typeof createOptions>[0]) {
  loaderData.current = createOptions(defaults);
  const page = (NewPoolRoute as unknown as { options: { component: ComponentType } }).options.component;
  await render(createElement(page));
}

const tartInput = () => document.querySelector<HTMLInputElement>("#tart-image");
const dockerInput = () => document.querySelector<HTMLInputElement>("#docker-image");
const labelFor = (id: string) => document.querySelector<HTMLLabelElement>(`label[for="${id}"]`);
const descriptionFor = (id: string) => labelFor(id)?.nextElementSibling?.textContent ?? "";

function button(name: string) {
  const found = document.querySelector<HTMLButtonElement>(`button[aria-label="${name}"]`);
  if (!found) throw new Error(`No "${name}" button`);
  return found;
}

function option(label: string) {
  const found = [...document.querySelectorAll<HTMLElement>("[role=option]")].find((element) => element.textContent?.includes(label));
  if (!found) throw new Error(`No "${label}" option`);
  return found;
}

async function choose(control: string, label: string) {
  act(() => button(control).click());
  act(() => option(label).click());
  await settle();
}

function type(input: HTMLInputElement, value: string) {
  const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")?.set;
  act(() => {
    setter?.call(input, value);
    input.dispatchEvent(new Event("input", { bubbles: true }));
  });
}

async function submitCreate() {
  const name = document.querySelector<HTMLInputElement>("#pool-name");
  if (!name) throw new Error("No pool name input");
  type(name, "macos-app");
  await choose("Repositories", "octo-org/app");
  const form = document.querySelector("form");
  if (!form) throw new Error("No form");
  await act(async () => { form.dispatchEvent(new Event("submit", { bubbles: true, cancelable: true })); });
  await settle();
  const call = fetcher.mock.calls.find(([url, init]) => url === "/api/v1/runner-pools" && (init as RequestInit | undefined)?.method === "POST");
  if (!call) throw new Error(`No create request; page says: ${document.body.textContent?.slice(0, 400)}`);
  return JSON.parse(String((call[1] as RequestInit).body)) as Record<string, unknown>;
}

describe("New runner pool: Tart base VM", () => {
  it("asks for the Tart base VM for a macOS · Tart pool, prefilled with the server default", async () => {
    await renderCreateForm({ providers: ["tart"] });

    expect(labelFor("tart-image")?.textContent).toBe("Tart base VM");
    expect(tartInput()?.value).toBe(SERVER_DEFAULT_VM);
    expect(tartInput()?.className).toContain("font-mono");
    expect(tartInput()?.required).toBe(true);
    expect(descriptionFor("tart-image")).toContain("scripts/prepare-tart-runner-image.sh");
    expect(descriptionFor("tart-image")).toMatch(/local Tart VM/);
    expect(dockerInput()).toBeNull();
  });

  it("has no Tart base VM for a Docker-only pool, and shows it once macOS · Tart is added", async () => {
    await renderCreateForm({ providers: ["docker"] });
    expect(tartInput()).toBeNull();
    expect(labelFor("tart-image")).toBeNull();
    expect(dockerInput()?.value).toBe("runner:default");

    await choose("Runner providers", "macOS · Tart");
    expect(tartInput()?.value).toBe(SERVER_DEFAULT_VM);
    expect(dockerInput()?.value).toBe("runner:default");
  });

  it("hides the Tart base VM for native execution, which clones no VM", async () => {
    await renderCreateForm({ providers: ["tart"] });
    expect(tartInput()).not.toBeNull();
    await choose("macOS execution", "Native on host");
    expect(tartInput()).toBeNull();
    await choose("macOS execution", "Virtual machine");
    expect(tartInput()?.value).toBe(SERVER_DEFAULT_VM);
  });

  it("points a Bitbucket pool at the Bitbucket preparation script", async () => {
    await renderCreateForm({ providers: ["docker"] });
    await choose("Bitbucket Cloud workspaces", "Acme Bitbucket");
    expect(tartInput()?.value).toBe(SERVER_DEFAULT_VM);
    expect(descriptionFor("tart-image")).toContain("scripts/prepare-tart-bitbucket-runner-image.sh");
  });

  it("sends the typed Tart base VM, not the default, for a Tart pool", async () => {
    await renderCreateForm({ providers: ["tart"] });
    const input = tartInput();
    if (!input) throw new Error("No Tart base VM input");
    type(input, "acme-xcode-26");

    const body = await submitCreate();
    expect(body).toMatchObject({ provider: "tart", providers: ["tart"], macosRuntime: "vm", tartImage: "acme-xcode-26", image: "acme-xcode-26" });
  });

  it("sends both images for a pool with Docker and Tart", async () => {
    await renderCreateForm({ providers: ["docker", "tart"] });
    const tart = tartInput();
    const docker = dockerInput();
    if (!tart || !docker) throw new Error("Both image inputs should be on the form");
    type(tart, "acme-xcode-26");
    type(docker, "ghcr.io/acme/runner:2");

    const body = await submitCreate();
    expect(body).toMatchObject({
      provider: "docker", providers: ["docker", "tart"],
      image: "ghcr.io/acme/runner:2", dockerImage: "ghcr.io/acme/runner:2", tartImage: "acme-xcode-26",
    });
  });
});

describe("Pool settings: Tart base VM", () => {
  it("uses the same row and wording as the create form", async () => {
    await renderCreateForm({ providers: ["tart"] });
    const createDescription = descriptionFor("tart-image");
    act(() => root.unmount());
    root = createRoot(container);

    loaderData.current = {
      id: "pool-1", installationId: 1, repositoryId: null, repository: null, repositoryIds: [], repositories: [],
      priorityRepositoryIds: [], priorityWaitMinutes: 10, bitbucketConnections: [], bitbucketConnectionIds: [],
      accountLogin: "octo-org", name: "macos-app", scope: "organization", mode: "ephemeral", provider: "tart",
      providers: ["tart"], labels: [], image: "acme-xcode-26", dockerImage: "runner:default", tartImage: "acme-xcode-26",
      macosRuntime: "vm", desiredCount: 0, minCount: 0, maxCount: 2, cpuLimit: 4, memoryLimitMb: 8192,
      maxCpuLimit: 16, maxMemoryLimitMb: 65_536, builtinRunnerHostEnabled: true, runnerGroupId: 1, paused: false,
      state: "active", autoscalingEnabled: true, queueScaleFactor: 1, idleTimeoutMinutes: 5, configurationVersion: 1,
      provisionFailureCount: 0, provisionRetryAt: null, provisionCircuitOpen: false, canManage: true,
    };
    const page = (PoolSettingsRoute as unknown as { options: { component: ComponentType } }).options.component;
    await render(createElement(page));

    expect(labelFor("tart-image")?.textContent).toBe("Tart base VM");
    expect(tartInput()?.value).toBe("acme-xcode-26");
    expect(descriptionFor("tart-image")).toBe(createDescription);
  });
});
