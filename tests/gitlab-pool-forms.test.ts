// @vitest-environment jsdom
/**
 * GitLab on the runner pool forms (#141), driven through the rendered routes with the production
 * registry: GitLab projects and groups in the new pool's Targets picker, the `Run untagged jobs` switch in
 * Platform options (only while GitLab is targeted) reaching the create request, a GitLab pool's
 * settings offering every connection (fetched) and saving the full set, and a GitHub pool's
 * settings showing nothing of GitLab. Requests are read where they leave `api()`, after the
 * submit-time schema parse.
 */
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { act, createElement, type ComponentType } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { DOCS_SITE, GITLAB_URL, MOBILE_APP, PLATFORM_GROUP, gitlabCreateOptions, gitlabPool, gitlabPoolFormFetch } from "./fixtures/gitlab";
import { input, openPicker, optionIn, pick, platformOptionsToggle, settingsPool, settle, submitForm, type } from "./fixtures/pool-forms";

const loaderData = vi.hoisted(() => ({ current: undefined as unknown }));

vi.mock("@tanstack/react-router", () => ({
  createFileRoute: () => (options: Record<string, unknown>) => ({ options, useLoaderData: () => loaderData.current }),
  getRouteApi: () => ({ useLoaderData: () => loaderData.current }),
  Link: ({ children, to }: { children?: unknown; to?: string }) => createElement("a", { href: String(to) }, children as never),
  useNavigate: () => vi.fn(),
}));
vi.mock("sonner", () => ({ toast: { success: vi.fn(), error: vi.fn() } }));

import { TooltipProvider } from "../src/components/ui/tooltip";
import { platforms } from "../src/features/platform-connections/registry";
import { Route as NewPoolRoute } from "../src/routes/_app/runner-pools.new";
import { Route as PoolSettingsRoute } from "../src/routes/_app/runner-pools.$poolId.settings";

declare global {
  var IS_REACT_ACT_ENVIRONMENT: boolean | undefined;
}

class NoopResizeObserver { observe() {} unobserve() {} disconnect() {} }

let container: HTMLDivElement;
let root: Root;
let fetcher: ReturnType<typeof vi.fn>;

