/**
 * The GitLab pool fields in the shared runner pool schemas (#141, Bitbucket pattern): they survive
 * the submit-time parse that createRunnerPoolAction / updateRunnerPoolAction send, and the rules
 * mirror #138's server split (`models.rs` validate_gitlab_targets / validate_gitlab_connections):
 * a create-only scope rule, and bounds shared by create and update.
 */
import { describe, expect, it } from "vitest";

import { DOCS_SITE, MOBILE_APP, PLATFORM_GROUP } from "./fixtures/gitlab";
import {
  createRunnerPoolSchema,
  parseCreateRunnerPoolInput,
  parseUpdateRunnerPoolInput,
  updateRunnerPoolSchema,
} from "../src/features/runner-pools/schemas";

const configuration = {
  name: "gitlab-builds",
  mode: "ephemeral" as const,
  provider: "docker" as const,
  providers: ["docker" as const],
  labels: ["build", "iOS 17"],
  image: "runner:latest",
  dockerImage: "runner:latest",
  tartImage: "",
  desiredCount: 1,
  minCount: 0,
  maxCount: 4,
  autoscalingEnabled: true,
  queueScaleFactor: 1,
  idleTimeoutMinutes: 5,
  cpuLimit: 2,
  memoryLimitMb: 4096,
  runnerGroupId: 1,
};

/** What the new-pool form sends for a GitLab-only selection (installation from the pool options). */
const gitlabCreate = {
  ...configuration,
  installationId: 1,
  scope: "gitlab",
  repositoryIds: [],
  bitbucketConnectionIds: [],
  gitlabConnectionIds: [MOBILE_APP.id, PLATFORM_GROUP.id],
  runUntagged: true,
};

const repositoryCreate = { ...configuration, installationId: 1, scope: "repository", repositoryIds: [10] };

const firstError = (result: { success: boolean; error?: { issues: Array<{ message: string; path: PropertyKey[] }> } }) => result.error?.issues[0];

describe("GitLab pool fields survive the submit-time parse", () => {
  it("create keeps scope gitlab, the connection ids and runUntagged", () => {
    const parsed = parseCreateRunnerPoolInput(gitlabCreate);
    expect(parsed).toMatchObject({ scope: "gitlab", gitlabConnectionIds: [MOBILE_APP.id, PLATFORM_GROUP.id], runUntagged: true, installationId: 1 });
  });

  it("create defaults runUntagged to false and a GitHub pool's gitlabConnectionIds to []", () => {
    const { runUntagged: _runUntagged, ...withoutToggle } = gitlabCreate;
    expect(parseCreateRunnerPoolInput(withoutToggle)).toMatchObject({ runUntagged: false });
    expect(parseCreateRunnerPoolInput(repositoryCreate)).toMatchObject({ scope: "repository", gitlabConnectionIds: [], runUntagged: false });
  });

  it("update keeps gitlabConnectionIds and runUntagged when sent, and leaves both out when not", () => {
    const parsed = parseUpdateRunnerPoolInput({ ...configuration, gitlabConnectionIds: [MOBILE_APP.id, DOCS_SITE.id], runUntagged: false });
    expect(parsed).toMatchObject({ gitlabConnectionIds: [MOBILE_APP.id, DOCS_SITE.id], runUntagged: false });
    expect(parseUpdateRunnerPoolInput({ ...configuration, runUntagged: true })).toMatchObject({ runUntagged: true });
    const untouched = parseUpdateRunnerPoolInput(configuration);
    expect(untouched.gitlabConnectionIds).toBeUndefined();
    expect(untouched.runUntagged).toBeUndefined();
  });

  it("GitLab tags share the 64-character label cap", () => {
    expect(createRunnerPoolSchema.safeParse({ ...gitlabCreate, labels: ["t".repeat(64)] }).success).toBe(true);
    expect(createRunnerPoolSchema.safeParse({ ...gitlabCreate, labels: ["t".repeat(65)] }).success).toBe(false);
  });
});

describe("GitLab pool rules (create-only scope rule)", () => {
  it.each([
    ["a GitLab pool with GitHub repositories", { ...gitlabCreate, repositoryIds: [10] }],
    ["a GitLab pool with Bitbucket workspaces", { ...gitlabCreate, bitbucketConnectionIds: ["6a1c1b2e-0000-4000-8000-000000000001"] }],
    ["a gitlab-scoped pool with no GitLab connection", { ...gitlabCreate, gitlabConnectionIds: [] }],
    ["a repository pool with GitLab connections", { ...repositoryCreate, gitlabConnectionIds: [MOBILE_APP.id] }],
    ["an organization pool with GitLab connections", { ...configuration, installationId: 1, scope: "organization", repositoryIds: [], gitlabConnectionIds: [MOBILE_APP.id] }],
  ])("rejects %s", (_name, input) => {
    // A valid GitLab create first, so the rejection below is the rule's and not an unknown scope.
    expect(createRunnerPoolSchema.safeParse(gitlabCreate).success).toBe(true);
    const result = createRunnerPoolSchema.safeParse(input);
    expect(result.success).toBe(false);
  });
});

describe("GitLab connection bounds (create and update)", () => {
  const schemas = [
    ["create", (ids: string[], maxCount = 4) => createRunnerPoolSchema.safeParse({ ...gitlabCreate, gitlabConnectionIds: ids, maxCount })],
    ["update", (ids: string[], maxCount = 4) => updateRunnerPoolSchema.safeParse({ ...configuration, gitlabConnectionIds: ids, maxCount })],
  ] as const;

  it.each(schemas)("%s accepts unique UUIDs up to the maximum runner count", (_name, parse) => {
    const ids = [MOBILE_APP.id, PLATFORM_GROUP.id, DOCS_SITE.id];
    const result = parse(ids, 3);
    expect(result.success).toBe(true);
    expect(result.data?.gitlabConnectionIds).toEqual(ids);
  });

  it.each(schemas)("%s rejects duplicates, more connections than the maximum runner count, and non-UUIDs", (_name, parse) => {
    expect(parse([MOBILE_APP.id, PLATFORM_GROUP.id]).success, "the valid baseline").toBe(true);
    const duplicate = parse([MOBILE_APP.id, MOBILE_APP.id]);
    expect(duplicate.success).toBe(false);
    expect(firstError(duplicate)?.path).toEqual(["gitlabConnectionIds"]);
    const tooMany = parse([MOBILE_APP.id, PLATFORM_GROUP.id, DOCS_SITE.id], 2);
    expect(tooMany.success).toBe(false);
    expect(firstError(tooMany)?.path).toEqual(["gitlabConnectionIds"]);
    expect(parse(["not-a-uuid"]).success).toBe(false);
  });
});
