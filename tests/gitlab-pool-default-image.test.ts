// @vitest-environment jsdom
/**
 * #172: a GitLab Docker pool may leave its Docker image empty, and then runs the GitLab runner image
 * (`GITLAB_RUNNER_IMAGE` in crates/gridops-core/src/provisioning.rs). The platform descriptor owns
 * that default (`defaultDockerImage`); the pool forms, the read-only views and the Fleet execution
 * editor read it from the descriptor, never from a platform comparison.
 *
 * Driven through the rendered routes with the production registry. Requests are read where they
 * leave `api()`, after the submit-time schema parse. Copy is the approved wireframe's
 * (docs/handoffs/172/wireframe.html, frames 1a-1d, 2a, 2c-2e, 3a-3d).
 */
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { act, createElement, type ComponentType } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { MOBILE_APP, gitlabCreateOptions, gitlabPool, gitlabPoolFormFetch } from "./fixtures/gitlab";
import { buttonByText, input, pick, settingsPool, settle, submitForm, type } from "./fixtures/pool-forms";

const loaderData = vi.hoisted(() => ({ current: undefined as unknown }));

vi.mock("@tanstack/react-router", () => ({
  createFileRoute: () => (options: Record<string, unknown>) => ({ options, useLoaderData: () => loaderData.current }),
  getRouteApi: () => ({ useLoaderData: () => loaderData.current }),
  Link: ({ children, to }: { children?: unknown; to?: string }) => createElement("a", { href: String(to) }, children as never),
  useMatch: () => undefined,
  useNavigate: () => vi.fn(),
  useRouter: () => ({ invalidate: vi.fn(async () => undefined) }),
}));
vi.mock("sonner", () => ({ toast: { success: vi.fn(), error: vi.fn(), promise: vi.fn() } }));

import { TooltipProvider } from "../src/components/ui/tooltip";
import { getPlatform, platforms } from "../src/features/platform-connections/registry";
import type { PlatformDescriptor, ResolvedSelection, TargetOption } from "../src/features/platform-connections/types";
import { defaultDockerImage } from "../src/features/runner-pools/targets";
import { Route as PoolDetailRoute } from "../src/routes/_app/runner-pools.$poolId.index";
import { Route as PoolSettingsRoute } from "../src/routes/_app/runner-pools.$poolId.settings";
import { Route as NewPoolRoute } from "../src/routes/_app/runner-pools.new";

declare global {
  var IS_REACT_ACT_ENVIRONMENT: boolean | undefined;
}

class NoopResizeObserver { observe() {} unobserve() {} disconnect() {} }

/** The GitLab runner image, as the server pins it. */
const GITLAB_IMAGE = "gitlab/gitlab-runner:ubuntu-v19.6.0";
const DEFAULT_LABEL = `Default (${GITLAB_IMAGE})`;
const GITHUB_HELP = "OCI image for each Linux runner.";
const GITLAB_HELP = `Leave empty to use ${GITLAB_IMAGE}.`;
/** `createOptions().defaults.dockerImage`: the control plane's GitHub runner image. */
const GITHUB_DEFAULT = "runner:default";
const GITHUB_REPOSITORY = "octo-org/app";

let container: HTMLDivElement;
let root: Root;
let fetcher: ReturnType<typeof vi.fn>;

/** `gitlabPoolFormFetch` plus an unsaved fleet profile, and empty runner and activity lists for the pool detail. */
function fetchStub(url: string, init?: RequestInit) {
  const method = init?.method ?? "GET";
  if (method === "GET" && /^\/api\/v1\/fleet\/pools\/[^/]+\/profile$/.test(url)) return Response.json({ item: null });
  if (method === "GET" && (url.startsWith("/api/v1/runners?") || /\/events\?/.test(url))) {
    return Response.json({ authenticated: true, items: [], total: 0, page: 1, perPage: 25 });
  }
  return gitlabPoolFormFetch(url, init);
}

