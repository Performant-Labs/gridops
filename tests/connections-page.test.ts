// @vitest-environment jsdom
/**
 * Settings → Connections (#145): one list of every platform's connections, driven by the registry.
 * Covers the GitHub rows from the settings page data, the Bitbucket create / edit / remove flows,
 * permission gating, per-platform load failure with its own Retry, the generic one-time reveal
 * after create, and the GitHub App section moved from the old GitHub page.
 */
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { act, createElement, type ComponentType, type ReactElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const loaderData = vi.hoisted(() => ({ current: undefined as unknown }));
const toast = vi.hoisted(() => ({ success: vi.fn(), error: vi.fn(), promise: vi.fn() }));

vi.mock("@tanstack/react-router", () => ({
  createFileRoute: () => (options: Record<string, unknown>) => ({ options, useLoaderData: () => loaderData.current }),
  getRouteApi: () => ({ useLoaderData: () => loaderData.current }),
  Link: ({ children, to }: { children?: unknown; to?: string }) => createElement("a", { href: String(to) }, children as never),
  useNavigate: () => vi.fn(),
  useRouter: () => ({ invalidate: vi.fn(async () => undefined) }),
}));
vi.mock("sonner", () => ({ toast }));

import { TooltipProvider } from "../src/components/ui/tooltip";
import { ConnectionsSection } from "../src/features/platform-connections/connections-section";
import { GitHubAppSection } from "../src/features/platform-connections/github-app-section";
import type { BitbucketConnection } from "../src/features/platform-connections/platform-connections.functions";
import { getPlatform } from "../src/features/platform-connections/registry";
import type { ConnectionList, PlatformDescriptor, SecretReveal } from "../src/features/platform-connections/types";
import type { SettingsPage } from "../src/features/operations/operations.functions";
import { Route as ConnectionsRoute } from "../src/routes/settings.connections";

declare global {
  var IS_REACT_ACT_ENVIRONMENT: boolean | undefined;
}

class NoopResizeObserver { observe() {} unobserve() {} disconnect() {} }

const BITBUCKET_URL = "/api/v1/platform-connections/bitbucket";
const MOBILE: BitbucketConnection = {
  id: "6a1c1b2e-0000-4000-8000-000000000002", provider: "bitbucket", name: "Mobile delivery", workspace: "acme-mobile", workspaceUuid: "{acme-mobile}",
  createdAt: "2026-11-01T20:00:00.000Z", updatedAt: "2026-11-19T20:00:00.000Z", canManage: true,
};

type PageData = NonNullable<SettingsPage["data"]>;

function settingsPage(role: "admin" | "member", overrides: Partial<PageData["configuration"]> = {}, data: Partial<PageData> = {}): SettingsPage {
  return {
    authenticated: true,
    data: {
      configuration: {
        githubOAuth: true, githubAppControl: true, webhookActive: false, webhookVerification: false, secureStorage: true, runnerManager: true,
        installationTokens: true, callbackUrl: "https://gridops.example/auth/github/callback", webhookUrl: "https://gridops.example/api/v1/webhooks/github",
        ...overrides,
      },
      githubApp: { slug: "gridops-self-hosted", appUrl: "https://github.com/apps/gridops-self-hosted", installUrl: "https://github.com/apps/gridops-self-hosted/installations/new" },
      installations: [
        { id: 1, accountLogin: "octo-org", accountType: "Organization", accountAvatarUrl: null, repositorySelection: "all", permission: "admin", suspended: false, lastSyncedAt: null, poolCount: 3, manageUrl: "https://github.com/organizations/octo-org/settings/installations/1" },
        { id: 3, accountLogin: "jdoe", accountType: "User", accountAvatarUrl: null, repositorySelection: "selected", permission: "read", suspended: true, lastSyncedAt: null, poolCount: 0, manageUrl: "https://github.com/settings/installations/3" },
      ],
      manager: { ok: true },
      settings: {
        logRetentionDays: 30, logStorageBudgetMb: 1024, webhookRetentionDays: 30, auditRetentionDays: 90, reconcileIntervalSeconds: 30,
        githubSyncIntervalSeconds: 300, autoUpdateImages: false, provisioningPaused: false, builtinRunnerHostEnabled: true,
      },
      user: { id: "u-1", login: role === "admin" ? "octocat" : "member-1", role },
      users: [],
      ...data,
    },
  };
}

let container: HTMLDivElement;
let root: Root;
let fetcher: ReturnType<typeof vi.fn>;
let bitbucketList: { canManage: boolean; items: BitbucketConnection[] };

function apiFetch(url: string, init?: RequestInit) {
  const method = init?.method ?? "GET";
  if (method === "GET" && url === BITBUCKET_URL) return Response.json(bitbucketList);
  // GitLab (#141) is registered too; these tests keep its list empty.
  if (method === "GET" && url === "/api/v1/platform-connections/gitlab") return Response.json({ canManage: true, items: [] });
  if (method === "POST" && url === BITBUCKET_URL) return Response.json({ id: "new-1", provider: "bitbucket", name: "Web platform", workspace: "acme-web", workspaceUuid: "{acme-web}" }, { status: 201 });
  if ((method === "PUT" || method === "DELETE") && url === `${BITBUCKET_URL}/${MOBILE.id}`) return method === "PUT" ? Response.json(MOBILE) : new Response(null, { status: 204 });
  if (method === "POST" && url === "/api/v1/repositories/sync") return Response.json({ repositories: 4 });
  if (method === "POST" && url === "/api/v1/github-app/manifest") return Response.json({ action: "https://github.com/settings/apps/new", state: "state-1", manifest: "{}", webhookActive: false });
  return Response.json({ error: `not in this test: ${method} ${url}` }, { status: 404 });
}

beforeEach(() => {
  globalThis.IS_REACT_ACT_ENVIRONMENT = true;
  vi.stubGlobal("ResizeObserver", NoopResizeObserver);
  bitbucketList = { canManage: true, items: [MOBILE] };
  fetcher = vi.fn(async (url: string, init?: RequestInit) => apiFetch(url, init));
  vi.stubGlobal("fetch", fetcher);
  container = document.createElement("div");
  document.body.append(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  document.body.innerHTML = "";
  vi.unstubAllGlobals();
  for (const fn of Object.values(toast)) fn.mockReset();
});

async function render(element: ReactElement) {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  await act(async () => root.render(createElement(QueryClientProvider, { client }, createElement(TooltipProvider, null, element))));
  await settle();
}

async function settle() {
  for (let index = 0; index < 6; index += 1) await act(async () => { await new Promise((resolve) => setTimeout(resolve, 0)); });
}

const click = async (element: HTMLElement | null | undefined) => {
  if (!element) throw new Error("Nothing to click");
  act(() => element.click());
  await settle();
};
const buttons = (text: string) => [...document.querySelectorAll<HTMLButtonElement>("button")].filter((element) => element.textContent?.trim() === text);
const body = () => document.body.textContent ?? "";
const calls = (method: string, url: string) => (fetcher.mock.calls as Array<[string, RequestInit | undefined]>)
  .filter(([callUrl, init]) => callUrl === url && (init?.method ?? "GET") === method);

function type(input: HTMLInputElement, value: string) {
  const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")?.set;
  act(() => { setter?.call(input, value); input.dispatchEvent(new Event("input", { bubbles: true })); });
}

function field(label: string) {
  const byLabel = [...document.querySelectorAll<HTMLLabelElement>("label[for]")].find((element) => element.textContent?.trim().startsWith(label));
  const found = byLabel ? document.getElementById(byLabel.htmlFor) : null;
  if (!(found instanceof HTMLInputElement)) throw new Error(`No field labelled "${label}"`);
  return found;
}

/** Add connection → choose the platform (platform first) → its fields. */
async function startAddConnection(platformName: string) {
  await click(buttons("Add connection")[0]);
  const dialog = document.querySelector<HTMLElement>("[role=dialog]");
  if (!dialog) throw new Error("Add connection opened no dialog");
  const choice = [...dialog.querySelectorAll<HTMLElement>("button, [role=radio], label")]
    .find((element) => element.textContent?.includes(platformName) && !/^(Cancel|Continue)/.test(element.textContent.trim()));
  if (!choice) throw new Error(`Add connection does not offer "${platformName}"`);
  await click(choice);
  const next = [...dialog.querySelectorAll<HTMLButtonElement>("button")].find((element) => element.textContent?.includes("Continue"));
  if (next && !next.disabled) await click(next);
}

async function fillBitbucketFields(name: string) {
  type(field("Connection name"), name);
  type(field("Workspace slug"), "acme-web");
  type(field("API token"), "bb-token-value");
  const form = field("API token").form;
  if (!form) throw new Error("The connection fields are not in a form");
  await act(async () => { form.requestSubmit(); });
  await settle();
}

const github = () => getPlatform("github");
const bitbucket = () => getPlatform("bitbucket");

/** A test-only platform: the Bitbucket descriptor under another id and name, with its own bindings. */
function testPlatform(id: string, name: string, overrides: Partial<PlatformDescriptor["connections"]>, extra: Partial<PlatformDescriptor> = {}): PlatformDescriptor {
  const base = bitbucket();
  return { ...base, id, name, connections: { ...base.connections, ...overrides }, ...extra };
}

describe("Connections list", () => {
  it("lists GitHub installations from the settings page data and Bitbucket workspaces from their binding", async () => {
    await render(createElement(ConnectionsSection, { page: settingsPage("admin") }));
    const text = body();
    expect(text).toContain("octo-org");
    expect(text).toContain("jdoe");
    expect(text).toContain("Mobile delivery");
    expect(text).toContain("GitHub");
    expect(text).toContain("Bitbucket");
    const configure = document.querySelector<HTMLAnchorElement>('a[href="https://github.com/organizations/octo-org/settings/installations/1"]');
    expect(configure?.textContent).toContain("Configure");
    expect(text).toContain("Owner approval required");
    expect(text).toContain("Suspended");
    expect(calls("GET", BITBUCKET_URL)).toHaveLength(1);
  });

  it.each([
    ["shows", "admin", true, 1],
    ["hides", "member", false, 0],
  ] as const)("%s Add connection, Edit and Remove when the descriptor reports the user %s can manage", async (_verb, role, canManage, count) => {
    bitbucketList = { canManage, items: [MOBILE] };
    await render(createElement(ConnectionsSection, { page: settingsPage(role) }));
    expect(body()).toContain("Mobile delivery");
    expect(buttons("Add connection").length > 0).toBe(count > 0);
    expect(buttons("Edit")).toHaveLength(count);
    expect(buttons("Remove")).toHaveLength(count);
  });
});

describe("Bitbucket connection flows", () => {
  it("creates a workspace: the error stays inline first, then a retry connects and reloads only Bitbucket", async () => {
    let fail = true;
    fetcher.mockImplementation(async (url: string, init?: RequestInit) => (fail && init?.method === "POST" && url === BITBUCKET_URL)
      ? Response.json({ error: "The token lacks read:runner." }, { status: 422 })
      : apiFetch(url, init));
    await render(createElement(ConnectionsSection, { page: settingsPage("admin") }));
    await startAddConnection("Bitbucket");
    await fillBitbucketFields("Web platform");
    expect(body()).toContain("Could not connect");
    expect(body()).toContain("The token lacks read:runner.");

    fail = false;
    const form = field("API token").form;
    await act(async () => { form?.requestSubmit(); });
    await settle();
    const posts = calls("POST", BITBUCKET_URL);
    expect(posts).toHaveLength(2);
    expect(JSON.parse(String(posts[1]?.[1]?.body))).toEqual({ name: "Web platform", workspace: "acme-web", accessToken: "bb-token-value" });
    expect(toast.success).toHaveBeenCalledWith("Bitbucket workspace connected.");
    expect(calls("GET", BITBUCKET_URL)).toHaveLength(2);
    expect(document.querySelector("[role=dialog]")).toBeNull();
  });

  it("edits only the editable fields; a blank token keeps the current one", async () => {
    await render(createElement(ConnectionsSection, { page: settingsPage("admin") }));
    await click(buttons("Edit")[0]);
    expect(() => field("Workspace slug")).toThrow();
    type(field("Connection name"), "Mobile apps");
    expect(field("New API token").value).toBe("");
    await click(buttons("Save")[0]);
    const puts = calls("PUT", `${BITBUCKET_URL}/${MOBILE.id}`);
    expect(puts).toHaveLength(1);
    expect(JSON.parse(String(puts[0]?.[1]?.body))).toEqual({ name: "Mobile apps" });
    expect(toast.success).toHaveBeenCalledWith("Connection updated.");
  });

  it("removes a workspace after today's confirmation", async () => {
    const confirm = vi.fn(() => true);
    vi.stubGlobal("confirm", confirm);
    await render(createElement(ConnectionsSection, { page: settingsPage("admin") }));
    await click(buttons("Remove")[0]);
    expect(confirm).toHaveBeenCalledWith("Remove Mobile delivery? GridOps deletes its stored token. Pools that use it must be changed first.");
    expect(calls("DELETE", `${BITBUCKET_URL}/${MOBILE.id}`)).toHaveLength(1);
  });
});

describe("Per-platform loading", () => {
  it("renders the platforms that loaded, an error with Retry for the one that failed, and Retry re-runs only that load", async () => {
    const failing = vi.fn(async () => { throw new Error("FailCI is unreachable"); });
    const working = vi.fn(async () => ({ canManage: true, items: [{ ...MOBILE, id: "ok-1", name: "OK project", workspace: "ok-project" }] }));
    const descriptors = [github(), testPlatform("failci", "FailCI", { list: { kind: "fetch", load: failing } }), testPlatform("okci", "OKCI", { list: { kind: "fetch", load: working } })];
    await render(createElement(ConnectionsSection, { descriptors, page: settingsPage("admin") }));

    expect(body()).toContain("octo-org");
    expect(body()).toContain("OK project");
    const alerts = [...document.querySelectorAll<HTMLElement>("[role=alert]")];
    expect(alerts).toHaveLength(1);
    expect(alerts[0]?.textContent).toContain("FailCI");
    expect(failing).toHaveBeenCalledTimes(1);
    expect(working).toHaveBeenCalledTimes(1);

    await click([...(alerts[0]?.querySelectorAll<HTMLButtonElement>("button") ?? [])].find((element) => element.textContent?.includes("Retry")));
    expect(failing).toHaveBeenCalledTimes(2);
    expect(working).toHaveBeenCalledTimes(1);
    expect(body()).toContain("octo-org");
  });

  it("with the default (production) registry, a failed Bitbucket list shows its own Retry and GitHub rows still render", async () => {
    fetcher.mockImplementation(async (url: string, init?: RequestInit) => (url === BITBUCKET_URL && (init?.method ?? "GET") === "GET")
      ? Response.json({ error: "Bitbucket is unreachable" }, { status: 502 })
      : apiFetch(url, init));
    await render(createElement(ConnectionsSection, { page: settingsPage("admin") }));

    expect(body()).toContain("octo-org");
    const alerts = [...document.querySelectorAll<HTMLElement>("[role=alert]")];
    expect(alerts).toHaveLength(1);
    expect(alerts[0]?.textContent).toContain("Bitbucket");

    const before = fetcher.mock.calls.length;
    await click([...(alerts[0]?.querySelectorAll<HTMLButtonElement>("button") ?? [])].find((element) => element.textContent?.includes("Retry")));
    const after = fetcher.mock.calls.slice(before) as Array<[string, RequestInit | undefined]>;
    expect(after).toHaveLength(1);
    expect(after[0]?.[0]).toBe(BITBUCKET_URL);
    expect(after[0]?.[1]?.method ?? "GET").toBe("GET");
  });
});

describe("Post-create one-time reveal", () => {
  const REVEAL: SecretReveal = {
    title: "Webhook secret created",
    description: "Copy this value now. It is shown only once.",
    secretLabel: "Webhook secret",
    value: "whsec_fixture_9f2ce41a",
  };
  const created = { id: "rev-1", provider: "bitbucket", name: "Web platform", workspace: "acme-web", workspaceUuid: "{acme-web}" } as const;

  function revealPlatform(postCreate: PlatformDescriptor["postCreate"]) {
    return testPlatform("revealci", "RevealCI", {
      list: { kind: "fetch", load: vi.fn(async () => ({ canManage: true, items: [] })) },
      create: vi.fn(async () => created),
    }, { postCreate });
  }

  it("shows the hook's value once, ignores Escape, and drops it from the DOM after the close button", async () => {
    const descriptors = [github(), revealPlatform(() => REVEAL)];
    await render(createElement(ConnectionsSection, { descriptors, page: settingsPage("admin") }));
    await startAddConnection("RevealCI");
    await fillBitbucketFields("Web platform");

    const dialog = [...document.querySelectorAll("[role=dialog]")].find((element) => element.textContent?.includes(REVEAL.value));
    expect(dialog?.textContent).toContain(REVEAL.title);

    act(() => { (document.activeElement ?? document.body).dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true })); });
    await settle();
    expect(document.body.innerHTML).toContain(REVEAL.value);

    await click(buttons("I’ve stored it, close")[0]);
    expect(document.body.innerHTML).not.toContain(REVEAL.value);
  });

  it("shows no dialog when the hook returns null", async () => {
    const descriptors = [github(), revealPlatform(() => null)];
    await render(createElement(ConnectionsSection, { descriptors, page: settingsPage("admin") }));
    await startAddConnection("RevealCI");
    await fillBitbucketFields("Web platform");
    expect(document.querySelector("[role=dialog]")).toBeNull();
  });

  it.each([
    ["throws", () => { throw new Error("Webhook setup failed"); }],
    ["rejects", async () => { throw new Error("Webhook setup failed"); }],
  ] as const)("a hook that %s after the dialog closed surfaces as an error toast with its message, and opens no reveal dialog", async (_verb, hook) => {
    const descriptors = [github(), revealPlatform(hook)];
    await render(createElement(ConnectionsSection, { descriptors, page: settingsPage("admin") }));
    await startAddConnection("RevealCI");
    await fillBitbucketFields("Web platform");
    expect(toast.error).toHaveBeenCalledTimes(1);
    expect(toast.error.mock.calls[0]?.[0]).toEqual(expect.stringContaining("Webhook setup failed"));
    expect(document.querySelector("[role=dialog]")).toBeNull();
  });
});

