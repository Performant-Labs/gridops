// @vitest-environment jsdom
/**
 * The built-in runner host switch in the UI: the Settings → Runner host section,
 * what a pool's create and settings pages offer while it is off, and how a pool
 * with nowhere to run is shown.
 */
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { act, createElement, type ReactElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { statusLabel, healthTone } from "../src/components/status-icon";
import { FleetExecutionSection } from "../src/features/fleet/fleet-profile-section";
import {
  type BuiltinHostChange, BUILTIN_HOST_OFF_LABEL, builtinHostChangeMessage, builtinHostOn, isBuiltinHostOff, showsBuiltinGuardrails,
} from "../src/features/runner-pools/builtin-host";
import { BuiltinHostOffNotice } from "../src/features/runner-pools/builtin-host-off-notice";
import { BuiltinHostSection } from "../src/features/runner-pools/builtin-host-section";

vi.mock("@tanstack/react-router", () => ({
  Link: ({ children, to, ...rest }: { children?: unknown; to?: string }) =>
    createElement("a", { href: String(to), "data-link": "" }, children as never, rest && null),
}));
const toast = vi.hoisted(() => ({ success: vi.fn(), error: vi.fn() }));
vi.mock("sonner", () => ({ toast }));

declare global {
  var IS_REACT_ACT_ENVIRONMENT: boolean | undefined;
}

class NoopResizeObserver {
  observe() {}
  unobserve() {}
  disconnect() {}
}

let container: HTMLDivElement;
let root: Root;

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
  toast.success.mockClear();
});

async function render(element: ReactElement) {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  await act(async () => root.render(createElement(QueryClientProvider, { client }, element)));
}

async function settle() {
  for (let index = 0; index < 5; index += 1) await act(async () => { await new Promise((resolve) => setTimeout(resolve, 0)); });
}

const switchNamed = (name: string) => {
  const found = document.querySelector<HTMLButtonElement>(`button[role=switch][aria-label="${name}"]`);
  if (!found) throw new Error(`No "${name}" switch`);
  return found;
};
const hasSwitch = (name: string) => document.querySelector(`button[role=switch][aria-label="${name}"]`) !== null;
const click = (element: HTMLElement) => act(() => { element.click(); });

const change = (overrides: Partial<BuiltinHostChange> = {}): BuiltinHostChange => ({
  ok: true, builtinRunnerHostEnabled: false, changed: true, managerSynced: true,
  cannotRunPools: [], runnersOnBuiltinHost: 0, ...overrides,
});

describe("Settings → Runner host: the built-in runner host section", () => {
  it("lets an administrator turn it off, saves the one setting and names the pools that can no longer run", async () => {
    const fetcher = vi.fn().mockResolvedValue(Response.json(change({ cannotRunPools: [{ id: "p1", name: "legacy-linux" }], runnersOnBuiltinHost: 2 })));
    vi.stubGlobal("fetch", fetcher);
    const changed = vi.fn();
    await render(createElement(BuiltinHostSection, { enabled: true, isAdmin: true, onChanged: changed }));

    expect(switchNamed("Use the built-in runner host").getAttribute("aria-checked")).toBe("true");
    expect(document.body.textContent).toContain("Runners may start on this machine");
    click(switchNamed("Use the built-in runner host"));
    await settle();

    const [url, init] = fetcher.mock.calls[0] as [string, RequestInit];
    expect(url).toBe("/api/v1/settings/runner-host");
    expect(init.method).toBe("PUT");
    expect(JSON.parse(String(init.body))).toEqual({ builtinHostEnabled: false });
    expect(switchNamed("Use the built-in runner host").getAttribute("aria-checked")).toBe("false");
    expect(document.body.textContent).toContain("GridOps starts no runner on the machine it runs on");
    expect(document.body.textContent).toContain("legacy-linux");
    expect(document.body.textContent).toContain("Cannot run: built-in runner host is off");
    expect(toast.success).toHaveBeenCalledWith(expect.stringMatching(/1 pool has no fleet execution and cannot run: legacy-linux.*2 runners are still on the host/));
    expect(changed).toHaveBeenCalledTimes(1);
  });

  it("turns it back on", async () => {
    const fetcher = vi.fn().mockResolvedValue(Response.json(change({ builtinRunnerHostEnabled: true })));
    vi.stubGlobal("fetch", fetcher);
    await render(createElement(BuiltinHostSection, { enabled: false, isAdmin: true }));
    click(switchNamed("Use the built-in runner host"));
    await settle();
    expect(JSON.parse(String((fetcher.mock.calls[0] as [string, RequestInit])[1].body))).toEqual({ builtinHostEnabled: true });
    expect(switchNamed("Use the built-in runner host").getAttribute("aria-checked")).toBe("true");
  });

  it("shows the error and keeps the old state when the change is refused", async () => {
    vi.stubGlobal("fetch", vi.fn().mockResolvedValue(Response.json({ error: "forbidden" }, { status: 403 })));
    await render(createElement(BuiltinHostSection, { enabled: true, isAdmin: true }));
    click(switchNamed("Use the built-in runner host"));
    await settle();
    expect(document.body.textContent).toContain("Could not change the setting");
    expect(switchNamed("Use the built-in runner host").getAttribute("aria-checked")).toBe("true");
  });

  it("is read only for members", async () => {
    const fetcher = vi.fn();
    vi.stubGlobal("fetch", fetcher);
    await render(createElement(BuiltinHostSection, { enabled: false, isAdmin: false }));
    expect(hasSwitch("Use the built-in runner host")).toBe(false);
    expect(document.body.textContent).toContain("Off");
    expect(document.body.textContent).toContain("Administrators");
    expect(fetcher).not.toHaveBeenCalled();
  });

  it("says when the manager runs without Docker access or has not caught up", async () => {
    await render(createElement(BuiltinHostSection, { dockerAvailable: false, enabled: true, isAdmin: true, managerSetting: false }));
    expect(document.body.textContent).toContain("GRIDOPS_MANAGER_DOCKER=off");
    expect(document.body.textContent).toContain("Still off; it catches up within one reconcile interval");
  });

  it("describes what the page hides while the host is off", () => {
    expect(showsBuiltinGuardrails(false)).toBe(false);
    expect(showsBuiltinGuardrails(true)).toBe(true);
    expect(showsBuiltinGuardrails(undefined)).toBe(true);
    expect(builtinHostOn(false)).toBe(false);
  });
});

