// @vitest-environment jsdom
/**
 * GitLab on Settings → Connections (#141), rendered with the production registry: the GitLab rows
 * and their new row actions (Verify, Rotate webhook secret) beside Edit and Remove, the Add
 * connection form (defaults, inline validation that blocks the request, the server's message
 * verbatim), and the one-time webhook reveal after create and after rotation (secret and URL, each
 * with a copy button, and the Job events setting). The API is a stubbed `fetch` shaped as
 * docs/openapi/gridops-v1.json.
 */
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const toast = vi.hoisted(() => ({ success: vi.fn(), error: vi.fn(), promise: vi.fn() }));

vi.mock("@tanstack/react-router", () => ({
  Link: ({ children, to }: { children?: unknown; to?: string }) => createElement("a", { href: String(to) }, children as never),
  useNavigate: () => vi.fn(),
  useRouter: () => ({ invalidate: vi.fn(async () => undefined) }),
}));
vi.mock("sonner", () => ({ toast }));

import { GITLAB_URL, MOBILE_APP, PLATFORM_GROUP } from "./fixtures/gitlab";
import { TooltipProvider } from "../src/components/ui/tooltip";
import type { SettingsPage } from "../src/features/operations/operations.functions";
import { ConnectionsSection } from "../src/features/platform-connections/connections-section";

declare global {
  var IS_REACT_ACT_ENVIRONMENT: boolean | undefined;
}

class NoopResizeObserver { observe() {} unobserve() {} disconnect() {} }

const WEBHOOK_URL = `https://gridops.example/api/v1/webhooks/gitlab/${MOBILE_APP.id}`;
const CREATED_SECRET = "glwh-created-0f3c1e9a";
const ROTATED_SECRET = "glwh-rotated-b81d7704";

function settingsPage(role: "admin" | "member"): SettingsPage {
  return {
    authenticated: true,
    data: {
      configuration: {
        githubOAuth: true, githubAppControl: true, webhookActive: false, webhookVerification: false, secureStorage: true, runnerManager: true,
        installationTokens: true, callbackUrl: "https://gridops.example/auth/github/callback", webhookUrl: "https://gridops.example/api/v1/webhooks/github",
      },
      githubApp: { slug: "gridops-self-hosted", appUrl: "https://github.com/apps/gridops-self-hosted", installUrl: "https://github.com/apps/gridops-self-hosted/installations/new" },
      installations: [
        { id: 1, accountLogin: "octo-org", accountType: "Organization", accountAvatarUrl: null, repositorySelection: "all", permission: "admin", suspended: false, lastSyncedAt: null, poolCount: 3, manageUrl: "https://github.com/organizations/octo-org/settings/installations/1" },
      ],
      manager: { ok: true },
      settings: {
        logRetentionDays: 30, logStorageBudgetMb: 1024, webhookRetentionDays: 30, auditRetentionDays: 90, reconcileIntervalSeconds: 30,
        githubSyncIntervalSeconds: 300, autoUpdateImages: false, provisioningPaused: false, builtinRunnerHostEnabled: true,
      },
      user: { id: "u-1", login: role === "admin" ? "octocat" : "member-1", role },
      users: [],
    },
  } as SettingsPage;
}

let container: HTMLDivElement;
let root: Root;
let fetcher: ReturnType<typeof vi.fn>;
let gitlabList: { canManage: boolean; items: unknown[] };
/** Per-route overrides: a response the stub returns instead of the success one. */
let failures: Record<string, () => Response>;

function apiFetch(url: string, init?: RequestInit): Response {
  const method = init?.method ?? "GET";
  const override = failures[`${method} ${url}`];
  if (override) return override();
  if (method === "GET" && url === "/api/v1/platform-connections/bitbucket") return Response.json({ canManage: true, items: [] });
  if (method === "GET" && url === GITLAB_URL) return Response.json(gitlabList);
  if (method === "POST" && url === GITLAB_URL) {
    return Response.json({ connection: MOBILE_APP, webhookUrl: WEBHOOK_URL, webhookSecret: CREATED_SECRET }, { status: 201 });
  }
  if (method === "POST" && url === `${GITLAB_URL}/${MOBILE_APP.id}/verify`) return new Response(null, { status: 204 });
  if (method === "POST" && url === `${GITLAB_URL}/${MOBILE_APP.id}/webhook-secret/rotate`) return Response.json({ webhookUrl: WEBHOOK_URL, webhookSecret: ROTATED_SECRET });
  return Response.json({ error: `not in this test: ${method} ${url}` }, { status: 404 });
}

