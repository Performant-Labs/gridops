// @vitest-environment jsdom
/**
 * The runner row's CI-platform badge comes from the platform registry (#145), fallback included;
 * its destination comes from the platform that owns the runner's pool scope (#169).
 */
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
    ["gitlab", "GitLab"],
    ["azure-devops", "azure-devops"],
  ])("shows the registry name for %s (the raw id for an unknown platform)", (platform, name) => {
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

/**
 * #169: a runner's destination comes from the platform that owns its pool's scope
 * (`platformForPool({ scope: runner.poolScope })`), in all three places the row shows it: the
 * right-hand column (text and hover title, `xl` and up), the line under the name below `md`,
 * and the details sheet's "Repository" row. GitHub, Bitbucket and older-server output is exactly
 * today's `runner.repository ?? runner.accountLogin`. A GitLab pool's runner shows its connection
 * name, else `gitlabBaseUrl/gitlabScopePath` with only the parts present, else "GitLab", never the
 * installation login. An unknown scope shows "—" titled "Unknown destination" and never the raw
 * scope. Copy from docs/handoffs/169/wireframe.html, frames A, B and the contract table E.
 */
describe("runner destination comes from the pool's platform (#169)", () => {
  const GITLAB_URL = "https://gitlab.example.com";
  /** A runner of a pool on this server's wire shape; `poolScope` and `gitlabConnectionName` are #169's fields. */
  const onPool = (overrides: Record<string, unknown>) => ({ ...runner("github"), ...overrides }) as Runner;
  const gitlabRunner = (overrides: Record<string, unknown>) => onPool({
    platform: "gitlab", poolScope: "gitlab", accountLogin: "octo-org", repository: null,
    gitlabRunnerId: 7, gitlabConnectionName: "Mobile app", gitlabBaseUrl: GITLAB_URL, gitlabScopePath: "acme/mobile", ...overrides,
  });
  /** A runner from a server before #169: the base fixture carries no `poolScope` and no `gitlabConnectionName`. */
  const olderServer = (overrides: Record<string, unknown>) => {
    const legacy = onPool(overrides);
    if ("poolScope" in legacy) throw new Error("The older-server fixture must not carry poolScope");
    return legacy;
  };

  const CASES: Array<[string, Runner, { text: string; title: string }]> = [
    ["a GitHub repository pool shows the repository", onPool({ poolScope: "repository", accountLogin: "acme", repository: "acme/web" }), { text: "acme/web", title: "acme/web" }],
    ["a GitHub organization pool with no repository shows the account login", onPool({ poolScope: "organization", accountLogin: "acme", repository: null }), { text: "acme", title: "acme" }],
    ["a Bitbucket runner on a GitHub pool shows what GitHub shows", onPool({ platform: "bitbucket", poolScope: "organization", accountLogin: "octo-org", repository: null }), { text: "octo-org", title: "octo-org" }],
    ["an older server's runner (no poolScope) with a repository looks as today", olderServer({ accountLogin: "acme", repository: "acme/web" }), { text: "acme/web", title: "acme/web" }],
    ["an older server's runner (no poolScope) with no repository looks as today", olderServer({ accountLogin: "acme", repository: null }), { text: "acme", title: "acme" }],
    ["a GitLab pool's runner shows its connection name", gitlabRunner({}), { text: "Mobile app", title: "Mobile app" }],
    ["a GitLab pool's runner with no connection name shows baseUrl/scopePath", gitlabRunner({ gitlabConnectionName: null }), { text: `${GITLAB_URL}/acme/mobile`, title: `${GITLAB_URL}/acme/mobile` }],
    ["a GitLab pool's runner with only a base URL shows it with no stray slash", gitlabRunner({ gitlabConnectionName: null, gitlabScopePath: null }), { text: GITLAB_URL, title: GITLAB_URL }],
    ["a GitLab pool's runner with only a scope path shows it with no stray slash", gitlabRunner({ gitlabConnectionName: null, gitlabBaseUrl: null }), { text: "acme/mobile", title: "acme/mobile" }],
    ["a GitLab pool's runner with no name and no address shows GitLab", gitlabRunner({ gitlabConnectionName: null, gitlabBaseUrl: null, gitlabScopePath: null }), { text: "GitLab", title: "GitLab" }],
    ["an unknown scope's runner shows the neutral fallback", onPool({ poolScope: "azure-devops", accountLogin: "octo-org", repository: "octo-org/app" }), { text: "—", title: "Unknown destination" }],
  ];

  /** The one element of the row carrying this responsive class (jsdom applies no CSS, so all three places render). */
  function only(className: string) {
    const found = container.querySelectorAll(`[class~="${className}"]`);
    expect(found, `exactly one element with class ${className}`).toHaveLength(1);
    return found[0] as HTMLElement;
  }

  /** The value of the details sheet's "Repository" row. */
  async function detailsDestination(name: string) {
    const open = document.querySelector<HTMLButtonElement>(`button[aria-label="Show details for ${name}"]`);
    if (!open) throw new Error("No details button");
    await act(async () => { open.click(); });
    const label = [...document.querySelectorAll("dt")].find((term) => term.textContent === "Repository");
    if (!label) throw new Error("The details sheet has no Repository row");
    return label.nextElementSibling?.textContent;
  }

  it.each(CASES)("%s", async (_case, subject, expected) => {
    act(() => root.render(createElement(TooltipProvider, null, createElement(RunnerRow, { runner: subject }))));

    const column = only("xl:block");
    expect({ text: column.textContent, title: column.getAttribute("title") }, "right-hand column").toEqual(expected);
    // Below `md` the line under the name shows the destination while no job runs; it has no title.
    expect(only("md:hidden").textContent, "narrow-screen line").toBe(expected.text);
    expect(await detailsDestination(subject.name), "details sheet").toBe(expected.text);
  });

  it("the narrow-screen line still shows the running job's name over the destination", () => {
    act(() => root.render(createElement(TooltipProvider, null, createElement(RunnerRow, { runner: gitlabRunner({ currentJobName: "build:ios" }) }))));
    expect(only("md:hidden").textContent).toBe("build:ios");
    expect(only("xl:block").textContent).toBe("Mobile app");
  });
});