beforeEach(() => {
  globalThis.IS_REACT_ACT_ENVIRONMENT = true;
  vi.stubGlobal("ResizeObserver", NoopResizeObserver);
  fetcher = vi.fn(async (url: string, init?: RequestInit) => fetchStub(url, init));
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

const dockerImageInput = () => input("#docker-image");

/** The Docker image row's help text (the SettingsRow description beside its label). */
function dockerImageHelp() {
  const label = document.querySelector<HTMLLabelElement>('label[for="docker-image"]');
  if (!label?.parentElement) throw new Error("No Docker image row");
  return (label.parentElement.textContent ?? "").replace(label.textContent ?? "", "").trim();
}

/** A read-only settings value by its label. */
function settingsValue(label: string) {
  const name = [...document.querySelectorAll("span")].find((element) => element.textContent === label);
  if (!name?.nextElementSibling) throw new Error(`No "${label}" value`);
  return name.nextElementSibling.textContent ?? "";
}

/** A pool detail property row by its label: its text and its hover title. */
function propertyRow(label: string) {
  const term = [...document.querySelectorAll("dt")].find((element) => element.textContent === label);
  const value = term?.nextElementSibling;
  if (!value) throw new Error(`No "${label}" property`);
  return { text: value.textContent ?? "", title: value.getAttribute("title") };
}

/** The image a Fleet execution platform starts from. */
function fleetPlatformImage(name: string) {
  return input(`input[aria-label="Image for ${name}"]`).value;
}

async function addFleetPlatform() {
  const add = buttonByText("Add platform");
  if (!add) throw new Error("No Add platform button");
  act(() => add.click());
  await settle();
}

/** A selected Targets option of `kindId` (the helpers under test read only its kind). */
function targetOption(kindId: string, nativeId: string, label: string, group: string) {
  return { kindId, connectionId: nativeId, nativeId, label, group } as TargetOption;
}

describe("the GitLab descriptor owns the default Docker image", () => {
  it("is the server's GITLAB_RUNNER_IMAGE, so the frontend's copy cannot drift", () => {
    // Vitest runs from the repository root.
    const source = readFileSync(join(process.cwd(), "crates/gridops-core/src/provisioning.rs"), "utf8");
    const pinned = /pub const GITLAB_RUNNER_IMAGE: &str = "([^"]+)";/.exec(source)?.[1];
    expect(pinned, "GITLAB_RUNNER_IMAGE in provisioning.rs").toBe(GITLAB_IMAGE);
    expect(getPlatform("gitlab").defaultDockerImage).toBe(pinned);
  });

  it("is resolved from a new pool's selection by a pure helper over the descriptors it is given", () => {
    const gitlabProject = targetOption("gitlab.project", MOBILE_APP.id, "Mobile app", "GitLab · Project");
    const githubRepository = targetOption("github.repository", "10", GITHUB_REPOSITORY, "GitHub · Repository");
    expect(defaultDockerImage(platforms, {})).toBeFalsy();
    expect(defaultDockerImage(platforms, { "github.repository": [githubRepository] })).toBeFalsy();
    expect(defaultDockerImage(platforms, { "gitlab.project": [gitlabProject] })).toBe(GITLAB_IMAGE);

    // A fixture registry: the helper reads the descriptors argument, not the production registry.
    const fixture = { ...getPlatform("github"), id: "fixture", targetKinds: [{ id: "fixture.kind" }], defaultDockerImage: "example.test/runner:1" } as unknown as PlatformDescriptor;
    const selection: ResolvedSelection = { "fixture.kind": [targetOption("fixture.kind", "1", "Fixture target", "Fixture")] };
    expect(defaultDockerImage([fixture], selection)).toBe("example.test/runner:1");
  });
});

describe("New pool form: the Docker image row follows the selected platform", () => {
  it("is required and prefilled with the GitHub default before any target is chosen (unchanged)", async () => {
    await renderPage(NewPoolRoute, gitlabCreateOptions());
    expect(dockerImageInput().required).toBe(true);
    expect(dockerImageInput().value).toBe(GITHUB_DEFAULT);
    expect(dockerImageHelp()).toBe(GITHUB_HELP);
  });

  it("is optional, empty and names the GitLab default for a GitLab target, and an empty image is sent empty", async () => {
    await renderPage(NewPoolRoute, gitlabCreateOptions());
    await pick("Targets", "Mobile app");
    expect(dockerImageInput().required, "optional").toBe(false);
    expect(dockerImageInput().value, "not prefilled with the GitHub default").toBe("");
    expect(dockerImageInput().placeholder).toBe(GITLAB_IMAGE);
    expect(dockerImageHelp()).toContain(GITLAB_HELP);

    type(input("#pool-name"), "gitlab-builds");
    await submitForm();
    const created = requests("POST", "/api/v1/runner-pools");
    expect(created, document.body.textContent ?? "").toHaveLength(1);
    expect(created[0]).toMatchObject({ scope: "gitlab", image: "", dockerImage: "" });
  });

  it("switching back to GitHub restores today's row, and a typed image survives switching either way", async () => {
    await renderPage(NewPoolRoute, gitlabCreateOptions());
    await pick("Targets", "Mobile app");
    expect(dockerImageInput().value).toBe("");
    await pick("Targets", "Mobile app");
    await pick("Targets", GITHUB_REPOSITORY);
    expect(dockerImageInput().required, "GitHub: required again").toBe(true);
    expect(dockerImageInput().value, "GitHub: prefilled again").toBe(GITHUB_DEFAULT);
    expect(dockerImageHelp()).toBe(GITHUB_HELP);

    // GitHub to GitLab keeps an image the user typed.
    type(dockerImageInput(), "ghcr.io/acme/runner:2");
    await pick("Targets", GITHUB_REPOSITORY);
    await pick("Targets", "Mobile app");
    expect(dockerImageInput().value).toBe("ghcr.io/acme/runner:2");

    // GitLab to GitHub keeps one too.
    type(dockerImageInput(), "registry.example.com/ci/gitlab-runner:1.2");
    await pick("Targets", "Mobile app");
    await pick("Targets", GITHUB_REPOSITORY);
    expect(dockerImageInput().value).toBe("registry.example.com/ci/gitlab-runner:1.2");
  });
});