beforeEach(() => {
  globalThis.IS_REACT_ACT_ENVIRONMENT = true;
  vi.stubGlobal("ResizeObserver", NoopResizeObserver);
  gitlabList = { canManage: true, items: [MOBILE_APP, PLATFORM_GROUP] };
  failures = {};
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

async function settle() {
  for (let index = 0; index < 6; index += 1) await act(async () => { await new Promise((resolve) => setTimeout(resolve, 0)); });
}

async function render(role: "admin" | "member" = "admin") {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  await act(async () => root.render(createElement(QueryClientProvider, { client }, createElement(TooltipProvider, null, createElement(ConnectionsSection, { page: settingsPage(role) })))));
  await settle();
}

const click = async (element: Element | null | undefined) => {
  if (!(element instanceof HTMLElement)) throw new Error("Nothing to click");
  act(() => element.click());
  await settle();
};
const buttons = (text: string, scope: ParentNode = document) => [...scope.querySelectorAll<HTMLButtonElement>("button")].filter((element) => element.textContent?.trim() === text);
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

/** The connection row showing `name`: the nearest ancestor of the name that holds the row's Edit button. */
function row(name: string) {
  const label = [...document.querySelectorAll<HTMLElement>("span, div")].find((element) => element.textContent?.trim() === name);
  let element: HTMLElement | null = label ?? null;
  while (element && !buttons("Edit", element).length) element = element.parentElement;
  if (!element) throw new Error(`No row for "${name}"`);
  return element;
}

function dialogWith(text: string) {
  return [...document.querySelectorAll<HTMLElement>("[role=dialog]")].find((element) => element.textContent?.includes(text)) ?? null;
}

/** Every toast message the page raised, success and error, including the outcome of `toast.promise`. */
async function toastMessages() {
  const successes = (toast.success.mock.calls as unknown[][]).map((call) => String(call[0]));
  const errors = (toast.error.mock.calls as unknown[][]).map((call) => String(call[0]));
  for (const [pending, options] of toast.promise.mock.calls as Array<[Promise<unknown>, { success?: unknown; error?: unknown }]>) {
    try {
      const value = await pending;
      successes.push(typeof options.success === "function" ? String(options.success(value)) : String(options.success));
    } catch (cause) {
      errors.push(typeof options.error === "function" ? String(options.error(cause)) : String(options.error));
    }
  }
  return { successes, errors };
}

async function openGitlabForm() {
  await click(buttons("Add connection")[0]);
  const dialog = document.querySelector<HTMLElement>("[role=dialog]");
  if (!dialog) throw new Error("Add connection opened no dialog");
  const choice = [...dialog.querySelectorAll<HTMLElement>("button, [role=radio], label")].find((element) => element.textContent?.trim().startsWith("GitLab"));
  if (!choice) throw new Error("Add connection does not offer GitLab");
  await click(choice);
  return dialog;
}

async function submitGitlabForm(dialog: HTMLElement) {
  const submit = dialog.querySelector<HTMLButtonElement>("form button[type=submit]");
  if (!submit) throw new Error("The GitLab form has no submit button");
  await click(submit);
}

describe("GitLab connection rows", () => {
  it("lists each GitLab connection with its scope kind and address, and Edit, Verify, Rotate webhook secret, Remove for a manager", async () => {
    await render("admin");
    expect(body()).toContain("Mobile app");
    expect(body()).toContain("gitlab.com/acme/platform/mobile-app");
    expect(body()).toContain("git.example.com/platform");
    const mobile = row("Mobile app");
    expect(mobile.textContent).toContain("GitLab");
    expect(mobile.textContent).toContain("Project");
    expect(row("Platform group").textContent).toContain("Group");
    const actions = [...mobile.querySelectorAll("button")].map((element) => element.textContent?.trim())
      .filter((text) => ["Edit", "Verify", "Rotate webhook secret", "Remove"].includes(text ?? ""));
    expect(actions).toEqual(["Edit", "Verify", "Rotate webhook secret", "Remove"]);
    expect(calls("GET", GITLAB_URL)).toHaveLength(1);
  });

  it("shows a member the rows with no Verify, Rotate webhook secret, Edit or Remove", async () => {
    gitlabList = { canManage: false, items: [{ ...MOBILE_APP, canManage: false }] };
    await render("member");
    expect(body()).toContain("Mobile app");
    for (const text of ["Verify", "Rotate webhook secret", "Edit", "Remove"]) expect(buttons(text), text).toHaveLength(0);
  });

  it("Verify posts to the connection's verify route and reports success; a refused token reports the server's message", async () => {
    await render("admin");
    await click(buttons("Verify", row("Mobile app"))[0]);
    expect(calls("POST", `${GITLAB_URL}/${MOBILE_APP.id}/verify`)).toHaveLength(1);
    let messages = await toastMessages();
    expect(messages.errors).toEqual([]);
    expect(messages.successes.length).toBeGreaterThan(0);

    for (const fn of Object.values(toast)) fn.mockReset();
    failures[`POST ${GITLAB_URL}/${MOBILE_APP.id}/verify`] = () => Response.json({ error: "The token is missing the read_api scope." }, { status: 400 });
    await click(buttons("Verify", row("Mobile app"))[0]);
    messages = await toastMessages();
    expect(messages.errors).toContain("The token is missing the read_api scope.");
  });

  it("Rotate webhook secret asks first, then reveals the new secret and the webhook URL once; declining sends nothing", async () => {
    const confirm = vi.fn((_message?: string) => false);
    vi.stubGlobal("confirm", confirm);
    await render("admin");
    await click(buttons("Rotate webhook secret", row("Mobile app"))[0]);
    expect(confirm).toHaveBeenCalledTimes(1);
    expect(String(confirm.mock.calls[0]?.[0])).toContain("Mobile app");
    expect(calls("POST", `${GITLAB_URL}/${MOBILE_APP.id}/webhook-secret/rotate`)).toHaveLength(0);

    confirm.mockReturnValue(true);
    await click(buttons("Rotate webhook secret", row("Mobile app"))[0]);
    expect(calls("POST", `${GITLAB_URL}/${MOBILE_APP.id}/webhook-secret/rotate`)).toHaveLength(1);
    const reveal = dialogWith(ROTATED_SECRET);
    expect(reveal, "the rotation reveal shows the new secret").not.toBeNull();
    expect(reveal?.textContent).toContain(WEBHOOK_URL);

    await click(buttons("I’ve stored it, close")[0]);
    expect(document.body.innerHTML).not.toContain(ROTATED_SECRET);
  });
});

describe("Add connection → GitLab", () => {
  it("is offered after GitHub and Bitbucket, and its form starts with the address https://gitlab.com and the Project kind", async () => {
    await render("admin");
    await click(buttons("Add connection")[0]);
    const choices = [...(document.querySelector("[role=dialog]")?.querySelectorAll<HTMLElement>("button") ?? [])].map((element) => element.textContent?.trim() ?? "");
    const position = (name: string) => choices.findIndex((text) => text.startsWith(name));
    expect(position("GitLab")).toBeGreaterThan(position("Bitbucket"));
    expect(position("Bitbucket")).toBeGreaterThan(position("GitHub"));
    act(() => root.unmount());
    root = createRoot(container);

    await render("admin");
    const dialog = await openGitlabForm();
    expect(field("Connection name").value).toBe("");
    expect(field("GitLab address").value).toBe("https://gitlab.com");
    expect(field("Project or group path").value).toBe("");
    expect(field("Access token").type).toBe("password");
    // The Project/Group choice is the shared SearchableSelect (a listbox trigger), starting on Project.
    const kind = [...dialog.querySelectorAll<HTMLButtonElement>('button[aria-haspopup="listbox"]')];
    expect(kind).toHaveLength(1);
    expect(kind[0]?.textContent?.trim()).toBe("Project");
  });

  it("creates the connection with the spec body, keeps the success toast, and reveals the webhook secret and URL once with copy buttons and the Job events setting", async () => {
    await render("admin");
    const dialog = await openGitlabForm();
    type(field("Connection name"), "Mobile app");
    type(field("Project or group path"), "acme/platform/mobile-app");
    type(field("Access token"), "glpat-abc");
    await submitGitlabForm(dialog);

    const posts = calls("POST", GITLAB_URL);
    expect(posts).toHaveLength(1);
    expect(JSON.parse(String(posts[0]?.[1]?.body))).toEqual({
      name: "Mobile app", baseUrl: "https://gitlab.com", scopeKind: "project", scopePath: "acme/platform/mobile-app", accessToken: "glpat-abc",
    });
    expect(toast.success).toHaveBeenCalled();

    const reveal = dialogWith(CREATED_SECRET);
    expect(reveal, "the post-create reveal shows the webhook secret").not.toBeNull();
    expect(reveal?.textContent).toContain(WEBHOOK_URL);
    expect(reveal?.textContent).toContain("Job events");
    const copyButtons = [...(reveal?.querySelectorAll<HTMLButtonElement>("button") ?? [])]
      .filter((element) => element.textContent?.trim() === "Copy" || element.getAttribute("aria-label")?.startsWith("Copy"));
    expect(copyButtons.length).toBeGreaterThanOrEqual(2);

    await click(buttons("I’ve stored it, close")[0]);
    expect(document.body.innerHTML).not.toContain(CREATED_SECRET);
    expect(document.body.innerHTML).not.toContain(WEBHOOK_URL);
  });

  it("marks an invalid address or path inline and sends nothing until both are fixed", async () => {
    await render("admin");
    const dialog = await openGitlabForm();
    type(field("Connection name"), "Mobile app");
    type(field("GitLab address"), "gitlab.com?x=1");
    type(field("Project or group path"), "/acme//mobile app/");
    type(field("Access token"), "glpat-abc");
    expect(field("GitLab address").getAttribute("aria-invalid")).toBe("true");
    expect(field("Project or group path").getAttribute("aria-invalid")).toBe("true");
    await submitGitlabForm(dialog);
    expect(calls("POST", GITLAB_URL)).toHaveLength(0);

    type(field("GitLab address"), "https://git.example.com/");
    type(field("Project or group path"), "acme/platform/mobile-app");
    expect(field("GitLab address").getAttribute("aria-invalid")).not.toBe("true");
    expect(field("Project or group path").getAttribute("aria-invalid")).not.toBe("true");
    await submitGitlabForm(dialog);
    expect(calls("POST", GITLAB_URL)).toHaveLength(1);
    expect(JSON.parse(String(calls("POST", GITLAB_URL)[0]?.[1]?.body))).toMatchObject({ baseUrl: "https://git.example.com" });
  });

  it("shows the server's refusal verbatim in the dialog and keeps what was typed", async () => {
    failures[`POST ${GITLAB_URL}`] = () => Response.json({ error: "GitLab rejected the access token." }, { status: 400 });
    await render("admin");
    const dialog = await openGitlabForm();
    type(field("Connection name"), "Mobile app");
    type(field("Project or group path"), "acme/platform/mobile-app");
    type(field("Access token"), "glpat-revoked");
    await submitGitlabForm(dialog);
    expect(calls("POST", GITLAB_URL)).toHaveLength(1);
    expect(dialog.textContent).toContain("GitLab rejected the access token.");
    expect(field("Project or group path").value).toBe("acme/platform/mobile-app");
    expect(dialogWith(CREATED_SECRET)).toBeNull();
  });
});
