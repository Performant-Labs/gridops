/**
 * #197: the jsdom helpers the fleet health frontend tests share (the #195, #199, #202 and #203
 * contract tests). Not a test file itself (`vitest.config.ts` collects `*.test.ts` only).
 *
 * Each test file still mocks `@tanstack/react-router` itself (a `vi.mock` is per file): the
 * fleet health components only use its `Link`, rendered as a plain anchor.
 */
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { act, createElement, type ReactElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, vi } from "vitest";

import fixtureJson from "../src/features/fleet-health/fixture.json";
import { fleetHealthSchema, type FleetHealth } from "~/features/fleet-health/types";

declare global {
  var IS_REACT_ACT_ENVIRONMENT: boolean | undefined;
}

let container: HTMLDivElement;
let root: Root;
let client: QueryClient;

// jsdom has no layout; Radix and charts only need the observer to exist.
class NoopResizeObserver {
  observe() {}
  unobserve() {}
  disconnect() {}
}

/** Mounts a fresh root before each test and removes it after. */
export function mountEachTest() {
  beforeEach(() => {
    globalThis.IS_REACT_ACT_ENVIRONMENT = true;
    vi.stubGlobal("ResizeObserver", NoopResizeObserver);
    container = document.createElement("div");
    document.body.append(container);
    root = createRoot(container);
    client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  });
  afterEach(() => {
    act(() => root.unmount());
    document.body.innerHTML = "";
    vi.unstubAllGlobals();
  });
}

/** The fixture, parsed (a fresh copy each call, so a test may change it). */
export const fixture = (): FleetHealth => fleetHealthSchema.parse(structuredClone(fixtureJson));

/** Renders (or re-renders, keeping the same root) inside a query client. */
export async function render(element: ReactElement) {
  await act(async () => root.render(createElement(QueryClientProvider, { client }, element)));
}

/** Waits for in-flight fetches and the state updates that follow them. */
export async function settle() {
  for (let index = 0; index < 5; index += 1) {
    await act(async () => {
      await new Promise((resolve) => setTimeout(resolve, 0));
    });
  }
}

export const all = <T extends Element = HTMLElement>(selector: string, scope: ParentNode = document) =>
  [...scope.querySelectorAll(selector)] as T[];

export function one<T extends Element = HTMLElement>(selector: string, scope: ParentNode = document): T {
  const found = scope.querySelector(selector);
  if (!found) throw new Error(`${selector} is missing`);
  return found as T;
}

export function button(name: string, scope: ParentNode = document): HTMLButtonElement {
  const found = all<HTMLButtonElement>("button", scope).find(
    (element) => element.textContent?.trim() === name || element.getAttribute("aria-label") === name,
  );
  if (!found) throw new Error(`No "${name}" button`);
  return found;
}

export const click = (element: HTMLElement) =>
  act(() => {
    element.click();
  });

export function type(input: HTMLInputElement, value: string) {
  const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")?.set;
  act(() => {
    setter?.call(input, value);
    input.dispatchEvent(new Event("input", { bubbles: true }));
  });
}
