// @vitest-environment jsdom
/** Drives the API token sections in a DOM: retries, the one-time secret and revoking. */
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { createElement, act, type ReactElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { CreateTokenSection, TokenListSection } from "../src/features/fleet/api-tokens-section";

declare global {
  var IS_REACT_ACT_ENVIRONMENT: boolean | undefined;
}

const tokenId = "00000009-0000-4000-8000-000000000001";
const secret = `gat_AbCdEfGh${"x".repeat(34)}A`;
const token = {
  id: tokenId, name: "ci-bot", displayPrefix: "AbCdEfGh", scopes: ["fleet:read"],
  createdAt: "2026-11-01T20:00:00.000Z", expiresAt: "2099-01-30T20:00:00.000Z", lastUsedAt: null, revokedAt: null,
};

let container: HTMLDivElement;
let root: Root;

// jsdom has no layout; Radix's switch only needs the observer to exist.
class NoopResizeObserver {
  observe() {}
  unobserve() {}
  disconnect() {}
}

beforeEach(() => {
  globalThis.IS_REACT_ACT_ENVIRONMENT = true;
  vi.stubGlobal("ResizeObserver", NoopResizeObserver);
  container = document.createElement("div");
  document.body.append(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  document.body.innerHTML = "";
  vi.unstubAllGlobals();
});

async function render(element: ReactElement) {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  await act(async () => root.render(createElement(QueryClientProvider, { client }, element)));
}

/** Waits for in-flight fetches and the state updates that follow them. */
async function settle() {
  for (let index = 0; index < 5; index += 1) await act(async () => { await new Promise((resolve) => setTimeout(resolve, 0)); });
}

function type(input: HTMLInputElement, value: string) {
  const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")?.set;
  act(() => {
    setter?.call(input, value);
    input.dispatchEvent(new Event("input", { bubbles: true }));
  });
}

const byId = <T extends HTMLElement>(id: string) => {
  const element = document.getElementById(id);
  if (!element) throw new Error(`#${id} is missing`);
  return element as T;
};
const button = (name: string) => {
  const found = [...document.querySelectorAll("button")].find((element) => element.textContent?.trim() === name);
  if (!found) throw new Error(`No "${name}" button`);
  return found;
};
const click = (element: HTMLElement) => act(() => { element.click(); });
const submitForm = () => act(() => { byId<HTMLInputElement>("api-token-name").form?.requestSubmit(); });

function postKeys(fetcher: ReturnType<typeof vi.fn>) {
  return (fetcher.mock.calls as Array<[string, RequestInit]>)
    .filter(([, init]) => init.method === "POST")
    .map(([, init]) => new Headers(init.headers).get("Idempotency-Key"));
}

describe("create token section", () => {
  it("validates before sending and focuses the first invalid field", async () => {
    const fetcher = vi.fn();
    vi.stubGlobal("fetch", fetcher);
    await render(createElement(CreateTokenSection));

    submitForm();
    const name = byId<HTMLInputElement>("api-token-name");
    expect(document.activeElement).toBe(name);
    expect(name.getAttribute("aria-invalid")).toBe("true");
    expect(byId(name.getAttribute("aria-describedby") ?? "").textContent).toBe("Enter a name.");
    expect(byId("api-token-scopes-error").textContent).toBe("Choose at least one scope.");

    type(name, "ci-bot");
    type(byId<HTMLInputElement>("api-token-days"), "400");
    submitForm();
    expect(document.activeElement).toBe(byId("api-token-scope-fleet:read"));
    click(byId("api-token-scope-fleet:read"));
    submitForm();
    expect(document.activeElement).toBe(byId("api-token-days"));
    expect(byId("api-token-days-error").textContent).toMatch(/1 to 365/);
    expect(fetcher).not.toHaveBeenCalled();
  });

  it("retries a failed submission with the same Idempotency-Key and a changed one with a new key", async () => {
    const fetcher = vi.fn()
      .mockRejectedValueOnce(new TypeError("Failed to fetch"))
      .mockResolvedValueOnce(Response.json({ token }))
      .mockResolvedValueOnce(Response.json({ code: "invalid_request", message: "Invalid fleet request",
        requestId: "0000000a-0000-4000-8000-000000000001", details: { kind: "invalid_field", field: "name" } }, { status: 400 }))
      .mockResolvedValue(Response.json({ ...token, name: "ci-bot-2", tokenSecret: secret }, { status: 201 }));
    vi.stubGlobal("fetch", fetcher);
    await render(createElement(CreateTokenSection));

    type(byId<HTMLInputElement>("api-token-name"), "ci-bot");
    click(byId("api-token-scope-fleet:read"));
    // Two submits in the same tick send one request.
    act(() => {
      byId<HTMLInputElement>("api-token-name").form?.requestSubmit();
      byId<HTMLInputElement>("api-token-name").form?.requestSubmit();
    });
    await settle();
    expect(postKeys(fetcher)).toHaveLength(1);
    expect(document.querySelector("[role=alert]")?.textContent).toMatch(/won’t create a second token/);

    // The retry replays: no secret, an explanation, and the same key.
    submitForm();
    await settle();
    expect(document.querySelector("[role=alert]")?.textContent).toMatch(/can’t be shown again/);
    expect(document.body.textContent).not.toContain("gat_");

    // That attempt is finished; the next one, here a duplicate name, gets a new key.
    submitForm();
    await settle();
    type(byId<HTMLInputElement>("api-token-name"), "ci-bot-2");
    submitForm();
    await settle();

    const [first, retry, afterReplay, renamed] = postKeys(fetcher);
    expect(first).toMatch(/^[0-9a-f-]{36}$/);
    expect(retry).toBe(first);
    expect(afterReplay).not.toBe(first);
    expect(renamed).not.toBe(afterReplay);
    expect(document.body.textContent).toContain(secret);
  });

  it("shows the secret once, ignores Escape, and forgets it on close", async () => {
    const setItem = vi.spyOn(Storage.prototype, "setItem");
    const info = vi.spyOn(console, "info");
    const log = vi.spyOn(console, "log");
    const fetcher = vi.fn().mockResolvedValue(Response.json({ ...token, tokenSecret: secret }, { status: 201 }));
    vi.stubGlobal("fetch", fetcher);
    await render(createElement(CreateTokenSection));

    type(byId<HTMLInputElement>("api-token-name"), "ci-bot");
    click(byId("api-token-scope-fleet:read"));
    submitForm();
    await settle();

    const dialog = document.querySelector("[role=dialog]");
    expect(dialog?.textContent).toContain(secret);
    expect(dialog?.textContent).toMatch(/only time GridOps shows the secret/);
    expect(button("Copy")).toBeTruthy();
    // The form is reset so the secret's request can't be resubmitted by accident.
    expect(byId<HTMLInputElement>("api-token-name").value).toBe("");

    act(() => { document.activeElement?.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true })); });
    expect(document.querySelector("[role=dialog]")?.textContent).toContain(secret);

    click(button("I’ve stored it, close"));
    await settle();
    expect(document.querySelector("[role=dialog]")).toBeNull();
    expect(document.body.innerHTML).not.toContain(secret);
    expect(document.activeElement).toBe(byId("api-token-name"));
    expect(window.location.href).not.toContain(secret);
    expect(setItem).not.toHaveBeenCalled();
    expect(info).not.toHaveBeenCalled();
    expect(log).not.toHaveBeenCalled();
  });
});

