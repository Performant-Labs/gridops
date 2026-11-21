// @vitest-environment jsdom
/**
 * Forward-compat for #141 (#145): a third, test-only descriptor (not GitLab, not registered) shows
 * up in the Targets row and the Platform options section with no change to either component,
 * because both read the injected `descriptors` list.
 */
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { act, createElement, useState } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { createOptions, isDisabled, openPicker, optionIn, pick, platformOptionsToggle, poolFormFetch, settle } from "./fixtures/pool-forms";
import { TooltipProvider } from "../src/components/ui/tooltip";
import { getPlatform, platforms } from "../src/features/platform-connections/registry";
import type { PlatformDescriptor, ResolvedSelection, TargetContext, TargetKind, TargetOption } from "../src/features/platform-connections/types";
import type { RunnerPoolOptions } from "../src/features/runner-pools/runner-pools.functions";
import { PlatformOptionsSection } from "../src/features/runner-pools/platform-options-section";
import { TargetsRow } from "../src/features/runner-pools/targets-row";

vi.mock("@tanstack/react-router", () => ({
  Link: ({ children, to }: { children?: unknown; to?: string }) => createElement("a", { href: String(to) }, children as never),
}));

declare global {
  var IS_REACT_ACT_ENVIRONMENT: boolean | undefined;
}

class NoopResizeObserver { observe() {} unobserve() {} disconnect() {} }

const TEST_HELP = "TestCI jobs need the testci-agent label.";
const TEST_OPTION: TargetOption = {
  key: "testci.project:p1", kindId: "testci.project", connectionId: "c1", nativeId: "p1", label: "Test project one", group: "TestCI · Project",
};

function testDescriptor(load = vi.fn(async () => [TEST_OPTION])): PlatformDescriptor {
  const bitbucket = getPlatform("bitbucket");
  const [workspaceKind] = bitbucket.targetKinds;
  if (!workspaceKind) throw new Error("Bitbucket has no target kind");
  const project: TargetKind = {
    ...workspaceKind,
    id: "testci.project",
    label: "Project",
    group: "TestCI · Project",
    standalone: true,
    requires: [],
    excludes: [],
    requiresProviders: [],
    available: () => true,
    load,
  };
  return {
    ...bitbucket,
    id: "testci",
    name: "TestCI",
    targetKinds: [project],
    platformOptions: {
      help: [TEST_HELP],
      Component: () => createElement("label", null, "Run untagged jobs"),
    },
  };
}

let container: HTMLDivElement;
let root: Root;

beforeEach(() => {
  globalThis.IS_REACT_ACT_ENVIRONMENT = true;
  vi.stubGlobal("ResizeObserver", NoopResizeObserver);
  vi.stubGlobal("fetch", vi.fn(async (url: string, init?: RequestInit) => poolFormFetch(url, init)));
  container = document.createElement("div");
  document.body.append(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  document.body.innerHTML = "";
  vi.unstubAllGlobals();
});

const context: TargetContext = { form: "create", options: createOptions() as unknown as RunnerPoolOptions };

/** The Targets row and the Platform options section sharing one selection, as the forms use them. */
function Harness({ descriptors, onSelection }: { descriptors: readonly PlatformDescriptor[]; onSelection?: (selection: ResolvedSelection) => void }) {
  const [selection, setSelection] = useState<ResolvedSelection>({});
  return createElement("div", null,
    createElement(TargetsRow, { context, descriptors, onSelectionChange: (next: ResolvedSelection) => { setSelection(next); onSelection?.(next); }, selection }),
    createElement(PlatformOptionsSection, { descriptors, selection }),
  );
}

async function render(descriptors: readonly PlatformDescriptor[], onSelection?: (selection: ResolvedSelection) => void) {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  await act(async () => root.render(createElement(QueryClientProvider, { client }, createElement(TooltipProvider, null, createElement(Harness, { descriptors, onSelection })))));
  await settle();
}

describe("a third platform needs only a descriptor", () => {
  it("lists its target kind under its own heading in the Targets picker and reports a pick in the selection", async () => {
    const load = vi.fn(async () => [TEST_OPTION]);
    const onSelection = vi.fn();
    await render([...platforms, testDescriptor(load)], onSelection);

    expect(openPicker("Targets").textContent).toContain("TestCI · Project");
    expect(load).toHaveBeenCalled();
    const option = optionIn("Targets", "Test project one");
    expect(isDisabled(option)).toBe(false);

    await pick("Targets", "Test project one");
    const selection = onSelection.mock.calls.at(-1)?.[0] as ResolvedSelection | undefined;
    expect(selection?.["testci.project"]?.map((selected) => selected.key)).toEqual(["testci.project:p1"]);
  });

  it("renders its platform-options content, and its component slot, once one of its targets is selected", async () => {
    await render([...platforms, testDescriptor()]);
    expect(platformOptionsToggle()).toBeNull();

    await pick("Targets", "Test project one");
    const toggle = platformOptionsToggle();
    expect(toggle?.getAttribute("aria-expanded")).toBe("false");
    act(() => toggle?.click());
    const text = document.body.textContent ?? "";
    expect(text).toContain("TestCI");
    expect(text).toContain(TEST_HELP);
    expect(text).toContain("Run untagged jobs");
  });
});
