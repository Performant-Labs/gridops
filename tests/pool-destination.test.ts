// @vitest-environment jsdom
/**
 * #159: the pool list's destination cell and the pool detail's Destination group come from the
 * platform registry, not from route code that assumes every pool belongs to a GitHub account.
 *
 * - The registry resolves the platform that owns a pool's scope (`platformForPool`): GitHub owns
 *   `repository` and `organization` (so a Bitbucket pool, which is GitHub-scoped, stays GitHub's),
 *   GitLab owns `gitlab`, and an unknown scope gets a registry fallback that never throws.
 * - GitHub and Bitbucket pools look exactly as they did before #159, in both views.
 * - A GitLab pool shows its GitLab connections and the GitLab icon, never the installation's
 *   GitHub account, avatar or runner group; with no connections it shows a neutral fallback.
 *
 * Both views are driven through the rendered routes with the production registry, so the tests
 * pin what the operator sees, whatever the descriptor hooks are named. Copy comes from the
 * approved wireframe (docs/handoffs/159/wireframe.html, frames A-E).
 */
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { act, createElement, type ComponentType } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { DOCS_SITE, MOBILE_APP, PLATFORM_GROUP, gitlabPool, poolConnection } from "./fixtures/gitlab";
import { BITBUCKET, REPOSITORIES, settingsPool, settle } from "./fixtures/pool-forms";

const loaderData = vi.hoisted(() => ({ current: undefined as unknown }));

vi.mock("@tanstack/react-router", () => ({
  createFileRoute: () => (options: Record<string, unknown>) => ({ options, useLoaderData: () => loaderData.current }),
  getRouteApi: () => ({ useLoaderData: () => loaderData.current }),
  Link: ({ children, to, ...props }: { children?: unknown; to?: string; "aria-label"?: string }) =>
    createElement("a", { href: String(to), "aria-label": props["aria-label"] }, children as never),
  useMatch: () => undefined,
  useNavigate: () => vi.fn(),
  useRouter: () => ({ invalidate: vi.fn(async () => undefined) }),
}));
vi.mock("sonner", () => ({ toast: { success: vi.fn(), error: vi.fn(), promise: vi.fn() } }));

import { TooltipProvider } from "../src/components/ui/tooltip";
import type { RunnerPool } from "../src/features/operations/operations.functions";
import * as registry from "../src/features/platform-connections/registry";
import type { PlatformDescriptor } from "../src/features/platform-connections/types";
import type { RunnerPoolDetail } from "../src/features/runner-pools/runner-pools.functions";
import { Route as PoolListRoute } from "../src/routes/_app/runner-pools.index";
import { Route as PoolDetailRoute } from "../src/routes/_app/runner-pools.$poolId.index";

declare global {
  var IS_REACT_ACT_ENVIRONMENT: boolean | undefined;
}

class NoopResizeObserver { observe() {} unobserve() {} disconnect() {} }

const AVATAR = (login: string) => `https://github.com/${login}.png?size=40`;

// ---- Fixtures ------------------------------------------------------------------------------------

/** A pool-list item (`RunnerPool`). `gitlabConnections` is the #159 wire field, absent from an older server. */
function listPool(overrides: Record<string, unknown>): RunnerPool {
  return {
    id: "pool-1", name: "linux-app", scope: "organization", mode: "ephemeral", provider: "docker", providers: ["docker"], labels: [],
    image: "runner:acme", dockerImage: "runner:acme", tartImage: "", macosRuntime: "vm",
    desiredCount: 1, minCount: 0, maxCount: 4, cpuLimit: 2, memoryLimitMb: 2048,
    paused: false, state: "active", accountLogin: "acme", repository: null, repositoryCount: 0,
    provisionFailureCount: 0, provisionRetryAt: null, provisionCircuitOpen: false, provisionWaitReason: null,
    accountCount: 1, totalRunners: 0, onlineRunners: 0, busyRunners: 0, failedRunners: 0,
    outdatedRunners: 0, createdAt: "2026-11-20T20:00:00.000Z", canManage: false, cannotRun: null,
    ...overrides,
  } as RunnerPool;
}

