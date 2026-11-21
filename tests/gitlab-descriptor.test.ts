/**
 * The GitLab platform descriptor (#141), read through the production registry: the connection
 * form (fields, defaults, client validation), the connection API calls its bindings make, the two
 * target kinds and where their options come from, the pool payload partials (registry order
 * included), the settings-form selection and summary of a GitLab pool, the tag rule and the runner
 * link. Pure functions plus a stubbed `fetch`; the rendered surfaces are in gitlab-connections,
 * gitlab-pool-forms and webhook-delivery-platform tests.
 */
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { DOCS_SITE, GITLAB_CONNECTIONS, GITLAB_URL, MOBILE_APP, PLATFORM_GROUP, gitlabCreateOptions, gitlabPool } from "./fixtures/gitlab";
import { createOptions, settingsPool } from "./fixtures/pool-forms";
import type { Runner } from "../src/features/operations/operations.functions";
import { getPlatform, platforms } from "../src/features/platform-connections/registry";
import type { ConnectionField, ResolvedSelection, TargetContext, TargetKind, TargetOption } from "../src/features/platform-connections/types";
import type { RunnerPoolDetail, RunnerPoolOptions } from "../src/features/runner-pools/runner-pools.functions";
import * as schemas from "../src/features/runner-pools/schemas";
import * as targets from "../src/features/runner-pools/targets";

const PROJECT = "gitlab.project";
const GROUP = "gitlab.group";
const DEFAULT_ADDRESS = "https://gitlab.com";

const gitlab = () => getPlatform("gitlab");
/**
 * Precondition of the tests that check GitLab adds nothing (they would also hold for the unknown-
 * platform fallback): the descriptor under test is the registered one.
 */
const expectGitlabRegistered = () => expect(platforms.map((platform) => platform.id), "GitLab is registered").toContain("gitlab");
const createContext = (options: unknown = gitlabCreateOptions()): TargetContext => ({ form: "create", options: options as RunnerPoolOptions });
const updateContext = (pool: unknown): TargetContext => ({ form: "update", pool: pool as RunnerPoolDetail });

function kind(id: string): TargetKind {
  const found = platforms.flatMap((platform) => platform.targetKinds).find((candidate) => candidate.id === id);
  if (!found) throw new Error(`No registered target kind "${id}"`);
  return found;
}

function field(label: string): ConnectionField {
  const found = (gitlab().connections.fields ?? []).find((candidate) => candidate.label === label);
  if (!found) throw new Error(`The GitLab connection form has no field labelled "${label}"`);
  return found;
}

function selectField(): ConnectionField {
  const found = (gitlab().connections.fields ?? []).find((candidate) => candidate.type === "select");
  if (!found) throw new Error("The GitLab connection form has no select field (Project / Group)");
  return found;
}

/** The create form's values as the dialog submits them: each field's default, then `byLabel`. */
function formValues(byLabel: Record<string, string>, kindValue?: string): Record<string, string> {
  const values = Object.fromEntries((gitlab().connections.fields ?? []).map((candidate) => [candidate.key, candidate.defaultValue ?? ""]));
  for (const [label, value] of Object.entries(byLabel)) values[field(label).key] = value;
  if (kindValue !== undefined) values[selectField().key] = kindValue;
  return values;
}

/** The selection the Targets row reports after picking `connections` from a kind's loaded options. */
async function pickFrom(context: TargetContext, kindId: string, connections: ReadonlyArray<{ id: string }>): Promise<TargetOption[]> {
  const options = await kind(kindId).load(context, new AbortController().signal);
  return connections.map((connection) => {
    const option = options.find((candidate) => candidate.connectionId === connection.id || candidate.nativeId === connection.id);
    if (!option) throw new Error(`${kindId} offers no option for connection ${connection.id}`);
    return option;
  });
}

let fetcher: ReturnType<typeof vi.fn>;
const requests = () => (fetcher.mock.calls as Array<[string, RequestInit | undefined]>).map(([url, init]) => ({
  url,
  method: init?.method ?? "GET",
  body: init?.body === undefined ? undefined : JSON.parse(String(init.body)) as Record<string, unknown>,
}));