describe("how a change is reported", () => {
  it("covers on, off, unchanged and a manager that has not confirmed", () => {
    expect(builtinHostChangeMessage(change({ builtinRunnerHostEnabled: true }))).toBe("Built-in runner host turned on.");
    expect(builtinHostChangeMessage(change({ changed: false }))).toBe("The built-in runner host was already off.");
    expect(builtinHostChangeMessage(change({ managerSynced: false }))).toMatch(/runner manager has not confirmed yet/);
    expect(builtinHostChangeMessage(change())).toBe("Built-in runner host turned off.");
  });
});

describe("a pool with nowhere to run", () => {
  it("is shown as unable to run, as an attention state, not left looking healthy", () => {
    expect(isBuiltinHostOff({ state: "builtin-host-off" })).toBe(true);
    expect(isBuiltinHostOff({ state: "active", cannotRun: { code: "builtin_runner_host_disabled", message: "x" } })).toBe(true);
    expect(isBuiltinHostOff({ state: "active", cannotRun: null })).toBe(false);
    expect(statusLabel("builtin-host-off")).toBe(BUILTIN_HOST_OFF_LABEL);
    expect(healthTone("builtin-host-off")).toBe("danger");
  });

  it("is explained on the create page", async () => {
    await render(createElement(BuiltinHostOffNotice));
    expect(document.body.textContent).toContain("The built-in runner host is off");
    expect(document.body.textContent).toContain("Run on machines");
  });
});

const profile = {
  enabled: false, placement: "host_first", mode: "ephemeral", cpuMillis: 2000, memoryMib: 2048, diskBytes: 0,
  platforms: [{ id: "00000001-0000-4000-8000-000000000001", revision: 1, runtimeKind: "docker", executionOs: "linux", architecture: "x64", image: "runner:fixture", requiresDockerSocket: false }],
  targetIds: [],
};

function fleetFetcher() {
  return vi.fn(async (url: string, init?: RequestInit) => {
    if (init?.method === "PUT") return Response.json({ item: { ...JSON.parse(String(init.body)), platforms: profile.platforms, targetIds: [] } });
    return Response.json({ item: profile });
  });
}

describe("pool settings: fleet execution", () => {
  it("offers the built-in placement while the host is on", async () => {
    vi.stubGlobal("fetch", fleetFetcher());
    await render(createElement(FleetExecutionSection, { dockerImage: "runner:fixture", poolId: "pool-1" }));
    await settle();
    expect(document.querySelector('[aria-label="Fleet placement"]')).not.toBeNull();
    expect(switchNamed("Run on machines").disabled).toBe(false);
    expect(switchNamed("Run on machines").getAttribute("aria-checked")).toBe("false");
  });

  it("offers no built-in placement and requires fleet execution while the host is off", async () => {
    const fetcher = fleetFetcher();
    vi.stubGlobal("fetch", fetcher);
    await render(createElement(FleetExecutionSection, { builtinHostEnabled: false, dockerImage: "runner:fixture", poolId: "pool-1" }));
    await settle();
    expect(document.querySelector('[aria-label="Fleet placement"]')).toBeNull();
    expect(document.body.textContent).not.toContain("Wherever there’s room");
    expect(document.body.textContent).toContain("Machines only");
    expect(document.body.textContent).toContain("A pool without fleet execution cannot run");
    const toggle = switchNamed("Run on machines");
    expect(toggle.getAttribute("aria-checked")).toBe("true");
    expect(toggle.disabled).toBe(true);

    // The saved profile said host_first and off; saving sends neither.
    const form = document.getElementById("fleet-profile-form") as HTMLFormElement;
    await act(async () => { form.requestSubmit(); });
    await settle();
    const put = fetcher.mock.calls.find(([, init]) => init?.method === "PUT");
    expect(put).toBeDefined();
    const body = JSON.parse(String(put?.[1]?.body));
    expect(body.placement).toBe("machines");
    expect(body.enabled).toBe(true);
  });
});