const GITHUB_REPO_ONE = listPool({ id: "p-repo-1", name: "web-builders", scope: "repository", accountLogin: "acme", repository: "acme/web", repositoryCount: 1 });
const GITHUB_REPO_MANY = listPool({ id: "p-repo-5", name: "mono-builders", scope: "repository", accountLogin: "acme", repository: "acme/web", repositoryCount: 5 });
const GITHUB_ORG = listPool({ id: "p-org", name: "org-builders", scope: "organization", accountLogin: "acme", repository: null });
/** A Bitbucket pool rides on a GitHub target: the list item is GitHub-scoped. */
const BITBUCKET_POOL = listPool({ id: "p-bb", name: "bb-builders", scope: "organization", accountLogin: "octo-org", repository: null });
const GITLAB_ONE = listPool({ id: "p-gl-1", name: "gl-builders", scope: "gitlab", accountLogin: "octo-org", gitlabConnections: [poolConnection(MOBILE_APP)] });
const GITLAB_TWO = listPool({ id: "p-gl-2", name: "gl-multi", scope: "gitlab", accountLogin: "octo-org", gitlabConnections: [poolConnection(MOBILE_APP), poolConnection(DOCS_SITE)] });
const GITLAB_NONE = listPool({ id: "p-gl-0", name: "gl-empty", scope: "gitlab", accountLogin: "octo-org", gitlabConnections: [] });
const GITLAB_OLD_SERVER = listPool({ id: "p-gl-old", name: "gl-old", scope: "gitlab", accountLogin: "octo-org" });
const UNKNOWN_SCOPE = listPool({ id: "p-unknown", name: "future-pool", scope: "azure-devops", accountLogin: "octo-org", repository: "octo-org/app" });

const NINE_REPOSITORIES = Array.from({ length: 9 }, (_, index) => ({
  id: 100 + index, installationId: 2, accountLogin: "acme", accountType: "Organization", fullName: `acme/repo-${index + 1}`, private: true,
}));

const detailPool = (overrides: Record<string, unknown>) => settingsPool(overrides) as unknown as RunnerPoolDetail;
const ACME_REPOS = REPOSITORIES.filter((repository) => repository.accountLogin === "acme");
const DETAIL_REPO_TWO = detailPool({ scope: "repository", accountLogin: "acme", installationId: 2, repositories: [...ACME_REPOS, { ...ACME_REPOS[0], id: 21, fullName: "acme/api" }], repositoryIds: [20, 21] });
const DETAIL_REPO_NINE = detailPool({ scope: "repository", accountLogin: "acme", installationId: 2, repositories: NINE_REPOSITORIES, repositoryIds: NINE_REPOSITORIES.map((repository) => repository.id) });
const DETAIL_ORG = detailPool({ scope: "organization", accountLogin: "acme", installationId: 2, runnerGroupId: 7 });
const DETAIL_BITBUCKET = detailPool({ scope: "organization", accountLogin: "acme", installationId: 2, runnerGroupId: 1, bitbucketConnections: BITBUCKET.slice(0, 2), bitbucketConnectionIds: BITBUCKET.slice(0, 2).map((connection) => connection.id) });
const DETAIL_GITLAB_ONE = gitlabPool({ gitlabConnections: [poolConnection(MOBILE_APP)], gitlabConnectionIds: [MOBILE_APP.id] }) as unknown as RunnerPoolDetail;
/** In the server's order, which is not alphabetical. */
const DETAIL_GITLAB_TWO = gitlabPool({ gitlabConnections: [poolConnection(PLATFORM_GROUP), poolConnection(MOBILE_APP)], gitlabConnectionIds: [PLATFORM_GROUP.id, MOBILE_APP.id] }) as unknown as RunnerPoolDetail;
const DETAIL_GITLAB_NONE = gitlabPool({ gitlabConnections: [], gitlabConnectionIds: [] }) as unknown as RunnerPoolDetail;
const DETAIL_GITLAB_OLD_SERVER = gitlabPool({ gitlabConnections: undefined, gitlabConnectionIds: undefined }) as unknown as RunnerPoolDetail;
const DETAIL_UNKNOWN = detailPool({ scope: "azure-devops", accountLogin: "octo-org", runnerGroupId: 1 });

