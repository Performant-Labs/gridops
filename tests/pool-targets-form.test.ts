// @vitest-environment jsdom
/**
 * The one Targets row on the runner pool forms (#145), driven through the rendered routes with the
 * production registry (GitHub + Bitbucket): the mixing rules, the per-kind caps, the Tart auto-add,
 * the create-button gate, the conditional Platform options section, and the pool settings form
 * offering only the pool's own kind.
 */
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { act, createElement, type ComponentType } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import {
  BITBUCKET, BITBUCKET_SCRIPT, buttonByText, createOptions, input, isDisabled, openPicker, optionIn,
  pick, platformOptionsToggle, poolFormFetch, settingsPool, settle, submitForm, type,
} from "./fixtures/pool-forms";

const loaderData = vi.hoisted(() => ({ current: undefined as unknown }));

vi.mock("@tanstack/react-router", () => ({
  createFileRoute: () => (options: Record<string, unknown>) => ({ options, useLoaderData: () => loaderData.current }),
  getRouteApi: () => ({ useLoaderData: () => loaderData.current }),
  Link: ({ children, to }: { children?: unknown; to?: string }) => createElement("a", { href: String(to) }, children as never),
  useNavigate: () => vi.fn(),
}));
vi.mock("sonner", () => ({ toast: { success: vi.fn(), error: vi.fn() } }));

import { TooltipProvider } from "../src/components/ui/tooltip";
import { Route as NewPoolRoute } from "../src/routes/_app/runner-pools.new";
import { Route as PoolSettingsRoute } from "../src/routes/_app/runner-pools.$poolId.settings";

declare global {
  var IS_REACT_ACT_ENVIRONMENT: boolean | undefined;
}

class NoopResizeObserver { observe() {} unobserve() {} disconnect() {} }

const NOT_STANDALONE_REASON = "Select a GitHub repository or runner group first";

let container: HTMLDivElement;
let root: Root;
let fetcher: ReturnType<typeof vi.fn>;