describe("token list section", () => {
  it("shows states without secrets and updates a row in place after revoking", async () => {
    vi.stubGlobal("confirm", vi.fn(() => true));
    const expired = { ...token, id: "00000009-0000-4000-8000-000000000002", name: "old", expiresAt: "2026-01-01T00:00:00.000Z" };
    const revoked = { ...token, revokedAt: "2026-11-02T20:00:00.000Z" };
    const fetcher = vi.fn()
      .mockResolvedValueOnce(Response.json({ items: [token, expired] }))
      .mockResolvedValueOnce(Response.json({ token: revoked }))
      .mockResolvedValue(Response.json({ items: [revoked, expired] }));
    vi.stubGlobal("fetch", fetcher);
    await render(createElement(TokenListSection));
    await settle();

    expect(document.body.textContent).toContain("1 active token");
    expect(document.body.textContent).toContain("gat_AbCdEfGh…");
    expect(document.body.textContent).toContain("Expired");

    click(button("Revoke").closest("button") ?? button("Revoke"));
    await settle();
    const calls = fetcher.mock.calls as Array<[string, RequestInit]>;
    expect(calls[1]?.[0]).toBe(`/api/v1/fleet/api-tokens/${tokenId}/revoke`);
    expect(document.body.textContent).toContain("0 active tokens");
    expect(document.body.textContent).toContain("Revoked");
    // Only the expired token still offers revoke.
    expect([...document.querySelectorAll("button")].filter((element) => element.textContent?.trim() === "Revoke")).toHaveLength(1);
  });

  it("fails closed when the list carries a secret", async () => {
    vi.stubGlobal("fetch", vi.fn().mockResolvedValue(Response.json({ items: [{ ...token, tokenSecret: secret }] })));
    await render(createElement(TokenListSection));
    await settle();
    expect(document.body.textContent).toContain("Couldn’t load tokens");
    expect(document.body.innerHTML).not.toContain(secret);
  });
});