beforeEach(() => {
  fetcher = vi.fn(async (url: string, init?: RequestInit) => {
    const method = init?.method ?? "GET";
    if (method === "GET" && url === GITLAB_URL) return Response.json({ canManage: true, items: GITLAB_CONNECTIONS });
    if (method === "POST" && url === GITLAB_URL) {
      return Response.json({ connection: MOBILE_APP, webhookUrl: "https://gridops.example/api/v1/webhooks/gitlab/a1", webhookSecret: "glwh-secret-1" }, { status: 201 });
    }
    if (method === "PATCH" && url === `${GITLAB_URL}/${MOBILE_APP.id}`) return Response.json(MOBILE_APP);
    if (method === "DELETE" && url === `${GITLAB_URL}/${MOBILE_APP.id}`) return new Response(null, { status: 204 });
    return Response.json({ error: `not in this test: ${method} ${url}` }, { status: 404 });
  });
  vi.stubGlobal("fetch", fetcher);
});

afterEach(() => {
  vi.unstubAllGlobals();
});

describe("GitLab connection form", () => {
  it("asks for name, address, Project/Group, path and token in that order, with the address and kind pre-filled", () => {
    const fields = gitlab().connections.fields ?? [];
    expect(fields.map((candidate) => candidate.type === "select" ? "<kind>" : candidate.label))
      .toEqual(["Connection name", "GitLab address", "<kind>", "Project or group path", "Access token"]);
    expect(field("GitLab address").defaultValue).toBe(DEFAULT_ADDRESS);
    const scopeKind = selectField();
    expect(scopeKind.options?.map((option) => option.value)).toEqual(["project", "group"]);
    expect(scopeKind.defaultValue).toBe("project");
    expect(field("Access token").type).toBe("secret");
  });

  it("names the create_runner and read_api scopes and the Owner role in the token help", () => {
    const help = String(field("Access token").help);
    expect(help).toContain("create_runner");
    expect(help).toContain("read_api");
    expect(help).toContain("Owner");
  });

  it("edits only the name and the token: address, kind and path are fixed at create", () => {
    const editable = (gitlab().connections.fields ?? []).filter((candidate) => candidate.editableOnUpdate);
    expect(editable.map((candidate) => candidate.label)).toEqual(["Connection name", "Access token"]);
  });

  it.each([
    ["https://gitlab.com", true],
    ["https://git.example.com", true],
    ["http://gitlab.internal:8080/gitlab", true],
    ["https://gitlab.com/", true],
    ["gitlab.com", false],
    ["ftp://gitlab.com", false],
    ["https://gitlab.com?x=1", false],
    ["https://gitlab.com/#top", false],
    ["not a url", false],
  ] as const)("address %s is %s", (value, valid) => {
    const validate = field("GitLab address").validate;
    expect(validate, "the address field has a validate hook").toBeTypeOf("function");
    const error = validate?.(value) ?? null;
    if (valid) expect(error).toBeNull();
    else expect(error).toEqual(expect.stringMatching(/\S/));
  });

  it.each([
    ["acme", true],
    ["acme/platform/mobile-app", true],
    ["acme.io/web_app", true],
    ["", false],
    ["/acme", false],
    ["acme/", false],
    ["acme//api", false],
    ["acme/../api", false],
    ["acme team/api", false],
    ["acme/\tapi", false],
  ] as const)("scope path %j is %s", (value, valid) => {
    const validate = field("Project or group path").validate;
    expect(validate, "the path field has a validate hook").toBeTypeOf("function");
    const error = validate?.(value) ?? null;
    if (valid) expect(error).toBeNull();
    else expect(error).toEqual(expect.stringMatching(/\S/));
  });
});