describe("Add connection offers every platform through its descriptor", () => {
  const INSTALL_URL = "https://github.com/apps/gridops-self-hosted/installations/new";
  const dialog = () => {
    const found = document.querySelector<HTMLElement>("[role=dialog]");
    if (!found) throw new Error("No dialog is open");
    return found;
  };
  /** The platform choices of step 1, as their visible text, in the order they are listed. */
  const choiceElements = () => [...dialog().querySelectorAll<HTMLElement>("button, [role=radio], label")].filter((element) => {
    const text = element.textContent?.trim() ?? "";
    return text !== "" && !/^(Cancel|Continue|Back|Close)$/.test(text);
  });
  const choices = () => choiceElements().map((element) => element.textContent?.trim() ?? "");
  const listed = (name: string) => choices().findIndex((text) => text.startsWith(name));
  const choose = (name: string) => click(choiceElements().find((element) => element.textContent?.trim().startsWith(name)));

  /** A test-only platform with a page-derived list the user manages and no create binding. */
  function managedPlatform(id: string, name: string, connectLink?: PlatformDescriptor["connections"]["connectLink"]): PlatformDescriptor {
    const base = getPlatform(id);
    return {
      ...base,
      name,
      connections: {
        ...base.connections,
        list: { kind: "page", read: () => ({ canManage: true, items: [] }) },
        emptyText: () => null,
        chooserDescription: `${name} runners, connected on ${name}.`,
        ...(connectLink ? { connectLink } : {}),
      },
    };
  }

  it("lists GitHub then Bitbucket for an administrator, each with its chooser description", async () => {
    await render(createElement(ConnectionsSection, { page: settingsPage("admin") }));
    await click(buttons("Add connection")[0]);
    const github = listed("GitHub");
    const bitbucket = listed("Bitbucket");
    expect(github).toBeGreaterThanOrEqual(0);
    expect(bitbucket).toBeGreaterThan(github);
    expect(choices()[github]).toContain("Runners for GitHub Actions. Added by installing the GridOps GitHub App.");
    expect(choices()[bitbucket]).toContain("Bitbucket Cloud workspaces with managed macOS runners for Pipelines.");
  });

  it("GitHub's step links to the App's install URL in a new tab, and Back returns to step 1 from it and from a form", async () => {
    await render(createElement(ConnectionsSection, { page: settingsPage("admin") }));
    await startAddConnection("GitHub");
    const install = dialog().querySelector<HTMLAnchorElement>(`a[href="${INSTALL_URL}"]`);
    expect(install?.textContent).toContain("Install GitHub App");
    expect(install?.target).toBe("_blank");
    expect(install?.rel).toContain("noreferrer");
    expect(dialog().textContent).toContain("GitHub accounts are connected by installing the GridOps GitHub App on them.");

    await click(buttons("Back")[0]);
    expect(dialog().querySelector(`a[href="${INSTALL_URL}"]`)).toBeNull();
    expect(listed("GitHub")).toBeGreaterThanOrEqual(0);
    expect(listed("Bitbucket")).toBeGreaterThanOrEqual(0);

    await choose("Bitbucket");
    expect(() => field("Workspace slug")).not.toThrow();
    await click(buttons("Back")[0]);
    expect(() => field("Workspace slug")).toThrow();
    expect(listed("GitHub")).toBeGreaterThanOrEqual(0);
  });

  it("while the GitHub App is not configured, GitHub's install button is disabled and says why", async () => {
    await render(createElement(ConnectionsSection, { page: settingsPage("admin", { githubAppControl: false }, { githubApp: null }) }));
    await startAddConnection("GitHub");
    expect(dialog().textContent).toContain("Create the GitHub App first (below on this page).");
    const install = [...dialog().querySelectorAll<HTMLElement>("a, button")].find((element) => element.textContent?.includes("Install GitHub App"));
    expect(install).toBeTruthy();
    expect(install?.getAttribute("href")).toBeNull();
    expect(install instanceof HTMLButtonElement ? install.disabled : install?.getAttribute("aria-disabled") === "true").toBe(true);
  });

  it("lists any platform with a connectLink and opens its link, and lists none with neither binding (registry-driven)", async () => {
    const connectLink = () => ({ label: "Open LinkCI", description: "LinkCI connects from its own site.", href: "https://linkci.example/connect" });
    const descriptors = [bitbucket(), managedPlatform("linkci", "LinkCI", connectLink), managedPlatform("bareci", "BareCI")];
    await render(createElement(ConnectionsSection, { descriptors, page: settingsPage("admin") }));
    await click(buttons("Add connection")[0]);
    expect(listed("Bitbucket")).toBeGreaterThanOrEqual(0);
    expect(listed("LinkCI")).toBeGreaterThan(listed("Bitbucket"));
    expect(choices()[listed("LinkCI")]).toContain("LinkCI runners, connected on LinkCI.");
    expect(listed("BareCI")).toBe(-1);

    await choose("LinkCI");
    const link = dialog().querySelector<HTMLAnchorElement>('a[href="https://linkci.example/connect"]');
    expect(link?.textContent).toContain("Open LinkCI");
    expect(link?.target).toBe("_blank");
    expect(dialog().textContent).toContain("LinkCI connects from its own site.");
  });
});

