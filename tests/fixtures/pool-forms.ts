/**
 * Fixtures and DOM helpers for the runner pool form tests of #145 (one Targets row for every CI
 * platform). The fixtures match the frozen-payload capture in tests/pool-target-payloads.test.ts.
 *
 * DOM contract the helpers rely on (see docs/handoffs/145/handoff-T-red.md):
 * - the Targets picker is the existing SearchableMultiSelect: a trigger button with
 *   aria-label "Targets" and aria-controls pointing at its listbox, options with role=option;
 * - a disabled option has the `disabled` attribute or aria-disabled="true".
 */
import { act } from "react";

export const INSTALLATIONS = [
  { id: 1, accountLogin: "octo-org", accountType: "Organization" },
  { id: 2, accountLogin: "acme", accountType: "Organization" },
  { id: 3, accountLogin: "jdoe", accountType: "User" },
];

/** In loaded-list order: installation 1's repositories come first. */
export const REPOSITORIES = [
  { id: 10, installationId: 1, accountLogin: "octo-org", accountType: "Organization", fullName: "octo-org/app", private: true },
  { id: 11, installationId: 1, accountLogin: "octo-org", accountType: "Organization", fullName: "octo-org/api", private: false },
  { id: 20, installationId: 2, accountLogin: "acme", accountType: "Organization", fullName: "acme/web", private: true },
];

export const RUNNER_GROUPS: Record<number, Array<{ id: number; name: string; visibility: string; isDefault: boolean }>> = {
  1: [{ id: 1, name: "Default", visibility: "all", isDefault: true }, { id: 4, name: "Linux builds", visibility: "selected", isDefault: false }],
  2: [{ id: 1, name: "Default", visibility: "all", isDefault: true }, { id: 7, name: "Release", visibility: "private", isDefault: false }],
};

export const BITBUCKET = [
  { id: "6a1c1b2e-0000-4000-8000-000000000001", name: "Acme Bitbucket", workspace: "acme", workspaceUuid: "{acme}" },
  { id: "6a1c1b2e-0000-4000-8000-000000000002", name: "Mobile delivery", workspace: "acme-mobile", workspaceUuid: "{acme-mobile}" },
  { id: "6a1c1b2e-0000-4000-8000-000000000003", name: "Web platform", workspace: "acme-web", workspaceUuid: "{acme-web}" },
];

export const BITBUCKET_SCRIPT = "scripts/prepare-tart-bitbucket-runner-image.sh";

/** The new-pool route loader's `getCreateRunnerPoolOptions()` result. */
export function createOptions(overrides: {
  installations?: typeof INSTALLATIONS;
  bitbucketConnections?: typeof BITBUCKET;
  defaults?: Record<string, unknown>;
} = {}) {
  return {
    authenticated: true,
    installUrl: "https://github.com/apps/gridops/installations/new",
    builtinRunnerHostEnabled: true,
    installations: overrides.installations ?? INSTALLATIONS,
    repositories: REPOSITORIES.map(({ id, installationId, fullName, private: isPrivate }) => ({ id, installationId, fullName, private: isPrivate })),
    runnerGroups: Object.entries(RUNNER_GROUPS).flatMap(([installationId, groups]) => groups.map((group) => ({ installationId: Number(installationId), ...group }))),
    bitbucketConnections: overrides.bitbucketConnections ?? BITBUCKET.slice(0, 2),
    defaults: {
      provider: "docker", providers: ["docker"], image: "runner:default", dockerImage: "runner:default", tartImage: "fixture-default-tahoe",
      macosRuntime: "vm", labels: ["gridops"], cpuLimit: 2, memoryLimitMb: 2048, desiredCount: 0, minCount: 0, maxCount: 10,
      autoscalingEnabled: true, queueScaleFactor: 1, idleTimeoutMinutes: 5, runnerGroupId: 1, maxCpuLimit: 16, maxMemoryLimitMb: 65_536,
      ...overrides.defaults,
    },
  };
}

/** A `RunnerPoolDetail` for the pool settings form. */
export function settingsPool(overrides: Record<string, unknown> = {}) {
  return {
    id: "pool-1", installationId: 1, repositoryId: null, repository: null, repositoryIds: [], repositories: [],
    priorityRepositoryIds: [], priorityWaitMinutes: 10, bitbucketConnections: [], bitbucketConnectionIds: [],
    accountLogin: "octo-org", name: "linux-app", scope: "organization", mode: "ephemeral", provider: "docker",
    providers: ["docker"], labels: ["build"], image: "runner:acme", dockerImage: "runner:acme", tartImage: "",
    macosRuntime: "vm", desiredCount: 1, minCount: 0, maxCount: 4, cpuLimit: 4, memoryLimitMb: 8192,
    maxCpuLimit: 16, maxMemoryLimitMb: 65_536, builtinRunnerHostEnabled: true, runnerGroupId: 1, paused: false,
    state: "active", autoscalingEnabled: true, queueScaleFactor: 1, idleTimeoutMinutes: 5, configurationVersion: 3,
    provisionFailureCount: 0, provisionRetryAt: null, provisionCircuitOpen: false, canManage: true,
    ...overrides,
  };
}