describe("GitLab connection API calls (through the descriptor's bindings)", () => {
  it("lists with GET /api/v1/platform-connections/gitlab, as a fetching list", async () => {
    const list = gitlab().connections.list;
    expect(list.kind).toBe("fetch");
    if (list.kind !== "fetch") return;
    const result = await list.load(new AbortController().signal);
    expect(result.items).toHaveLength(GITLAB_CONNECTIONS.length);
    expect(requests()).toEqual([{ url: GITLAB_URL, method: "GET", body: undefined }]);
  });

  it("creates with POST and the spec body, dropping a trailing slash from the address", async () => {
    const create = gitlab().connections.create;
    expect(create).toBeTypeOf("function");
    await create?.(formValues({
      "Connection name": "Platform group", "GitLab address": "https://git.example.com/", "Project or group path": "platform", "Access token": "glpat-abc",
    }, "group"));
    expect(requests()).toEqual([{
      url: GITLAB_URL,
      method: "POST",
      body: { name: "Platform group", baseUrl: "https://git.example.com", scopeKind: "group", scopePath: "platform", accessToken: "glpat-abc" },
    }]);
  });

  it("sends the default address https://gitlab.com when the address is left blank", async () => {
    await gitlab().connections.create?.(formValues({
      "Connection name": "Mobile app", "GitLab address": "", "Project or group path": "acme/platform/mobile-app", "Access token": "glpat-abc",
    }));
    const body = requests()[0]?.body;
    expect(body?.baseUrl ?? DEFAULT_ADDRESS).toBe(DEFAULT_ADDRESS);
    expect(body).toMatchObject({ scopeKind: "project", scopePath: "acme/platform/mobile-app" });
  });

  it("reveals the created connection's webhook secret once, from the 201 body", async () => {
    const created = await gitlab().connections.create?.(formValues({
      "Connection name": "Mobile app", "Project or group path": "acme/platform/mobile-app", "Access token": "glpat-abc",
    }));
    const reveal = await gitlab().postCreate?.(created);
    expect(reveal?.value).toBe("glwh-secret-1");
  });

  it("renames with PATCH {name}, and adds accessToken only when a new token is typed", async () => {
    const { update, editValues } = gitlab().connections;
    expect(update).toBeTypeOf("function");
    const values = editValues?.(MOBILE_APP) ?? {};
    expect(values[field("Connection name").key]).toBe("Mobile app");
    expect(values[field("Access token").key] ?? "").toBe("");

    await update?.(MOBILE_APP, { ...values, [field("Connection name").key]: "Mobile apps" });
    await update?.(MOBILE_APP, { ...values, [field("Connection name").key]: "Mobile apps", [field("Access token").key]: "glpat-new" });
    expect(requests()).toEqual([
      { url: `${GITLAB_URL}/${MOBILE_APP.id}`, method: "PATCH", body: { name: "Mobile apps" } },
      { url: `${GITLAB_URL}/${MOBILE_APP.id}`, method: "PATCH", body: { name: "Mobile apps", accessToken: "glpat-new" } },
    ]);
  });

  it("removes with DELETE", async () => {
    await gitlab().connections.remove?.(MOBILE_APP);
    expect(requests()).toEqual([{ url: `${GITLAB_URL}/${MOBILE_APP.id}`, method: "DELETE", body: undefined }]);
  });
});