describe("Connections empty states", () => {
  /** The settings page with no GitHub installations (App configured unless overridden). */
  const noInstallations = (role: "admin" | "member", data: Partial<PageData> = {}) => settingsPage(role, {}, { installations: [], ...data });
  const PER_PLATFORM_EMPTY = ["No installations are visible", "No Bitbucket workspaces are connected yet."];

  it("an administrator with nothing connected sees one generic empty state whose Add connection opens the chooser", async () => {
    bitbucketList = { canManage: true, items: [] };
    await render(createElement(ConnectionsSection, { page: noInstallations("admin") }));
    expect(body()).toContain("No connections yet");
    expect(body()).toContain("Connect a CI platform to start assigning it to runner pools.");
    expect(body()).not.toContain("A GridOps administrator can add platform connections.");
    for (const text of PER_PLATFORM_EMPTY) expect(body()).not.toContain(text);
    const add = buttons("Add connection");
    expect(add.length).toBeGreaterThan(0);
    await click(add.at(-1));
    expect(document.querySelector("[role=dialog]")?.textContent).toContain("Bitbucket");
  });

  it("a member with nothing connected sees the generic empty state with the member copy and no Add connection", async () => {
    bitbucketList = { canManage: false, items: [] };
    await render(createElement(ConnectionsSection, { page: noInstallations("member") }));
    expect(body()).toContain("No connections yet");
    expect(body()).toContain("A GridOps administrator can add platform connections.");
    expect(body()).not.toContain("Connect a CI platform to start assigning it to runner pools.");
    for (const text of PER_PLATFORM_EMPTY) expect(body()).not.toContain(text);
    expect(buttons("Add connection")).toHaveLength(0);
  });

  it("shows the GitHub empty row beside Bitbucket's rows only while the GitHub App is configured", async () => {
    await render(createElement(ConnectionsSection, { page: noInstallations("admin", { githubApp: null }) }));
    expect(body()).toContain("Mobile delivery");
    expect(body()).not.toContain("No installations are visible");
    expect(body()).not.toContain("No connections yet");

    act(() => root.unmount());
    root = createRoot(container);
    await render(createElement(ConnectionsSection, { page: noInstallations("admin") }));
    expect(body()).toContain("Mobile delivery");
    expect(body()).toContain("No installations are visible to @octocat.");
    expect(body()).not.toContain("No connections yet");
  });

  it("holds the generic empty state back while a list is loading, and shows it once every list has loaded empty", async () => {
    let finish: (list: ConnectionList) => void = () => undefined;
    const pending = new Promise<ConnectionList>((resolve) => { finish = resolve; });
    const descriptors = [github(), testPlatform("slowci", "SlowCI", { list: { kind: "fetch", load: () => pending } })];
    await render(createElement(ConnectionsSection, { descriptors, page: noInstallations("admin") }));
    expect(body()).not.toContain("No connections yet");

    await act(async () => { finish({ canManage: true, items: [] }); });
    await settle();
    expect(body()).toContain("No connections yet");
  });

  it("while the Bitbucket list has failed shows its error with Retry and the per-platform rows, not the generic empty state, until Retry loads it empty", async () => {
    let fail = true;
    bitbucketList = { canManage: true, items: [] };
    fetcher.mockImplementation(async (url: string, init?: RequestInit) => (fail && url === BITBUCKET_URL && (init?.method ?? "GET") === "GET")
      ? Response.json({ error: "Bitbucket is unreachable" }, { status: 502 })
      : apiFetch(url, init));
    await render(createElement(ConnectionsSection, { page: noInstallations("admin") }));
    const alerts = [...document.querySelectorAll<HTMLElement>("[role=alert]")];
    expect(alerts).toHaveLength(1);
    expect(alerts[0]?.textContent).toContain("Bitbucket");
    expect(body()).toContain("No installations are visible to @octocat.");
    expect(body()).not.toContain("No connections yet");

    fail = false;
    await click([...(alerts[0]?.querySelectorAll<HTMLButtonElement>("button") ?? [])].find((element) => element.textContent?.includes("Retry")));
    expect(document.querySelectorAll("[role=alert]")).toHaveLength(0);
    expect(body()).toContain("No connections yet");
  });
});