describe("Pool settings: a GitLab pool's Docker image may stay empty", () => {
  const emptyGitlabPool = (overrides: Record<string, unknown> = {}) => gitlabPool({ image: "", dockerImage: "", ...overrides });

  it("is optional with the default named, saves empty, and never sends the optional flag in the body", async () => {
    await renderPage(PoolSettingsRoute, emptyGitlabPool());
    expect(dockerImageInput().required, "optional").toBe(false);
    expect(dockerImageInput().value).toBe("");
    expect(dockerImageHelp()).toContain(GITLAB_HELP);

    await submitForm();
    const saved = requests("PUT", "/api/v1/runner-pools/pool-1");
    expect(saved, document.body.textContent ?? "").toHaveLength(1);
    expect(saved[0]).toMatchObject({ image: "", dockerImage: "" });
    expect(saved[0]).not.toHaveProperty("dockerImageOptional");
  });

  it("a GitHub pool's row is unchanged: required, today's help, and no optional flag in the body", async () => {
    await renderPage(PoolSettingsRoute, settingsPool({ scope: "organization", installationId: 2, accountLogin: "acme", runnerGroupId: 1 }));
    expect(dockerImageInput().required).toBe(true);
    expect(dockerImageInput().value).toBe("runner:acme");
    expect(dockerImageHelp()).toBe(GITHUB_HELP);
    await submitForm();
    const saved = requests("PUT", "/api/v1/runner-pools/pool-1");
    expect(saved).toHaveLength(1);
    expect(saved[0]).toMatchObject({ dockerImage: "runner:acme" });
    expect(saved[0]).not.toHaveProperty("dockerImageOptional");
  });

  it("the read-only view shows the GitLab default for an empty image, and any stored image verbatim", async () => {
    await renderPage(PoolSettingsRoute, emptyGitlabPool({ canManage: false }));
    expect(settingsValue("Docker image")).toBe(DEFAULT_LABEL);

    act(() => root.unmount());
    root = createRoot(container);
    await renderPage(PoolSettingsRoute, gitlabPool({ canManage: false, image: "registry.example.com/ci/gitlab-runner:1.2", dockerImage: "registry.example.com/ci/gitlab-runner:1.2" }));
    expect(settingsValue("Docker image")).toBe("registry.example.com/ci/gitlab-runner:1.2");

    act(() => root.unmount());
    root = createRoot(container);
    await renderPage(PoolSettingsRoute, settingsPool({ canManage: false }));
    expect(settingsValue("Docker image")).toBe("runner:acme");
  });
});

describe("Pool detail: the Image property", () => {
  it("shows the GitLab default for a GitLab pool with an empty image, as text and title", async () => {
    await renderPage(PoolDetailRoute, gitlabPool({ image: "", dockerImage: "" }));
    expect(propertyRow("Image")).toEqual({ text: DEFAULT_LABEL, title: DEFAULT_LABEL });
  });

  it("shows a stored image verbatim (unchanged)", async () => {
    await renderPage(PoolDetailRoute, settingsPool());
    expect(propertyRow("Image")).toEqual({ text: "runner:acme", title: "runner:acme" });
  });
});

describe("Fleet execution: a new Docker platform's starting image", () => {
  it("is the raw GitLab runner image for a GitLab pool with an empty image, with no 'not the pool's image' hint", async () => {
    await renderPage(PoolSettingsRoute, gitlabPool({ image: "", dockerImage: "" }));
    await addFleetPlatform();
    expect(fleetPlatformImage("Linux x64 · Docker")).toBe(GITLAB_IMAGE);
    expect(document.body.textContent).not.toContain("Not the pool’s Docker image");
  });

  it("is the pool's own image for a GitHub pool (unchanged)", async () => {
    await renderPage(PoolSettingsRoute, settingsPool());
    await addFleetPlatform();
    expect(fleetPlatformImage("Linux x64 · Docker")).toBe("runner:acme");
  });
});