beforeEach(() => {
  globalThis.IS_REACT_ACT_ENVIRONMENT = true;
  vi.stubGlobal("ResizeObserver", NoopResizeObserver);
  fetcher = vi.fn(async (url: string, init?: RequestInit) => gitlabPoolFormFetch(url, init));
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

const requests = (method: string, url: string) => (fetcher.mock.calls as Array<[string, RequestInit | undefined]>)
  .filter(([callUrl, init]) => callUrl === url && (init?.method ?? "GET") === method)
  .map(([, init]) => (init?.body === undefined ? undefined : JSON.parse(String(init.body))) as Record<string, unknown> | undefined);

const optionTexts = () => [...openPicker("Targets").querySelectorAll("[role=option]")].map((element) => element.textContent ?? "");

/** The switch whose accessible name (aria-label, aria-labelledby, label[for] or wrapping label) says `name`. */
function switchNamed(name: string) {
  return [...document.querySelectorAll<HTMLElement>("[role=switch]")].find((element) => {
    const labelledBy = (element.getAttribute("aria-labelledby") ?? "").split(/\s+/).map((id) => (id ? document.getElementById(id)?.textContent ?? "" : ""));
    const forLabels = [...document.querySelectorAll<HTMLLabelElement>("label[for]")].filter((label) => element.id && label.htmlFor === element.id).map((label) => label.textContent ?? "");
    return [element.getAttribute("aria-label") ?? "", ...labelledBy, ...forLabels, element.closest("label")?.textContent ?? ""].some((text) => text.includes(name));
  }) ?? null;
}

function expandPlatformOptions() {
  const toggle = platformOptionsToggle();
  if (!toggle) throw new Error("No Platform options section");
  if (toggle.getAttribute("aria-expanded") !== "true") act(() => toggle.click());
}

describe("New pool with GitLab targets", () => {
  it("lists GitLab projects and groups as their own kinds, each option described as <baseUrl>/<scopePath>", async () => {
    await renderPage(NewPoolRoute, gitlabCreateOptions());
    const listbox = openPicker("Targets").textContent ?? "";
    expect(listbox).toContain("GitLab · Project");
    expect(listbox).toContain("GitLab · Group");
    expect(optionIn("Targets", "Mobile app").textContent).toContain("https://gitlab.com/acme/platform/mobile-app");
    expect(optionIn("Targets", "Platform group").textContent).toContain("https://git.example.com/platform");
    expect(requests("GET", GITLAB_URL)).toHaveLength(0);
  });

  it("shows Run untagged jobs (off) in Platform options only while GitLab is targeted, and creates a gitlab pool with the picked connections and the switch's value", async () => {
    await renderPage(NewPoolRoute, gitlabCreateOptions());
    await pick("Targets", "octo-org/app");
    if (platformOptionsToggle()) expandPlatformOptions();
    expect(switchNamed("Run untagged jobs")).toBeNull();
    await pick("Targets", "octo-org/app");

    await pick("Targets", "Mobile app");
    await pick("Targets", "Platform group");
    expect(platformOptionsToggle()?.getAttribute("aria-expanded")).toBe("false");
    expandPlatformOptions();
    const runUntagged = switchNamed("Run untagged jobs");
    expect(runUntagged, "a switch named Run untagged jobs").not.toBeNull();
    expect(runUntagged?.getAttribute("aria-checked")).toBe("false");
    act(() => runUntagged?.click());
    expect(switchNamed("Run untagged jobs")?.getAttribute("aria-checked")).toBe("true");

    type(input("#pool-name"), "gitlab-builds");
    await submitForm();
    const created = requests("POST", "/api/v1/runner-pools");
    expect(created).toHaveLength(1);
    expect(created[0]).toMatchObject({
      scope: "gitlab",
      installationId: 1,
      repositoryIds: [],
      runnerGroupId: 1,
      gitlabConnectionIds: [MOBILE_APP.id, PLATFORM_GROUP.id],
      runUntagged: true,
    });
  });
});

describe("Pool settings and GitLab", () => {
  it("a GitLab pool offers every GitLab connection (fetched) and nothing of GitHub or Bitbucket, starts the switch from the pool, and saves the full set and the switch", async () => {
    await renderPage(PoolSettingsRoute, gitlabPool({ runUntagged: true }));
    expect(requests("GET", GITLAB_URL).length).toBeGreaterThan(0);
    const options = optionTexts();
    expect(options.some((text) => text.includes("Docs site"))).toBe(true);
    expect(options.some((text) => /octo-org\/|acme\/web|Default|Release|Acme Bitbucket|Mobile delivery/.test(text))).toBe(false);
    // No locked GitHub runner-group chip (gap 4): it would read "octo-org · Runner group 1".
    expect(document.body.textContent).not.toContain("Runner group 1");

    await pick("Targets", "Docs site");
    expandPlatformOptions();
    expect(switchNamed("Run untagged jobs")?.getAttribute("aria-checked")).toBe("true");
    act(() => switchNamed("Run untagged jobs")?.click());
    await submitForm();
    const saved = requests("PUT", "/api/v1/runner-pools/pool-1");
    expect(saved).toHaveLength(1);
    expect([...((saved[0]?.gitlabConnectionIds as string[] | undefined) ?? [])].sort()).toEqual([MOBILE_APP.id, PLATFORM_GROUP.id, DOCS_SITE.id].sort());
    expect(saved[0]?.runUntagged).toBe(false);
    expect(saved[0]).not.toHaveProperty("repositoryIds");
  });

  it("a GitHub organization pool shows no GitLab group and fetches no GitLab connection", async () => {
    expect(platforms.map((platform) => platform.id), "GitLab is registered").toContain("gitlab");
    await renderPage(PoolSettingsRoute, settingsPool({ scope: "organization", installationId: 2, accountLogin: "acme", runnerGroupId: 1 }));
    expect(openPicker("Targets").textContent).not.toContain("GitLab");
    expect(requests("GET", GITLAB_URL)).toHaveLength(0);
  });
});