beforeEach(() => {
  globalThis.IS_REACT_ACT_ENVIRONMENT = true;
  vi.stubGlobal("ResizeObserver", NoopResizeObserver);
  fetcher = vi.fn(async (url: string, init?: RequestInit) => poolFormFetch(url, init));
  vi.stubGlobal("fetch", fetcher);
  container = document.createElement("div");
  document.body.append(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  document.body.innerHTML = "";
  vi.unstubAllGlobals();
});

async function renderPage(route: unknown, data: unknown) {
  loaderData.current = data;
  const page = (route as { options: { component: ComponentType } }).options.component;
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  await act(async () => root.render(createElement(QueryClientProvider, { client }, createElement(TooltipProvider, null, createElement(page)))));
  await settle();
}

const renderCreate = (options = createOptions()) => renderPage(NewPoolRoute, options);
const target = (...parts: string[]) => optionIn("Targets", ...parts);
const createRequests = () => fetcher.mock.calls.filter(([url, init]) => url === "/api/v1/runner-pools" && (init as RequestInit | undefined)?.method === "POST");

describe("New pool Targets row: mixing rules", () => {
  it("(a) shows every Bitbucket workspace disabled with a reason until a GitHub repository or runner group is picked", async () => {
    await renderCreate();
    for (const connection of BITBUCKET.slice(0, 2)) {
      expect(isDisabled(target(connection.name))).toBe(true);
      expect(target(connection.name).textContent).toContain(NOT_STANDALONE_REASON);
    }

    await pick("Targets", "octo-org/app");
    for (const connection of BITBUCKET.slice(0, 2)) expect(isDisabled(target(connection.name))).toBe(false);

    await pick("Targets", "octo-org/app");
    expect(isDisabled(target("Acme Bitbucket"))).toBe(true);
    await pick("Targets", "acme", "Release");
    for (const connection of BITBUCKET.slice(0, 2)) expect(isDisabled(target(connection.name))).toBe(false);
  });

  it("(b) disables runner groups while a repository is picked, and repositories while a runner group is picked", async () => {
    await renderCreate();
    await pick("Targets", "octo-org/app");
    expect(isDisabled(target("acme", "Release"))).toBe(true);
    expect(target("acme", "Release").textContent).toMatch(/combine with repositories/);
    expect(isDisabled(target("octo-org/api"))).toBe(false);

    await pick("Targets", "octo-org/app");
    await pick("Targets", "acme", "Release");
    expect(isDisabled(target("octo-org/app"))).toBe(true);
    expect(target("octo-org/app").textContent).toMatch(/combine with a runner group/);
  });

  it("(c) caps repositories and workspaces at the maximum runner count separately", async () => {
    await renderCreate(createOptions({ bitbucketConnections: BITBUCKET, defaults: { maxCount: 2 } }));
    await pick("Targets", "octo-org/app");
    await pick("Targets", "Acme Bitbucket");
    await pick("Targets", "Mobile delivery");
    // Two workspaces reach the workspace cap without using the repositories' room.
    expect(isDisabled(target("Web platform"))).toBe(true);
    expect(isDisabled(target("octo-org/api"))).toBe(false);

    await pick("Targets", "octo-org/api");
    expect(isDisabled(target("acme/web"))).toBe(true);
  });

  it("(d) adds macOS · Tart when a Bitbucket workspace is picked", async () => {
    await renderCreate();
    expect(document.querySelector("#tart-image")).toBeNull();
    await pick("Targets", "octo-org/app");
    await pick("Targets", "Acme Bitbucket");
    expect(input("#tart-image").value).toBe("fixture-default-tahoe");
  });

  it("(e) keeps the workspace when the last GitHub target is removed, and submit fails in the schema with no request", async () => {
    await renderCreate();
    type(input("#pool-name"), "mobile-app");
    await pick("Targets", "octo-org/app");
    await pick("Targets", "Acme Bitbucket");
    await pick("Targets", "octo-org/app");

    expect(target("Acme Bitbucket").getAttribute("aria-selected")).toBe("true");
    expect(document.querySelector('button[aria-label="Remove Acme Bitbucket"]')).not.toBeNull();
    expect(isDisabled(target("Mobile delivery"))).toBe(true);
    expect(buttonByText("Create pool")?.disabled).toBe(false);

    await submitForm();
    expect(createRequests()).toHaveLength(0);
    expect(document.body.textContent).toContain("Could not create the pool");
  });

  it("raises the Maximum floor for selected repositories only", async () => {
    await renderCreate();
    expect(input('input[name="maxCount"]').min).toBe("1");
    await pick("Targets", "octo-org/app");
    await pick("Targets", "Acme Bitbucket");
    await pick("Targets", "Mobile delivery");
    expect(input('input[name="maxCount"]').min).toBe("1");
    await pick("Targets", "octo-org/api");
    expect(input('input[name="maxCount"]').min).toBe("2");
  });
});

describe("New pool: create-button gate (today: submitting || installations.length === 0)", () => {
  it.each([
    ["no Bitbucket connection", []],
    ["Bitbucket connections", BITBUCKET.slice(0, 2)],
  ])("(c)/(d)/(e) is enabled with an installation and %s, before any target is picked", async (_label, bitbucketConnections) => {
    await renderCreate(createOptions({ bitbucketConnections }));
    expect(document.querySelector('button[aria-label="Targets"]')).not.toBeNull();
    expect(buttonByText("Create pool")?.disabled).toBe(false);
  });

  it.each([
    ["no Bitbucket connection", []],
    ["Bitbucket connections", BITBUCKET.slice(0, 2)],
  ])("(a)/(b) with no installation and %s, shows today's Install the GitHub App state instead of the form", async (_label, bitbucketConnections) => {
    await renderCreate(createOptions({ installations: [], bitbucketConnections }));
    expect(document.body.textContent).toContain("Install the GitHub App");
    expect(buttonByText("Create pool")).toBeNull();
  });

  it("(f) is disabled while the pool is being created", async () => {
    let release: (response: Response) => void = () => undefined;
    fetcher.mockImplementation(async (url: string, init?: RequestInit) => (url === "/api/v1/runner-pools" && init?.method === "POST")
      ? new Promise<Response>((resolve) => { release = resolve; })
      : poolFormFetch(url, init));
    await renderCreate();
    type(input("#pool-name"), "linux-app");
    await pick("Targets", "octo-org/app");
    await submitForm();
    expect(createRequests()).toHaveLength(1);
    expect(buttonByText("Creating…")?.disabled).toBe(true);
    await act(async () => { release(Response.json({ id: "pool-new" }, { status: 201 })); });
    await settle();
  });
});

describe("New pool: Platform options", () => {
  it("is absent with no target, collapsed once a platform with options is targeted, and follows the selection", async () => {
    await renderCreate();
    expect(platformOptionsToggle()).toBeNull();

    await pick("Targets", "octo-org/app");
    await pick("Targets", "Acme Bitbucket");
    const toggle = platformOptionsToggle();
    expect(toggle?.getAttribute("aria-expanded")).toBe("false");
    act(() => toggle?.click());
    expect(platformOptionsToggle()?.getAttribute("aria-expanded")).toBe("true");
    expect(document.body.textContent).toContain(BITBUCKET_SCRIPT);

    await pick("Targets", "Acme Bitbucket");
    expect(document.body.textContent).not.toContain(BITBUCKET_SCRIPT);
  });
});

describe("Pool settings Targets row: only the pool's own kind", () => {
  it("repository pool: offers repositories only, shows its Bitbucket workspace locked, keeps Priority", async () => {
    await renderPage(PoolSettingsRoute, settingsPool({
      scope: "repository", repositoryIds: [10], repositories: [{ id: 10, installationId: 1, accountLogin: "octo-org", accountType: "Organization", fullName: "octo-org/app", private: true }],
      priorityRepositoryIds: [10], bitbucketConnections: [BITBUCKET[0]], bitbucketConnectionIds: [BITBUCKET[0]?.id],
      provider: "tart", providers: ["tart"], tartImage: "acme-xcode-26", dockerImage: "",
    }));
    const options = [...openPicker("Targets").querySelectorAll("[role=option]")].map((element) => element.textContent ?? "");
    expect(options.some((text) => text.includes("octo-org/api"))).toBe(true);
    expect(options.some((text) => /Default|Release|Linux builds|Enter runner group ID/.test(text))).toBe(false);
    expect(options.some((text) => text.includes("Mobile delivery") || text.includes("Acme Bitbucket"))).toBe(false);

    expect(document.body.textContent).toContain("Acme Bitbucket");
    const remove = document.querySelector<HTMLButtonElement>('button[aria-label="Remove Acme Bitbucket"]');
    expect(remove === null || remove.disabled).toBe(true);
    expect(document.querySelector('button[aria-label="Prioritised repositories"]')).not.toBeNull();
  });

  it("organization pool: offers runner groups of its own installation only, and no Priority", async () => {
    await renderPage(PoolSettingsRoute, settingsPool({ scope: "organization", installationId: 2, accountLogin: "acme", runnerGroupId: 1 }));
    const options = [...openPicker("Targets").querySelectorAll("[role=option]")].map((element) => element.textContent ?? "");
    expect(options.some((text) => text.includes("Release"))).toBe(true);
    expect(options.some((text) => text.includes("Linux builds"))).toBe(false);
    expect(options.some((text) => /octo-org\/|acme\/web/.test(text))).toBe(false);
    expect(document.querySelector('button[aria-label="Prioritised repositories"]')).toBeNull();
  });

  it("read-only pool: lists its targets per platform through the registry, Priority only for a repository pool", async () => {
    await renderPage(PoolSettingsRoute, settingsPool({
      canManage: false, scope: "repository", repositoryIds: [10, 11], bitbucketConnections: [BITBUCKET[0]], bitbucketConnectionIds: [BITBUCKET[0]?.id],
      provider: "tart", providers: ["tart"], tartImage: "acme-xcode-26",
    }));
    const text = document.body.textContent ?? "";
    expect(text).toContain("GitHub");
    expect(text).toContain("2 repositories");
    expect(text).toContain("Bitbucket");
    expect(text).toContain("Fair-share wait");

    act(() => root.unmount());
    root = createRoot(container);
    await renderPage(PoolSettingsRoute, settingsPool({ canManage: false, scope: "organization", installationId: 2, accountLogin: "acme" }));
    expect(document.body.textContent).toContain("acme");
    expect(document.body.textContent).toContain("GitHub");
    expect(document.body.textContent).not.toContain("Fair-share wait");
  });
});