/** Serves the GitHub lists the Targets row loads, whichever of today's endpoints it uses. */
export function poolFormFetch(url: string, init?: RequestInit): Response | Promise<Response> {
  const method = init?.method ?? "GET";
  if (method === "GET" && url === "/api/v1/runner-pools/repositories") return Response.json({ items: REPOSITORIES });
  const repos = /^\/api\/v1\/installations\/(\d+)\/repositories$/.exec(url);
  if (method === "GET" && repos) return Response.json({ items: REPOSITORIES.filter((repository) => repository.installationId === Number(repos[1])) });
  const groups = /^\/api\/v1\/installations\/(\d+)\/runner-groups$/.exec(url);
  if (method === "GET" && groups) {
    const items = RUNNER_GROUPS[Number(groups[1])];
    return items ? Response.json({ items }) : Response.json({ error: "Runner groups are listed for organizations only." }, { status: 404 });
  }
  if (method === "GET" && url === "/api/v1/platform-connections/bitbucket") return Response.json({ canManage: true, items: [] });
  if (method === "GET" && url === "/api/v1/platform-connections/gitlab") return Response.json({ canManage: true, items: [] });
  if (method === "POST" && url === "/api/v1/runner-pools") return Response.json({ id: "pool-new" }, { status: 201 });
  if (method === "PUT" && url === "/api/v1/runner-pools/pool-1") return Response.json({ ok: true, configurationVersion: 4, rollingReplacement: false });
  return Response.json({ error: `not in this test: ${method} ${url}` }, { status: 404 });
}

export async function settle() {
  for (let index = 0; index < 6; index += 1) await act(async () => { await new Promise((resolve) => setTimeout(resolve, 0)); });
}

export function trigger(name: string) {
  const found = document.querySelector<HTMLButtonElement>(`button[aria-label="${name}"]`);
  if (!found) throw new Error(`No "${name}" control`);
  return found;
}

/** Opens a picker if it is closed and returns its listbox. */
export function openPicker(name: string) {
  const button = trigger(name);
  if (button.getAttribute("aria-expanded") !== "true") act(() => button.click());
  const listbox = document.getElementById(button.getAttribute("aria-controls") ?? "");
  if (!listbox) throw new Error(`"${name}" has no listbox`);
  return listbox;
}

/** The option in a picker whose text contains every part. */
export function optionIn(name: string, ...parts: string[]) {
  const found = [...openPicker(name).querySelectorAll<HTMLElement>("[role=option]")]
    .find((element) => parts.every((part) => element.textContent?.includes(part)));
  if (!found) throw new Error(`No option with ${parts.join(" + ")} in "${name}"`);
  return found;
}

export async function pick(name: string, ...parts: string[]) {
  const found = optionIn(name, ...parts);
  act(() => found.click());
  await settle();
}

export const isDisabled = (element: HTMLElement) =>
  element.hasAttribute("disabled") || element.getAttribute("aria-disabled") === "true";

export function type(input: HTMLInputElement, value: string) {
  const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")?.set;
  act(() => { setter?.call(input, value); input.dispatchEvent(new Event("input", { bubbles: true })); });
}

export function input(selector: string) {
  const found = document.querySelector<HTMLInputElement>(selector);
  if (!found) throw new Error(`No input ${selector}`);
  return found;
}

/** The input labelled `label` (a label[for] or an aria-label). */
export function labelledInput(label: string) {
  const byLabel = [...document.querySelectorAll<HTMLLabelElement>("label[for]")].find((element) => element.textContent?.trim() === label);
  const found = byLabel ? document.getElementById(byLabel.htmlFor) : document.querySelector(`input[aria-label="${label}"]`);
  if (!(found instanceof HTMLInputElement)) throw new Error(`No input labelled "${label}"`);
  return found;
}

export async function submitForm() {
  const form = document.querySelector("form");
  if (!form) throw new Error("No form");
  await act(async () => { form.dispatchEvent(new Event("submit", { bubbles: true, cancelable: true })); });
  await settle();
}

export function buttonByText(text: string) {
  return [...document.querySelectorAll<HTMLButtonElement>("button")].find((element) => element.textContent?.trim() === text) ?? null;
}

/** The Platform options disclosure: a button naming the section that carries aria-expanded. */
export function platformOptionsToggle() {
  return [...document.querySelectorAll<HTMLButtonElement>("button[aria-expanded]")]
    .find((element) => element.textContent?.includes("Platform options")) ?? null;
}
