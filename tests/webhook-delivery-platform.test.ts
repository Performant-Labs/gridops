// @vitest-environment jsdom
/**
 * The deliveries list names each delivery's CI platform (#141). The list API has no platform
 * field: a delivery id prefixed with a registered descriptor's id (`gitlab:<connection>:<key>`,
 * from #140) belongs to that platform, anything else to GitHub. Each row and the detail sheet show
 * the registry's platform badge; the "GitHub App" fallback destination is GitHub's alone, so a
 * GitLab delivery without a repository never reads as a GitHub App delivery.
 */
import { act, createElement, type ComponentType } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const loaderData = vi.hoisted(() => ({ current: undefined as unknown }));

vi.mock("@tanstack/react-router", () => ({
  createFileRoute: () => (options: Record<string, unknown>) => ({ options, useLoaderData: () => loaderData.current }),
  Link: ({ children, to }: { children?: unknown; to?: string }) => createElement("a", { href: String(to) }, children as never),
  useMatch: () => undefined,
  useNavigate: () => vi.fn(),
  useRouter: () => ({ invalidate: vi.fn(async () => undefined) }),
}));
vi.mock("sonner", () => ({ toast: { success: vi.fn(), error: vi.fn(), promise: vi.fn() } }));

import { MOBILE_APP } from "./fixtures/gitlab";
import { TooltipProvider } from "../src/components/ui/tooltip";
import type { WebhookDelivery } from "../src/features/operations/operations.functions";
import { Route as WebhooksRoute } from "../src/routes/_app/webhooks";

declare global {
  var IS_REACT_ACT_ENVIRONMENT: boolean | undefined;
}

class NoopResizeObserver { observe() {} unobserve() {} disconnect() {} }

const delivery = (overrides: Partial<WebhookDelivery>): WebhookDelivery => ({
  id: "d-1", event: "workflow_job", action: "completed", installationId: 1, repositoryId: 10, signatureValid: true, status: "processed", error: null,
  receivedAt: "2026-11-20T18:00:00.000Z", processedAt: "2026-11-20T18:00:01.000Z", accountLogin: "acme", repository: "acme/web",
  canRetry: false, hasPayload: true,
  ...overrides,
});

const GITHUB_JOB = delivery({ id: "3f1b9a70-1d2c-11f0-8e1a-5a6b7c8d9e01" });
const GITHUB_APP = delivery({ id: "3f1b9a70-1d2c-11f0-8e1a-5a6b7c8d9e02", event: "installation", action: "created", installationId: null, repositoryId: null, accountLogin: null, repository: null });
const GITLAB_JOB = delivery({ id: `gitlab:${MOBILE_APP.id}:job-77`, event: "Job Hook", action: null, installationId: null, repositoryId: null, accountLogin: null, repository: null });
const GITLAB_UNMATCHED = delivery({ id: "gitlab:unmatched:job-78", event: "Job Hook", action: null, installationId: null, repositoryId: null, accountLogin: null, repository: null, status: "rejected", signatureValid: false });
const DELIVERIES = [GITHUB_JOB, GITHUB_APP, GITLAB_JOB, GITLAB_UNMATCHED];

let container: HTMLDivElement;
let root: Root;

beforeEach(() => {
  globalThis.IS_REACT_ACT_ENVIRONMENT = true;
  vi.stubGlobal("ResizeObserver", NoopResizeObserver);
  vi.stubGlobal("fetch", vi.fn(async (url: string) => {
    const found = DELIVERIES.find((item) => url === `/api/v1/webhooks/${encodeURIComponent(item.id)}`);
    return found ? Response.json({ id: found.id, event: found.event, payload: { object_kind: "build" }, payloadBytes: 24 }) : Response.json({ error: `not in this test: ${url}` }, { status: 404 });
  }));
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

async function render() {
  loaderData.current = { authenticated: true, items: DELIVERIES, total: DELIVERIES.length, page: 1, perPage: 25, failedTotal: 0 };
  const page = (WebhooksRoute as unknown as { options: { component: ComponentType } }).options.component;
  await act(async () => root.render(createElement(TooltipProvider, null, createElement(page))));
  await settle();
}

function inspect(item: WebhookDelivery) {
  const button = document.querySelector<HTMLButtonElement>(`button[aria-label="Inspect delivery ${item.id}"]`);
  if (!button) throw new Error(`No row for delivery ${item.id}`);
  return button;
}

const rowText = (item: WebhookDelivery) => inspect(item).parentElement?.textContent ?? "";

describe("deliveries list platform", () => {
  it("badges GitHub deliveries as GitHub, and keeps the GitHub App destination for one with no repository", async () => {
    await render();
    expect(rowText(GITHUB_JOB)).toContain("GitHub");
    expect(rowText(GITHUB_JOB)).not.toContain("GitLab");
    // The badge and the fallback destination: "GitHub" twice, once as "GitHub App".
    expect(rowText(GITHUB_APP)).toContain("GitHub App");
    expect(rowText(GITHUB_APP).match(/GitHub/g) ?? []).toHaveLength(2);
    expect(rowText(GITHUB_APP)).not.toContain("GitLab");
  });

  it.each([
    ["a matched GitLab delivery", GITLAB_JOB],
    ["an unmatched (rejected) GitLab delivery", GITLAB_UNMATCHED],
  ])("badges %s as GitLab, with no GitHub destination", async (_name, item) => {
    await render();
    expect(rowText(item)).toContain("GitLab");
    expect(rowText(item)).not.toContain("GitHub");
  });

  it("puts the GitLab badge in a GitLab delivery's detail sheet and shows no GitHub App destination there", async () => {
    await render();
    await act(async () => { inspect(GITLAB_JOB).click(); });
    await settle();
    const sheet = document.querySelector<HTMLElement>("[role=dialog]");
    expect(sheet?.textContent).toContain("Job Hook");
    expect(sheet?.textContent).toContain("GitLab");
    expect(sheet?.textContent).not.toContain("GitHub App");
    expect(sheet?.textContent).toContain("Destination");
  });
});