describe("GitLab target kinds", () => {
  it("adds a standalone, many-per-pool Project kind and Group kind, with no provider requirement", () => {
    expect(gitlab().targetKinds.map((candidate) => candidate.id)).toEqual([PROJECT, GROUP]);
    for (const id of [PROJECT, GROUP]) {
      expect(kind(id)).toMatchObject({ standalone: true, cardinality: "many" });
      expect(kind(id).requiresProviders ?? []).toEqual([]);
    }
    expect(kind(PROJECT).group).toBe("GitLab · Project");
    expect(kind(GROUP).group).toBe("GitLab · Group");
  });

  it("on a new pool, offers one option per connection of the kind's scope, from the pool options, described as <baseUrl>/<scopePath>", async () => {
    const context = createContext(gitlabCreateOptions([MOBILE_APP, PLATFORM_GROUP, DOCS_SITE]));
    const projects = await kind(PROJECT).load(context, new AbortController().signal);
    const groups = await kind(GROUP).load(context, new AbortController().signal);
    expect(projects.map((option) => [option.label, option.description])).toEqual([
      ["Mobile app", "https://gitlab.com/acme/platform/mobile-app"],
      ["Docs site", "https://gitlab.com/acme/docs"],
    ]);
    expect(groups.map((option) => [option.label, option.description])).toEqual([["Platform group", "https://git.example.com/platform"]]);
    expect(projects.every((option) => option.key.startsWith(`${PROJECT}:`))).toBe(true);
    expect(fetcher).not.toHaveBeenCalled();
  });

  it("on a new pool, a kind is available only while its scope kind has a connection (none on a server without the field)", () => {
    expect(kind(PROJECT).available(createContext(gitlabCreateOptions([MOBILE_APP])))).toBe(true);
    expect(kind(GROUP).available(createContext(gitlabCreateOptions([MOBILE_APP])))).toBe(false);
    expect(kind(GROUP).available(createContext(gitlabCreateOptions([PLATFORM_GROUP])))).toBe(true);
    expect(kind(PROJECT).available(createContext(createOptions()))).toBe(false);
    expect(kind(GROUP).available(createContext(createOptions()))).toBe(false);
  });

  it.each([
    ["a new pool", true, true, createContext()],
    ["a GitLab pool's settings", true, true, updateContext(gitlabPool())],
    ["a repository pool's settings", false, false, updateContext(settingsPool({ scope: "repository", repositoryIds: [10] }))],
    ["an organization pool's settings", false, false, updateContext(settingsPool({ scope: "organization" }))],
  ] as const)("on %s, the GitLab kinds are offered: %s, available: %s", (_name, offered, available, context) => {
    for (const id of [PROJECT, GROUP]) {
      expect(targets.isOffered(kind(id), context)).toBe(offered);
      if (context.form === "update") expect(kind(id).available(context)).toBe(available);
    }
  });

  it("on a GitLab pool's settings, fetches every connection so one the pool does not use can be added, with the keys of the pool's selection", async () => {
    const pool = gitlabPool();
    const context = updateContext(pool);
    const projects = await kind(PROJECT).load(context, new AbortController().signal);
    const groups = await kind(GROUP).load(context, new AbortController().signal);
    expect(requests().filter((request) => request.url === GITLAB_URL && request.method === "GET").length).toBeGreaterThan(0);
    expect(projects.map((option) => option.label)).toEqual(["Mobile app", "Docs site"]);
    expect(groups.map((option) => option.label)).toEqual(["Platform group"]);

    const selected = targets.initialSelection(platforms, pool as unknown as RunnerPoolDetail);
    const loadedKeys = new Set([...projects, ...groups].map((option) => option.key));
    for (const option of [...(selected[PROJECT] ?? []), ...(selected[GROUP] ?? [])]) expect(loadedKeys.has(option.key)).toBe(true);
  });

  it("cannot be combined with any GitHub or Bitbucket target, in either direction; a project and a group can be", async () => {
    const context = createContext();
    const [project] = await pickFrom(context, PROJECT, [MOBILE_APP]);
    const withProject: ResolvedSelection = { [PROJECT]: [project as TargetOption] };
    for (const id of ["github.repository", "github.runner-group", "bitbucket.workspace"]) {
      expect(targets.unavailableReason(platforms, withProject, kind(id)), id).toEqual(expect.stringMatching(/\S/));
    }
    expect(targets.unavailableReason(platforms, withProject, kind(GROUP))).toBeNull();

    const repository: TargetOption = { key: "github.repository:10", kindId: "github.repository", connectionId: "1", nativeId: "10", label: "octo-org/app", group: "GitHub · Repository" };
    const runnerGroup: TargetOption = { key: "github.runner-group:2/7", kindId: "github.runner-group", connectionId: "2", nativeId: "2/7", label: "acme · Release", group: "GitHub · Organization runner group" };
    for (const selection of [{ "github.repository": [repository] }, { "github.runner-group": [runnerGroup] }]) {
      for (const id of [PROJECT, GROUP]) expect(targets.unavailableReason(platforms, selection, kind(id)), id).toEqual(expect.stringMatching(/\S/));
    }
  });
});

