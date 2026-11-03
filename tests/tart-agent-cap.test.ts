// @vitest-environment jsdom
/**
 * Settings → General warns, next to "macOS VM slots on the Tart host", when the
 * Tart agent's own runner cap (GRIDOPS_TART_MAX_RUNNERS) is below the slot
 * setting. The lower of the two wins, so without the warning GridOps silently
 * uses fewer slots than it is set to.
 */
import { act, createElement, type ReactElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it } from "vitest";

import { MacosVmSlotsRow, tartAgentCapWarning } from "../src/features/runner-pools/tart-agent-cap";

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
});

async function render(element: ReactElement) {
  await act(async () => root.render(element));
}

describe("tartAgentCapWarning", () => {
  it("names both numbers when the agent allows fewer runners than the slots", () => {
    const warning = tartAgentCapWarning(2, 1);
    expect(warning?.title).toBe("The Tart agent allows 1 runner; slots are set to 2");
    expect(warning?.body).toContain("GridOps uses at most 1 of the Mac's 2 slots");
    expect(warning?.body).toContain("a pool can wait for room on the Tart host while a slot is free");
    expect(warning?.body).toContain("Raise GRIDOPS_TART_MAX_RUNNERS to 2");
  });

  it("says nothing when they agree or the agent allows more", () => {
    expect(tartAgentCapWarning(2, 2)).toBeNull();
    expect(tartAgentCapWarning(1, 1)).toBeNull();
    expect(tartAgentCapWarning(1, 2)).toBeNull();
    expect(tartAgentCapWarning(2, 16)).toBeNull();
  });

  it("says nothing without a Tart agent or a server that predates the setting", () => {
    expect(tartAgentCapWarning(2, null)).toBeNull();
    expect(tartAgentCapWarning(2, undefined)).toBeNull();
    expect(tartAgentCapWarning(undefined, 1)).toBeNull();
  });
});

describe("MacosVmSlotsRow", () => {
  it("shows the warning in the slot setting's row", async () => {
    await render(createElement(MacosVmSlotsRow, { slots: 2, agentMaxRunners: 1 }));
    const input = container.querySelector<HTMLInputElement>("input#macos-vm-slots");
    expect(input?.value).toBe("2");
    expect(container.textContent).toContain("macOS VM slots on the Tart host");
    expect(container.textContent).toContain("The Tart agent allows 1 runner; slots are set to 2");
  });

  it("shows no warning when the agent's cap is not lower", async () => {
    for (const agentMaxRunners of [2, 4, null]) {
      await render(createElement(MacosVmSlotsRow, { slots: 2, agentMaxRunners }));
      expect(container.querySelector("input#macos-vm-slots")).not.toBeNull();
      expect(container.textContent).not.toContain("The Tart agent allows");
    }
  });
});