describe("GitHub App section (moved from the GitHub page)", () => {
  const props = (page: SettingsPage) => {
    if (!page.data) throw new Error("No page data");
    const { configuration, githubApp, user } = page.data;
    return { configuration, githubApp, user };
  };

  it("refreshes access, and links Install and (for administrators) Manage App", async () => {
    await render(createElement(GitHubAppSection, props(settingsPage("admin"))));
    await click(buttons("Refresh access")[0]);
    expect(calls("POST", "/api/v1/repositories/sync")).toHaveLength(1);
    expect(document.querySelector('a[href="https://github.com/apps/gridops-self-hosted/installations/new"]')?.textContent).toContain("Install");
    expect(document.querySelector('a[href="https://github.com/apps/gridops-self-hosted"]')?.textContent).toContain("Manage App");
  });

  it("lets an administrator start GitHub App setup while the App is not configured", async () => {
    const submit = vi.spyOn(HTMLFormElement.prototype, "submit").mockImplementation(() => undefined);
    await render(createElement(GitHubAppSection, props(settingsPage("admin", { githubAppControl: false }))));
    expect(body()).toContain("Create the GitHub App");
    await click(buttons("Create GitHub App")[0]);
    const posts = calls("POST", "/api/v1/github-app/manifest");
    expect(posts).toHaveLength(1);
    expect(JSON.parse(String(posts[0]?.[1]?.body))).toEqual({ ownerType: "user", name: "GridOps Self-Hosted" });
    expect(submit).toHaveBeenCalledTimes(1);
  });

  it("shows a member the checks and endpoints, with no Create section and no Manage App", async () => {
    await render(createElement(GitHubAppSection, props(settingsPage("member", { githubAppControl: false }))));
    expect(body()).toContain("GitHub OAuth");
    expect(body()).toContain("Webhook URL");
    expect(body()).not.toContain("Create the GitHub App");
    expect(document.querySelector('a[href="https://github.com/apps/gridops-self-hosted"]')).toBeNull();
    expect(document.querySelector('a[href="https://github.com/apps/gridops-self-hosted/installations/new"]')).not.toBeNull();
  });
});

describe("Connections route", () => {
  const page = () => (ConnectionsRoute as unknown as { options: { component: ComponentType } }).options.component;

  it.each([
    ["a member sees exactly one", "member", 1],
    ["an administrator sees no", "admin", 0],
  ] as const)("%s Read-only access callout", async (_label, role, count) => {
    bitbucketList = { canManage: role === "admin", items: [MOBILE] };
    loaderData.current = settingsPage(role);
    await render(createElement(page()));
    expect(body()).toContain("Mobile delivery");
    expect(body().match(/Read-only access/g) ?? []).toHaveLength(count);
  });
});