describe("GitLab pool payloads (production registry, registry order)", () => {
  /** The forms' platform option values (gap 2), seeded as a pool (or a new pool) starts them. */
  const optionValues = (pool?: unknown) => {
    expect(targets.initialPlatformOptionValues, "targets.ts exports initialPlatformOptionValues").toBeTypeOf("function");
    return targets.initialPlatformOptionValues(platforms, pool as RunnerPoolDetail | undefined);
  };
  const createValues = (pool?: unknown) => ({ platformOptions: optionValues(pool) });

  it.each([
    ["project only", [MOBILE_APP], []],
    ["group only", [], [PLATFORM_GROUP]],
    ["a project and a group", [MOBILE_APP], [PLATFORM_GROUP]],
  ] as const)("a GitLab-only selection (%s) creates a gitlab-scoped pool on the first installation, with no repositories and runner group 1", async (_name, projectConnections, groupConnections) => {
    const context = createContext();
    const selection: ResolvedSelection = {
      ...(projectConnections.length ? { [PROJECT]: await pickFrom(context, PROJECT, projectConnections) } : {}),
      ...(groupConnections.length ? { [GROUP]: await pickFrom(context, GROUP, groupConnections) } : {}),
    };
    const payload = targets.buildCreatePayload(platforms, selection, context, createValues());
    expect(payload).toMatchObject({
      scope: "gitlab",
      installationId: 1,
      repositoryIds: [],
      runnerGroupId: 1,
      gitlabConnectionIds: [...projectConnections, ...groupConnections].map((connection) => connection.id),
      runUntagged: false,
    });
  });

  it("sends runUntagged from the platform option values", async () => {
    const context = createContext();
    const selection = { [PROJECT]: await pickFrom(context, PROJECT, [MOBILE_APP]) };
    const on = targets.buildCreatePayload(platforms, selection, context, createValues(gitlabPool({ runUntagged: true })));
    expect(on.runUntagged).toBe(true);
  });

  it("with no GitLab target, adds no defined field to the payload, so GitHub's partial stands", () => {
    expectGitlabRegistered();
    const repository: TargetOption = { key: "github.repository:10", kindId: "github.repository", connectionId: "1", nativeId: "10", label: "octo-org/app", group: "GitHub · Repository", loadIndex: 0 };
    const selection = { "github.repository": [repository] };
    const context = createContext();
    const partial = gitlab().toCreatePayload(selection, context, createValues());
    expect(JSON.parse(JSON.stringify(partial))).toEqual({});
    expect(targets.buildCreatePayload(platforms, selection, context, createValues())).toMatchObject({ scope: "repository", installationId: 1, repositoryIds: [10], runnerGroupId: 1 });
  });

  it("a GitLab pool's settings save the full connection set and a changed runUntagged, and no GitHub repository, priority or runner-group field", async () => {
    const pool = gitlabPool({ runUntagged: false });
    // The switch turned on: the option values a pool with it on starts from.
    const switchedOn = optionValues(gitlabPool({ runUntagged: true }));
    const context = updateContext(pool);
    const selection: ResolvedSelection = {
      ...targets.initialSelection(platforms, pool as unknown as RunnerPoolDetail),
      [PROJECT]: await pickFrom(context, PROJECT, [MOBILE_APP, DOCS_SITE]),
    };
    const payload = targets.buildUpdatePayload(platforms, selection, pool as unknown as RunnerPoolDetail, { priorityKeys: [], platformOptions: switchedOn });
    expect([...(payload.gitlabConnectionIds as string[])].sort()).toEqual([MOBILE_APP.id, PLATFORM_GROUP.id, DOCS_SITE.id].sort());
    expect(payload.runUntagged).toBe(true);
    for (const key of ["repositoryIds", "priorityRepositoryIds", "priorityWaitMinutes", "runnerGroupId", "bitbucketConnectionIds"]) {
      expect(payload[key], key).toBeUndefined();
    }
  });

  it.each([
    ["repository", settingsPool({ scope: "repository", repositoryIds: [10] })],
    ["organization", settingsPool({ scope: "organization" })],
  ] as const)("a %s pool's settings never send gitlabConnectionIds or runUntagged", (_scope, pool) => {
    expectGitlabRegistered();
    const detail = pool as unknown as RunnerPoolDetail;
    const payload = targets.buildUpdatePayload(platforms, targets.initialSelection(platforms, detail), detail, {
      priorityKeys: [],
      platformOptions: optionValues(detail),
    });
    expect(payload.gitlabConnectionIds).toBeUndefined();
    expect(payload.runUntagged).toBeUndefined();
  });

  it("a GitLab pool's settings start from its GitLab connections only, list only GitLab read-only, and prioritise nothing", () => {
    const pool = gitlabPool() as unknown as RunnerPoolDetail;
    const selection = targets.initialSelection(platforms, pool);
    expect(Object.keys(selection).filter((id) => (selection[id]?.length ?? 0) > 0).sort()).toEqual([GROUP, PROJECT]);
    expect(selection[PROJECT]?.map((option) => option.label)).toEqual(["Mobile app"]);
    expect(selection[GROUP]?.map((option) => option.label)).toEqual(["Platform group"]);
    expect(targets.readOnlySummaries(platforms, pool).map(({ platform, value }) => [platform.id, value])).toEqual([["gitlab", "Mobile app, Platform group"]]);
    expect(targets.initialPriorityKeys(platforms, pool)).toEqual([]);
  });

  it("a pool detail from a server without the GitLab fields still opens (no GitLab selection or summary)", () => {
    expectGitlabRegistered();
    const pool = settingsPool({ scope: "repository", repositoryIds: [10] }) as unknown as RunnerPoolDetail;
    expect(gitlab().initialSelection?.(pool) ?? {}).toEqual({});
    expect(gitlab().readOnlySummary?.(pool) ?? null).toBeNull();
  });
});

