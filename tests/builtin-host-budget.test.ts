// @vitest-environment jsdom
/**
 * The built-in runner host's budget on Settings → Runner host: set in GridOps
 * like a fleet machine's, with the runner manager's own figures as the
 * fallback.
 */
import { act, createElement, type ReactElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import {
  type BuiltinBudgetDraft, builtinBudgetDraft, builtinBudgetRequest, budgetSourceLabel,
} from "../src/features/runner-pools/builtin-host";
import { BuiltinHostBudgetSection } from "../src/features/runner-pools/builtin-host-budget-section";

const toast = vi.hoisted(() => ({ success: vi.fn(), error: vi.fn() }));
vi.mock("sonner", () => ({ toast }));

declare global {
  var IS_REACT_ACT_ENVIRONMENT: boolean | undefined;
}

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
  vi.unstubAllGlobals();
  toast.success.mockClear();
});

async function render(element: ReactElement) {
  await act(async () => root.render(element));
}

async function settle() {
  for (let index = 0; index < 5; index += 1) await act(async () => { await new Promise((resolve) => setTimeout(resolve, 0)); });
}

/** A 16-core, 64,301 MB host at its default 95%. */
const inForce = { cpuBudget: 15.2, memoryBudgetMb: 61_085 };
const reported = { cpus: 16, memoryMb: 64_301 };
const unset = { cpuMillis: null, memoryMib: null };
const defaults = { cpu: "default", memory: "default" } as const;

function type(input: HTMLInputElement, value: string) {
  const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")?.set;
  act(() => {
    setter?.call(input, value);
    input.dispatchEvent(new Event("input", { bubbles: true }));
  });
}

const input = (id: string) => {
  const found = document.querySelector<HTMLInputElement>(`#${id}`);
  if (!found) throw new Error(`No #${id}`);
  return found;
};
const button = (text: string) => {
  const found = [...document.querySelectorAll<HTMLButtonElement>("button")].find((candidate) => candidate.textContent?.includes(text));
  if (!found) throw new Error(`No "${text}" button`);
  return found;
};

describe("the built-in host's budget request", () => {
  const draft = (overrides: Partial<BuiltinBudgetDraft> = {}) => ({ ...builtinBudgetDraft(inForce), ...overrides });

  it("starts from the budget in force", () => {
    expect(builtinBudgetDraft(inForce)).toEqual({ cores: "15.2", memoryGb: "59.65" });
  });

  it("sends only the figures that were edited, in the API's units", () => {
    expect(builtinBudgetRequest(inForce, draft({ cores: "12" }), reported)).toEqual({ ok: true, request: { cpuMillis: 12_000 } });
    expect(builtinBudgetRequest(inForce, draft({ memoryGb: "48" }), reported)).toEqual({ ok: true, request: { memoryMib: 49_152 } });
    expect(builtinBudgetRequest(inForce, draft(), reported)).toEqual({ ok: false, error: "Nothing to save." });
  });

  it("refuses a figure that is not above zero or exceeds what the host reports", () => {
    expect(builtinBudgetRequest(inForce, draft({ cores: "0" }), reported)).toEqual({ ok: false, error: "CPU must be a number greater than zero." });
    expect(builtinBudgetRequest(inForce, draft({ memoryGb: "" }), reported)).toEqual({ ok: false, error: "Memory must be a number greater than zero." });
    expect(builtinBudgetRequest(inForce, draft({ cores: "17" }), reported)).toEqual({ ok: false, error: "CPU can’t exceed the 16 cores this host reports." });
    expect(builtinBudgetRequest(inForce, draft({ memoryGb: "63" }), reported)).toEqual({ ok: false, error: "Memory can’t exceed the 62.79 GB this host reports." });
    expect(builtinBudgetRequest(inForce, draft({ cores: "16" }), reported)).toEqual({ ok: true, request: { cpuMillis: 16_000 } });
  });

  it("says where each figure comes from", () => {
    expect(budgetSourceLabel("setting", "cpu")).toBe("Set here");
    expect(budgetSourceLabel("environment", "cpu")).toBe("From GRIDOPS_RUNNER_CPU_BUDGET on the runner manager");
    expect(budgetSourceLabel("environment", "memory")).toBe("From GRIDOPS_RUNNER_MEMORY_BUDGET_MB on the runner manager");
    expect(budgetSourceLabel("default", "memory")).toBe("Default: 95% of the host");
    expect(budgetSourceLabel(undefined, "cpu")).toBe("Set on the runner manager");
  });
});

