/**
 * The pure Targets rules in src/features/runner-pools/targets.ts (#145), run with the production
 * registry: the plan-preview target check, the create-button gate's standalone-target test, and
 * the descriptors' create partials for an empty selection.
 */
import { describe, expect, it } from "vitest";

import { platforms } from "../src/features/platform-connections/registry";
import type { ResolvedSelection, TargetContext, TargetOption } from "../src/features/platform-connections/types";
import type { RunnerPoolOptions } from "../src/features/runner-pools/runner-pools.functions";
import { anyStandaloneKindAvailable, targetPlanCheck } from "../src/features/runner-pools/targets";

const repository = (id: number, installationId = 1): TargetOption => ({
  key: `github.repository:${id}`, kindId: "github.repository", connectionId: String(installationId), nativeId: String(id),
  label: `octo-org/repo-${id}`, group: "GitHub · Repository",
});
const runnerGroup: TargetOption = {
  key: "github.runner-group:2/7", kindId: "github.runner-group", connectionId: "2", nativeId: "2/7", label: "acme · Release", group: "GitHub · Organization runner group",
};
const workspace = (id: string): TargetOption => ({
  key: `bitbucket.workspace:${id}`, kindId: "bitbucket.workspace", connectionId: id, nativeId: id, label: `Workspace ${id}`, group: "Bitbucket · Workspace",
});

const RUNNER_GROUP_SENTENCE = "GitHub runner-group access defines which repositories can use the pool.";

function createContext(installations: number, bitbucketConnections: number): TargetContext {
  const options = {
    authenticated: true,
    installUrl: "https://github.com/apps/gridops/installations/new",
    installations: Array.from({ length: installations }, (_, index) => ({ id: index + 1, accountLogin: `org-${index + 1}`, accountType: "Organization" })),
    repositories: [],
    runnerGroups: [],
    bitbucketConnections: Array.from({ length: bitbucketConnections }, (_, index) => ({ id: `bb-${index}`, name: `Workspace ${index}`, workspace: `ws-${index}`, workspaceUuid: `{ws-${index}}` })),
    defaults: {
      provider: "docker", providers: ["docker"], image: "runner:default", dockerImage: "runner:default", tartImage: "", macosRuntime: "vm",
      labels: [], cpuLimit: 2, memoryLimitMb: 2048, desiredCount: 0, minCount: 0, maxCount: 10, autoscalingEnabled: true,
      queueScaleFactor: 1, idleTimeoutMinutes: 5, runnerGroupId: 1, maxCpuLimit: 16, maxMemoryLimitMb: 65_536,
    },
  } satisfies RunnerPoolOptions;
  return { form: "create", options };
}

describe("targetPlanCheck: the plan preview's target sentence", () => {
  it("speaks for repositories on an empty form (the form starts as a repository pool)", () => {
    expect(targetPlanCheck(platforms, {}, { maxCount: 10 })).toEqual({ ok: true, text: "0 selected repositories fit within the 10-runner maximum." });
  });

  it("counts selected repositories against the maximum", () => {
    expect(targetPlanCheck(platforms, { "github.repository": [repository(1), repository(2)] }, { maxCount: 10 }))
      .toEqual({ ok: true, text: "2 selected repositories fit within the 10-runner maximum." });
  });

  it("fails when more repositories are selected than the maximum", () => {
    expect(targetPlanCheck(platforms, { "github.repository": [repository(1), repository(2), repository(3)] }, { maxCount: 2 }))
      .toEqual({ ok: false, text: "3 selected repositories fit within the 2-runner maximum." });
  });

  it("uses the runner-group sentence for an organization runner group", () => {
    expect(targetPlanCheck(platforms, { "github.runner-group": [runnerGroup] }, { maxCount: 1 })).toEqual({ ok: true, text: RUNNER_GROUP_SENTENCE });
  });

  it("ignores Bitbucket workspaces, as the preview does today", () => {
    const selection: ResolvedSelection = { "github.repository": [repository(1)], "bitbucket.workspace": [workspace("a"), workspace("b"), workspace("c")] };
    expect(targetPlanCheck(platforms, selection, { maxCount: 2 })).toEqual({ ok: true, text: "1 selected repositories fit within the 2-runner maximum." });
    expect(targetPlanCheck(platforms, { "bitbucket.workspace": [workspace("a")] }, { maxCount: 2 }))
      .toEqual({ ok: true, text: "0 selected repositories fit within the 2-runner maximum." });
  });
});

describe("anyStandaloneKindAvailable: the create-button gate (today: installations.length > 0)", () => {
  it.each([
    ["(a) 0 installations, 0 Bitbucket connections", 0, 0, false],
    ["(b) 0 installations, Bitbucket connections", 0, 2, false],
    ["(c) an installation, 0 Bitbucket connections", 1, 0, true],
    ["(d) installations and Bitbucket connections", 2, 1, true],
  ])("%s", (_label, installations, bitbucketConnections, expected) => {
    expect(anyStandaloneKindAvailable(platforms, createContext(installations, bitbucketConnections))).toBe(expected);
  });
});

describe("descriptor create partials are total", () => {
  it("merge to today's empty-selection payload fields without throwing", () => {
    const context = createContext(1, 1);
    const merged = Object.assign({}, ...platforms.map((platform) => platform.toCreatePayload({}, context)));
    expect(merged).toEqual({ installationId: 0, scope: "repository", repositoryIds: [], runnerGroupId: 1, bitbucketConnectionIds: [] });
  });
});