describe("GitLab tags", () => {
  it("accept any character but a comma, and must not be empty; the descriptor and the schema share the rule", () => {
    const validate = gitlab().labelRules?.validate;
    expect(validate, "GitLab declares a label rule").toBeTypeOf("function");
    expect(schemas.gitlabLabelError, "schemas.ts exports gitlabLabelError").toBeTypeOf("function");
    for (const check of [validate, schemas.gitlabLabelError]) {
      expect(check?.(["build", "iOS 17", "release/1.2", "Docker_x64"])).toBeNull();
      expect(check?.(["build", "a,b"])).toEqual(expect.stringMatching(/\S/));
      expect(check?.([""])).toEqual(expect.stringMatching(/\S/));
      expect(check?.(["   "])).toEqual(expect.stringMatching(/\S/));
    }
  });
});

describe("GitLab runner link", () => {
  const runner = (overrides: Record<string, unknown>): Runner => ({
    id: "r-1", name: "gitlab-runner-1", provider: "docker", platform: "gitlab", status: "idle", busy: false, ephemeral: true, os: "linux",
    architecture: "X64", containerId: null, githubRunnerId: null, bitbucketRunnerUuid: null, failureReason: null, registeredAt: null,
    lastHeartbeatAt: null, createdAt: "2026-11-20T20:00:00.000Z", poolId: "pool-1", poolName: "gitlab-builds", poolPaused: false,
    accountLogin: "octo-org", repository: null, currentJobName: null, currentRunId: null, machine: null, canManage: false,
    gitlabRunnerId: 42, gitlabBaseUrl: "https://gitlab.com", gitlabScopePath: "acme/platform/mobile-app",
    ...overrides,
  } as unknown as Runner);

  it("links <baseUrl>/<scopePath>/-/runners/<id>", () => {
    expect(gitlab().runnerLink?.(runner({}))?.href).toBe("https://gitlab.com/acme/platform/mobile-app/-/runners/42");
    expect(gitlab().runnerLink?.(runner({ gitlabRunnerId: 7, gitlabBaseUrl: "https://git.example.com", gitlabScopePath: "platform" }))?.href)
      .toBe("https://git.example.com/platform/-/runners/7");
  });

  it.each(["gitlabRunnerId", "gitlabBaseUrl", "gitlabScopePath"])("links nothing when %s is missing or null", (missing) => {
    expect(typeof gitlab().runnerLink).toBe("function");
    expect(gitlab().runnerLink?.(runner({ [missing]: null }))).toBeNull();
    expect(gitlab().runnerLink?.(runner({ [missing]: undefined }))).toBeNull();
  });
});