describe("Settings → Runner host: Budget", () => {
  it("lets an administrator change one figure and sends only that figure", async () => {
    const fetcher = vi.fn().mockResolvedValue(Response.json({
      ok: true, budget: { cpuMillis: 12_000, memoryMib: null }, changed: true, managerSynced: true,
      effective: { cpuBudget: 12, memoryBudgetMb: 61_085, maxRunners: 6, source: { cpu: "setting", memory: "default" } },
    }));
    vi.stubGlobal("fetch", fetcher);
    const saved = vi.fn();
    await render(createElement(BuiltinHostBudgetSection, { isAdmin: true, inForce, onSaved: saved, reported, setting: unset, source: defaults }));

    expect(document.body.textContent).toContain("Default: 95% of the host");
    expect(document.body.textContent).toContain("Host reports 16 cores");
    expect(document.body.textContent).not.toContain("Use the runner manager’s own figures");
    type(input("builtin-budget-cpu"), "12");
    act(() => { button("Save budget").click(); });
    await settle();

    const [url, init] = fetcher.mock.calls[0] as [string, RequestInit];
    expect(url).toBe("/api/v1/settings/runner-host/budget");
    expect(init.method).toBe("PUT");
    expect(JSON.parse(String(init.body))).toEqual({ cpuMillis: 12_000 });
    expect(toast.success).toHaveBeenCalledWith("Budget saved.");
    expect(saved).toHaveBeenCalledTimes(1);
  });

  it("returns a set budget to the runner manager's own figures", async () => {
    const fetcher = vi.fn().mockResolvedValue(Response.json({ ok: true, budget: unset, changed: true, managerSynced: false, effective: null }));
    vi.stubGlobal("fetch", fetcher);
    await render(createElement(BuiltinHostBudgetSection, {
      isAdmin: true, inForce: { cpuBudget: 12, memoryBudgetMb: 61_085 }, reported,
      setting: { cpuMillis: 12_000, memoryMib: null }, source: { cpu: "setting", memory: "default" },
    }));
    expect(document.body.textContent).toContain("Set here");
    act(() => { button("Use the runner manager’s own figures").click(); });
    await settle();
    expect(JSON.parse(String((fetcher.mock.calls[0] as [string, RequestInit])[1].body))).toEqual({ cpuMillis: null, memoryMib: null });
    expect(toast.success).toHaveBeenCalledWith(expect.stringContaining("the runner manager has not confirmed yet"));
  });

  it("shows what the API refused and keeps the form", async () => {
    vi.stubGlobal("fetch", vi.fn().mockResolvedValue(Response.json({ error: "CPU can't exceed the 16 cores the built-in host reports." }, { status: 400 })));
    await render(createElement(BuiltinHostBudgetSection, { isAdmin: true, inForce, reported, setting: unset, source: defaults }));
    type(input("builtin-budget-cpu"), "15.9");
    act(() => { button("Save budget").click(); });
    await settle();
    expect(document.querySelector("[role=alert]")?.textContent).toContain("16 cores the built-in host reports");
    expect(input("builtin-budget-cpu").value).toBe("15.9");
  });

  it("checks the figure before sending it", async () => {
    const fetcher = vi.fn();
    vi.stubGlobal("fetch", fetcher);
    await render(createElement(BuiltinHostBudgetSection, { isAdmin: true, inForce, reported, setting: unset, source: defaults }));
    type(input("builtin-budget-cpu"), "20");
    act(() => { button("Save budget").click(); });
    await settle();
    expect(document.querySelector("[role=alert]")?.textContent).toBe("CPU can’t exceed the 16 cores this host reports.");
    expect(fetcher).not.toHaveBeenCalled();
  });

  it("is read only for members", async () => {
    await render(createElement(BuiltinHostBudgetSection, { isAdmin: false, inForce, reported, setting: unset, source: defaults }));
    expect(document.querySelector("input")).toBeNull();
    expect(document.body.textContent).toContain("15.2 cores");
    expect(document.body.textContent).toContain("59.7 GB");
    expect(document.body.textContent).toContain("Administrators");
  });
});