// ---- Rendering -----------------------------------------------------------------------------------

let container: HTMLDivElement;
let root: Root;

beforeEach(() => {
  globalThis.IS_REACT_ACT_ENVIRONMENT = true;
  vi.stubGlobal("ResizeObserver", NoopResizeObserver);
  // The detail page loads its runners and its activity; both are empty here.
  vi.stubGlobal("fetch", vi.fn(async () => Response.json({ authenticated: true, items: [], total: 0, page: 1, perPage: 25 })));
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

/** The class lucide gives the GitLab descriptor's icon (e.g. `lucide-gitlab`), read from the descriptor, not assumed. */
function gitlabIconClass() {
  const scratch = document.createElement("div");
  const scratchRoot = createRoot(scratch);
  act(() => scratchRoot.render(createElement(registry.getPlatform("gitlab").icon)));
  const token = (scratch.querySelector("svg")?.getAttribute("class") ?? "").split(/\s+/).find((name) => name.startsWith("lucide-"));
  act(() => scratchRoot.unmount());
  if (!token) throw new Error("The GitLab descriptor's icon renders no lucide-<name> class");
  return token;
}

const hasIcon = (element: Element, iconClass: string) =>
  [...element.querySelectorAll("svg")].some((svg) => (svg.getAttribute("class") ?? "").split(/\s+/).includes(iconClass));

// ---- List row helpers ----------------------------------------------------------------------------

function listRow(name: string) {
  const link = container.querySelector(`a[aria-label="Open ${name}"]`);
  if (!link?.parentElement) throw new Error(`No list row for ${name}`);
  return link.parentElement;
}

/**
 * The row's destination cell: its one titled element that is not the capacity meter (whose title
 * reads "N busy · N online · N target") and not an icon or image.
 */
function destinationCell(name: string) {
  const cells = [...listRow(name).querySelectorAll<HTMLElement>("[title]")]
    .filter((element) => !/\bbusy · /.test(element.title) && !["svg", "img"].includes(element.tagName.toLowerCase()));
  expect(cells.map((cell) => cell.title), `${name} has one titled destination cell`).toHaveLength(1);
  return cells[0] as HTMLElement;
}

function listDestination(name: string) {
  const cell = destinationCell(name);
  const images = [...cell.querySelectorAll("img")];
  return {
    cell,
    text: (cell.textContent ?? "").trim(),
    title: cell.title,
    images: images.map((image) => image.getAttribute("src")),
    leadingFirst: leadsText(cell),
  };
}

/** Whether the cell's first avatar or icon comes before its first text, as the wireframe draws it. */
function leadsText(cell: Element) {
  const leading = cell.querySelector("img, svg");
  const walker = document.createTreeWalker(cell, NodeFilter.SHOW_TEXT);
  let text = walker.nextNode();
  while (text && !(text.textContent ?? "").trim()) text = walker.nextNode();
  return Boolean(leading && text && leading.compareDocumentPosition(text) & Node.DOCUMENT_POSITION_FOLLOWING);
}

// ---- Detail group helpers ------------------------------------------------------------------------

function destinationGroup() {
  const heading = [...container.querySelectorAll("h3")].find((element) => element.textContent === "Destination");
  const group = heading?.parentElement?.parentElement;
  if (!group) throw new Error("No Destination group");
  return group;
}

/** Text as the operator sees it: a `capitalize` span shows each word's first letter in upper case. */
function shown(element: Element) {
  const walker = document.createTreeWalker(element, NodeFilter.SHOW_TEXT);
  let text = "";
  for (let node = walker.nextNode(); node; node = walker.nextNode()) {
    const value = node.textContent ?? "";
    text += node.parentElement?.closest(".capitalize") ? value.replace(/\b\p{L}/gu, (letter) => letter.toUpperCase()) : value;
  }
  return text.trim();
}

/** Each property row of the Destination group as `label: value`, in order. */
const propertyRows = () => [...destinationGroup().querySelectorAll("dt")].map((term) => `${term.textContent}: ${shown(term.nextElementSibling ?? term)}`);
const groupText = () => destinationGroup().textContent ?? "";
const groupImages = () => [...destinationGroup().querySelectorAll("img")].map((image) => image.getAttribute("src"));

/** The repository chips under the Account row, in order. */
const repositoryChips = () => [...destinationGroup().querySelectorAll<HTMLElement>("div[title]")].map((chip) => `${chip.textContent} [${chip.title}]`);

// ---- Registry ------------------------------------------------------------------------------------

describe("the registry resolves the platform that owns a pool's scope", () => {
  const platformForPool = (registry as unknown as { platformForPool?: (pool: RunnerPool | RunnerPoolDetail) => PlatformDescriptor }).platformForPool;

  it("exports platformForPool beside getPlatform and platformForDeliveryId", () => {
    expect(typeof platformForPool).toBe("function");
  });

  it.each([
    ["a GitHub repository pool (one repository)", GITHUB_REPO_ONE, "github"],
    ["a GitHub repository pool (several repositories)", GITHUB_REPO_MANY, "github"],
    ["a GitHub organization pool", GITHUB_ORG, "github"],
    // Bitbucket's descriptor owns no scope: its pools ride on a GitHub target.
    ["a Bitbucket pool (GitHub-scoped, list item)", BITBUCKET_POOL, "github"],
    ["a Bitbucket pool (GitHub-scoped, detail with workspaces)", DETAIL_BITBUCKET, "github"],
    ["a GitLab pool with one connection", GITLAB_ONE, "gitlab"],
    ["a GitLab pool with two connections (detail)", DETAIL_GITLAB_TWO, "gitlab"],
    ["a GitLab pool with no connections", GITLAB_NONE, "gitlab"],
  ] as const)("resolves %s to its owner", (_case, pool, owner) => {
    expect(platformForPool?.(pool)).toBe(registry.getPlatform(owner));
  });

  it("gives an unknown scope a registry fallback that never throws and is no registered platform", () => {
    const resolve = () => platformForPool?.(UNKNOWN_SCOPE);
    expect(resolve).not.toThrow();
    const fallback = resolve();
    expect(fallback).toBeTruthy();
    expect(registry.platforms).not.toContain(fallback);
  });
});

// ---- Pool list -----------------------------------------------------------------------------------

describe("pool list destination cell", () => {
  const LIST = [GITHUB_REPO_ONE, GITHUB_REPO_MANY, GITHUB_ORG, BITBUCKET_POOL, GITLAB_ONE, GITLAB_TWO, GITLAB_NONE, GITLAB_OLD_SERVER, UNKNOWN_SCOPE];
  const renderList = (items = LIST) => renderPage(PoolListRoute, { authenticated: true, items, total: items.length, page: 1, perPage: 25 });

  it.each([
    ["a GitHub repository pool with one repository: the repository full name", GITHUB_REPO_ONE, "acme/web", "acme"],
    ["a GitHub repository pool with several repositories: N repositories", GITHUB_REPO_MANY, "5 repositories", "acme"],
    ["a GitHub organization pool: the account login", GITHUB_ORG, "acme", "acme"],
    ["a Bitbucket pool (GitHub-scoped): the GitHub account login", BITBUCKET_POOL, "octo-org", "octo-org"],
  ] as const)("is unchanged for %s, with the GitHub avatar first", async (_case, pool, text, login) => {
    await renderList();
    const destination = listDestination(pool.name);
    expect(destination.text).toBe(text);
    expect(destination.title).toBe(text);
    expect(destination.images).toEqual([AVATAR(login)]);
    expect(destination.leadingFirst).toBe(true);
    const avatar = destination.cell.querySelector("img");
    expect([avatar?.getAttribute("width"), avatar?.getAttribute("height")]).toEqual(["16", "16"]);
  });

  it("shows a GitLab pool's one connection by name with the GitLab icon, never the installation's GitHub account", async () => {
    await renderList();
    const destination = listDestination(GITLAB_ONE.name);
    expect(destination.text).toBe("Mobile app");
    expect(destination.title).toBe("Mobile app");
    expect(destination.images).toEqual([]);
    expect(hasIcon(destination.cell, gitlabIconClass())).toBe(true);
    expect(destination.leadingFirst).toBe(true);
    expect(listRow(GITLAB_ONE.name).textContent).not.toContain("octo-org");
  });

  it("counts two or more GitLab connections in the cell and names them all in its title", async () => {
    await renderList();
    const destination = listDestination(GITLAB_TWO.name);
    expect(destination.text).toBe("2 connections");
    expect(destination.title).toBe("Mobile app, Docs site");
    expect(destination.images).toEqual([]);
    expect(hasIcon(destination.cell, gitlabIconClass())).toBe(true);
  });

  it.each([
    ["an empty connection list", GITLAB_NONE],
    ["no gitlabConnections field (an older server)", GITLAB_OLD_SERVER],
  ] as const)("shows a neutral GitLab destination for a GitLab pool with %s", async (_case, pool) => {
    await renderList();
    const destination = listDestination(pool.name);
    expect(destination.text).toBe("GitLab");
    expect(destination.title).toBe("GitLab");
    expect(destination.images).toEqual([]);
    expect(hasIcon(destination.cell, gitlabIconClass())).toBe(true);
    expect(listRow(pool.name).textContent).not.toContain("octo-org");
  });

  it("uses a nameless connection's baseUrl/scopePath in place of its name (partial data)", async () => {
    const nameless = listPool({ id: "p-gl-nameless", name: "gl-nameless", scope: "gitlab", accountLogin: "octo-org", gitlabConnections: [{ ...poolConnection(MOBILE_APP), name: "" }] });
    await renderList([nameless]);
    const destination = listDestination(nameless.name);
    expect(destination.text).toBe("https://gitlab.com/acme/platform/mobile-app");
    expect(destination.images).toEqual([]);
  });

  it("shows a dash, no avatar and no platform icon for a scope no descriptor owns, and does not throw", async () => {
    await renderList();
    const destination = listDestination(UNKNOWN_SCOPE.name);
    expect(destination.text).toBe("—");
    expect(destination.title).toBe("Unknown destination");
    expect(destination.images).toEqual([]);
    expect(hasIcon(destination.cell, gitlabIconClass())).toBe(false);
    expect(listRow(UNKNOWN_SCOPE.name).textContent).not.toMatch(/octo-org|azure-devops/);
  });
});

// ---- Pool detail ---------------------------------------------------------------------------------

describe("pool detail Destination group", () => {
  it("is unchanged for a GitHub repository pool: Scope, Account with the GitHub avatar, then the repositories", async () => {
    await renderPage(PoolDetailRoute, DETAIL_REPO_TWO);
    expect(propertyRows()).toEqual(["Scope: Repository", "Account: acme"]);
    expect(groupImages()).toEqual([AVATAR("acme")]);
    expect(repositoryChips()).toEqual(["acme/web [acme/web]", "acme/api [acme/api]"]);
    expect(destinationGroup().querySelector("button")).toBeNull();
  });

  it("keeps the six-repository list and its Show N more / Show fewer toggle for a GitHub repository pool", async () => {
    await renderPage(PoolDetailRoute, DETAIL_REPO_NINE);
    expect(propertyRows()).toEqual(["Scope: Repository", "Account: acme"]);
    expect(repositoryChips().map((chip) => chip.split(" ")[0])).toEqual(NINE_REPOSITORIES.slice(0, 6).map((repository) => repository.fullName));
    const toggle = destinationGroup().querySelector("button");
    expect(toggle?.textContent).toBe("Show 3 more");
    expect(toggle?.getAttribute("aria-expanded")).toBe("false");
    await act(async () => toggle?.click());
    expect(repositoryChips()).toHaveLength(9);
    expect(toggle?.textContent).toBe("Show fewer");
    expect(toggle?.getAttribute("aria-expanded")).toBe("true");
  });

  it.each([
    ["a GitHub organization pool", DETAIL_ORG, "#7"],
    ["a Bitbucket pool (GitHub-scoped, with workspaces)", DETAIL_BITBUCKET, "#1"],
  ] as const)("is unchanged for %s: Scope, Account with the GitHub avatar, Runner group", async (_case, pool, group) => {
    await renderPage(PoolDetailRoute, pool);
    expect(propertyRows()).toEqual(["Scope: Organization", "Account: acme", `Runner group: ${group}`]);
    expect(groupImages()).toEqual([AVATAR("acme")]);
    expect(repositoryChips()).toEqual([]);
  });

  it("shows Scope GitLab and the one connection (name, then baseUrl/scopePath), with no Account or Runner group row", async () => {
    await renderPage(PoolDetailRoute, DETAIL_GITLAB_ONE);
    expect(propertyRows()).toEqual(["Scope: GitLab"]);
    expect(groupText()).toContain("Mobile app");
    expect(groupText()).toContain("https://gitlab.com/acme/platform/mobile-app");
    expect(groupImages()).toEqual([]);
    expect(groupText()).not.toMatch(/octo-org|Runner group|#1/);
  });

  it("lists every GitLab connection in the server's order, each name before its own baseUrl/scopePath", async () => {
    await renderPage(PoolDetailRoute, DETAIL_GITLAB_TWO);
    expect(propertyRows()).toEqual(["Scope: GitLab"]);
    const text = groupText();
    const at = (value: string) => text.indexOf(value);
    for (const value of ["Platform group", "https://git.example.com/platform", "Mobile app", "https://gitlab.com/acme/platform/mobile-app"]) expect(text).toContain(value);
    expect(at("Platform group")).toBeLessThan(at("https://git.example.com/platform"));
    expect(at("https://git.example.com/platform")).toBeLessThan(at("Mobile app"));
    expect(at("Mobile app")).toBeLessThan(at("https://gitlab.com/acme/platform/mobile-app"));
    expect(destinationGroup().querySelector("button")).toBeNull();
    expect(groupImages()).toEqual([]);
    expect(text).not.toMatch(/octo-org|Runner group/);
  });

  it.each([
    ["an empty connection list", DETAIL_GITLAB_NONE],
    ["no gitlabConnections field (an older server)", DETAIL_GITLAB_OLD_SERVER],
  ] as const)("shows Scope GitLab and No connections listed for a GitLab pool with %s, never the GitHub account", async (_case, pool) => {
    await renderPage(PoolDetailRoute, pool);
    expect(propertyRows()).toEqual(["Scope: GitLab"]);
    expect(groupText()).toContain("No connections listed");
    expect(groupImages()).toEqual([]);
    expect(groupText()).not.toMatch(/octo-org|Runner group/);
  });

  it("shows only the parts of a connection's address that are present, and a nameless connection by its address (partial data)", async () => {
    const partial = gitlabPool({
      gitlabConnections: [{ ...poolConnection(MOBILE_APP), scopePath: "" }, { ...poolConnection(PLATFORM_GROUP), name: "" }],
    }) as unknown as RunnerPoolDetail;
    await renderPage(PoolDetailRoute, partial);
    const text = groupText();
    expect(text).toContain("Mobile app");
    expect(text).toContain("https://gitlab.com");
    expect(text).not.toContain("https://gitlab.com/");
    expect(text.split("https://git.example.com/platform")).toHaveLength(2);
  });

  it("shows only Scope Unknown for a scope no descriptor owns: no account, avatar or runner group, and no raw scope", async () => {
    await renderPage(PoolDetailRoute, DETAIL_UNKNOWN);
    expect(propertyRows()).toEqual(["Scope: Unknown"]);
    expect(groupImages()).toEqual([]);
    expect(groupText()).not.toMatch(/octo-org|Runner group|azure-devops/i);
  });
});
