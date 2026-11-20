# Handoff-T-red: Phase 4 (test-first RED) - One Connections page and one Targets picker for every CI platform (#145)

**Date:** 2026-11-20 (1:40 PM PST)
**Branch:** claude/ui-145-platform-neutral-cf5832 (tip `ef65745`)
**Brief / wireframe reviewed:** `docs/handoffs/145-brief.md` (incl. the binding "Round-3 architecture decisions"), `docs/handoffs/145/wireframe.html` (revision 4), `docs/handoffs/145/handoff-D.md`, `docs/handoffs/145/handoff-A.md` (round 4), `docs/handoffs/145/decisions.md`

## A precondition
Confirmed: A returned **PASS** on the plan (`handoff-A.md`, round 4: 0 blocks, 5 warns). I applied A's warns where they touch tests: finding 1 (two orders) is pinned by a frozen payload case clicked in reverse list order; finding 2 (only the pool's own kind on settings) and finding 3 (manual runner-group entry per installation) each have a test.

## Starting state (Sequencing gate, step 1)
- `git diff 5550cf5 HEAD --stat -- src tests` at tip `ef65745`: **empty** (only `docs/handoffs/` changed since `5550cf5`).
- `git diff 5550cf5 --stat -- src` on the working tree after my work: **empty**. I changed nothing under `src/`. The only file there that is not tracked is the gitignored, generated `src/routeTree.gen.ts`, which `vite build` creates so that `tsc` can run.
- `npm ci` then `npx vitest run` before any change: **37 files, 364 tests, all passed.**

## Tests authored

All files are new except `tests/runner-pool-tart-image.test.ts`. The brief names that file under "Tests known to break", so I updated it.

| File | Tier | What it pins (criterion) | Why this tier |
|---|---|---|---|
| `tests/platform-registry.test.ts` | unit | Registry lookups. Fallback for an unknown id: its name is the raw id and it has no targets. Every descriptor is well-formed: namespaced, unique kind ids; label, group, `load` and `available` present; `requires`/`excludes` only point at registered kinds; a non-standalone kind requires a standalone one. Today's kind rules: repository ⟂ runner group, runner group `cardinality: "one"`, a workspace is not standalone, needs a GitHub kind and `requiresProviders: ["tart"]`. Bitbucket `labelRules.validate` agrees with the schema rule. Plus the `expectTypeOf<PostCreateReveal>()` assertion, written exactly as the brief gives it (criterion "types.ts exports PostCreateReveal"). | These are pure data and function contracts. |
| `tests/pool-targets-rules.test.ts` | unit | `targetPlanCheck` covers the empty form (repository sentence, count 0), repositories within and over the cap, a runner group, and workspaces having no effect. `anyStandaloneKindAvailable` covers gate cases (a)-(d). Descriptor create partials are total and merge, for an empty selection, to today's `{ installationId: 0, scope: "repository", repositoryIds: [], runnerGroupId: 1, bitbucketConnectionIds: [] }`. | Pure functions in `targets.ts`; the brief names them. |
| `tests/pool-target-payloads.test.ts` | integration (rendered route forms) | **Payload equality.** It has 5 frozen constants plus 1 extra (manual runner-group ID, A finding 3). The repository-create case uses two installations clicked in reverse list order, so it pins both the click order of `repositoryIds` and the list-order `installationId` (A finding 1). It drives the forms through `Targets`, records `data` at `createRunnerPoolAction`/`updateRunnerPoolAction` by wrapping them with `vi.mock`, and compares with `toEqual`. | The brief requires the rendered form, not the builder. |
| `tests/pool-targets-form.test.ts` | integration (rendered route forms, production registry) | Mixing rules (a)-(e). The Maximum floor counts repositories only (`raisesMinMaxCount`). Create-button gate: (c)-(e) enabled, (f) disabled while submitting, and (a)/(b) still show today's "Install the GitHub App" state. Platform options: absent with no target, collapsed by default, follows the selection. Pool settings offers only the pool's own kind, shows Bitbucket chips locked and keeps Priority only for repository pools (A finding 2). The read-only pool lists targets per platform. | These are the user-visible form behaviours. |
| `tests/targets-row-forward-compat.test.ts` | integration (components) | The forward-compat claim. A test-only third descriptor, injected through `descriptors`, shows its group heading and options in `TargetsRow` and reports a pick through `onSelectionChange`. Its `platformOptions` help and `Component` slot then render in `PlatformOptionsSection`. | The brief requires direct rendering with an injected list. |
| `tests/connections-page.test.ts` | integration (components + route) | One list holds GitHub installation rows (page-derived: Configure link, "Owner approval required", Suspended) and Bitbucket rows. Permission gating covers Add connection, Edit and Remove. Bitbucket flows: create (the error stays inline, a retry connects, only Bitbucket reloads), edit (only editable fields; a blank token is not sent) and remove (today's confirm). Partial load failure, injected case: per-binding call counts before and after Retry. Production failure path: a stubbed 502, and Retry makes exactly one more request. The post-create reveal shows the value once, ignores Escape and drops the value after close; a `null` hook shows no dialog. `GitHubAppSection` covers trace items (a)-(d), and members get no Create section and no Manage App. The route shows exactly one Read-only callout to members (A finding 4a). | Component behaviour, with `fetch` stubbed at the `api()` boundary. |
| `tests/connections-routes.test.ts` | integration | `/settings/integrations`, `/settings/github` and `/platform-connections` are redirect-only: each replace-redirects to `/settings/connections` and has no component and no loader. The `?appError=` cases are as specified. Rendered hrefs: the settings nav has a single Connections item, the webhooks empty state has "GitHub settings" pointing to `/settings/connections`, and the new-pool form's connect-a-workspace link points to `/settings/connections`. | It uses the router's real `redirect`; the links are checked as rendered. |
| `tests/platform-neutral-source.test.ts` | unit (source scan) | The brief's grep criteria, encoded so that `npm test` enforces them: the redirect-string rule with its exact whole-line exclusion regex and the declaration-line invariant; `appError` is read only in `settings.index.tsx`; the two pool routes contain no quoted platform or kind id, `pool.scope`, `scope ===` or `bitbucketConnectionIds.length`; `tart-base-vm-row.tsx` and `pool-plan-preview.tsx` contain no platform text and no `scope`; the three generic files contain no `github`/`bitbucket`; `runner-row.tsx` has no platform ternary and imports the registry; the Bitbucket label regex appears only in `schemas.ts`. | The criteria are stated as greps; as tests they cannot regress silently. |
| `tests/runner-row-platform.test.ts` | integration (component) | The runner row shows the registry name for `github`, `bitbucket` and an unknown id (`gitlab`, the raw id). | The badge is what renders. |
| `tests/runner-pool-tart-image.test.ts` (updated) | integration | `submitCreate` now picks the repository through `Targets`. The Bitbucket case picks a GitHub target, then the workspace, through `Targets`. It asserts that the Bitbucket script is **not** in the Tart base VM row and **is** in Platform options once that section is expanded (it is collapsed by default). The other cases are unchanged apart from a `TooltipProvider` wrapper, which mirrors `__root.tsx`. | These are the "tests known to break". |
| `tests/fixtures/pool-forms.ts` (new, not collected) | - | Fixtures that match the capture, plus DOM helpers for the pool-form tests. | - |

**Proportionality:** about 80 new or changed tests for a story that adds a registry, a page, a picker, an options section, three redirects and payload preservation. I removed one redundant case (an "absent when not injected" forward-compat check that the injection case already implies) and one fragile negative assertion. I folded the plain single-repository create case into the reverse-order case, which is a strict superset.

## Test contract F implements against (names pinned by these tests)
The brief leaves some names to D/F. The tests need concrete names, so these are the contract. A mismatch is a typecheck failure in the tests, and F must not edit the tests: if a name is wrong, route it back to T.

**Modules and exports:** `src/features/platform-connections/{types.ts, registry.tsx, connections-section.tsx, github-app-section.tsx}`, `src/features/runner-pools/{targets.ts, targets-row.tsx, platform-options-section.tsx}` and `src/routes/settings.connections.tsx`. The tests typecheck (`tsc` exit 0) and lint clean against the declaration-only sketch below. I built the sketch in a scratch worktree and never in this one. It is the minimum the tests touch. F may add fields, and may widen a type where it stays compatible.

    import type { ComponentType, ReactNode } from "react";

    import type { SettingsPage } from "~/features/operations/operations.functions";
    import type { RunnerPoolDetail, RunnerPoolOptions } from "~/features/runner-pools/runner-pools.functions";

    export type PostCreateReveal = { title: string; description: string; secretLabel: string; value: string; details?: ReactNode; closeButtonText?: string };
    export type TargetContext = { form: "create"; options: RunnerPoolOptions } | { form: "update"; pool: RunnerPoolDetail };
    export type TargetOption = { key: string; kindId: string; connectionId: string; nativeId: string; label: string; description?: string; group: string };
    export type ResolvedSelection = Readonly<Partial<Record<string, readonly TargetOption[]>>>;
    export type TargetKind = {
      id: string; label: string; group: string; cardinality: "many" | "one";
      standalone: boolean; requires?: readonly string[]; excludes?: readonly string[];
      requiresProviders?: ReadonlyArray<"docker" | "tart">;
      available: (context: TargetContext) => boolean;
      load: (context: TargetContext, signal: AbortSignal) => Promise<TargetOption[]>;
    };
    export type ConnectionList = { canManage: boolean; items: unknown[] };
    export type PlatformDescriptor = {
      id: string; name: string; icon: ComponentType<{ className?: string }>;
      connections: {
        list: { kind: "fetch"; load: (signal?: AbortSignal) => Promise<ConnectionList> } | { kind: "page"; read: (page: SettingsPage) => ConnectionList };
        create?: (values: Record<string, string>) => Promise<unknown>;
      };
      targetKinds: readonly TargetKind[];
      labelRules?: { help?: ReactNode; validate?: (labels: string[]) => string | null };
      platformOptions?: { help: readonly ReactNode[]; Component?: ComponentType };
      // eslint-disable-next-line @typescript-eslint/no-explicit-any
      postCreate?: (created: any) => PostCreateReveal | null | Promise<PostCreateReveal | null>;
      toCreatePayload: (selection: ResolvedSelection, context: TargetContext) => Record<string, unknown>;
      toUpdatePayload: (selection: ResolvedSelection, pool: RunnerPoolDetail, values: Record<string, unknown>) => Record<string, unknown>;
    };

    import type { PlatformDescriptor } from "./types";
    export declare const platforms: readonly PlatformDescriptor[];
    export declare function getPlatform(id: string): PlatformDescriptor;
    import type { SettingsPage } from "~/features/operations/operations.functions";
    import type { PlatformDescriptor } from "./types";
    export declare function ConnectionsSection(props: { descriptors?: readonly PlatformDescriptor[]; page: SettingsPage }): React.ReactNode;
    import type { SettingsPage } from "~/features/operations/operations.functions";
    type Data = NonNullable<SettingsPage["data"]>;
    export declare function GitHubAppSection(props: { configuration: Data["configuration"]; githubApp: Data["githubApp"]; user: Data["user"] }): React.ReactNode;
    import type { PlatformDescriptor, ResolvedSelection, TargetContext } from "~/features/platform-connections/types";
    export declare function targetPlanCheck(descriptors: readonly PlatformDescriptor[], selection: ResolvedSelection, limits: { maxCount: number }): { ok: boolean; text: string };
    export declare function anyStandaloneKindAvailable(descriptors: readonly PlatformDescriptor[], context: TargetContext): boolean;
    import type { PlatformDescriptor, ResolvedSelection, TargetContext } from "~/features/platform-connections/types";
    export declare function TargetsRow(props: { descriptors?: readonly PlatformDescriptor[]; context: TargetContext; selection: ResolvedSelection; onSelectionChange: (selection: ResolvedSelection) => void }): React.ReactNode;
    import type { PlatformDescriptor, ResolvedSelection } from "~/features/platform-connections/types";
    export declare function PlatformOptionsSection(props: { descriptors?: readonly PlatformDescriptor[]; selection: ResolvedSelection }): React.ReactNode;

**Descriptor and component behaviour the tests assume:**
- **Registry order.** `platforms` is `[github, bitbucket]`. `getPlatform(id)` returns the registered object itself. For an unknown id it returns a fallback with `id` and `name` equal to the raw id, a truthy `icon` and `targetKinds: []`.
- **Kinds.** Kind ids are `github.repository`, `github.runner-group` and `bitbucket.workspace`. A kind with nothing selected may be absent from `ResolvedSelection`, and each kind's list is in click order.
- **List binding.** A fetching list binding is `connections.list = { kind: "fetch", load(signal) }` and resolves `{ canManage, items }`, where `items` are the platform's own records (Bitbucket: `BitbucketConnection`) and the descriptor describes each row for display. The page-derived form is F's to name: in the sketch it is `{ kind: "page", read }`, and only `kind: "fetch"` is used by tests. `connections.create(values)` is the create binding.
- **Per-platform queries.** Use the test `QueryClient` defaults: no per-query `retry` override.
- **Targets picker DOM.** The Targets picker is the extended `SearchableMultiSelect`: a trigger with `aria-label="Targets"` and `aria-controls` pointing at its listbox, and options with `role="option"` and `aria-selected`. A disabled option carries `disabled` or `aria-disabled="true"`, with its reason inside the option's text.
- **Option text.**
  - Repositories: the full name, e.g. `octo-org/app`.
  - Runner groups: the account login plus the group name, e.g. `acme` + `Release`.
  - Workspaces: the connection name.
  - Per user-account installation: `<account> · Enter runner group ID`. Picking it reveals an input labelled `Runner group ID`.
  - Disabled reasons: the not-standalone reason is exactly `Select a GitHub repository or runner group first`. The exclusion reasons match `/combine with repositories/` and `/combine with a runner group/`.
  - The group heading for a kind is `<platform name> · <kind label>`, which also equals the test descriptor's `group`.
- **Chips.** Selected options keep today's `Remove <label>` chip buttons. A locked chip (Bitbucket on settings) has no such button, or a disabled one.
- **Platform options.** The section is a `button[aria-expanded]` whose text includes `Platform options`. It is collapsed by default and is not rendered when no targeted platform has options content.
- **Connections page copy.** These strings are today's copy, from the wireframe or the existing pages:
  - Buttons and labels: `Add connection`; the platform is chosen first in a dialog (a `Continue` button is optional); `Connection name`, `Workspace slug`, `API token`; `Edit`, `Save`, `New API token`, `Remove`.
  - Messages: `Could not connect`; the toast `Bitbucket workspace connected.`; the toast `Connection updated.`; the remove confirm `Remove <name>? GridOps deletes its stored token. Pools that use it must be changed first.`.
  - Load errors: the per-platform load error is a `role="alert"` naming the platform, with a `Retry` button (`InlineError`).
  - Reveal: the reveal close button is `I’ve stored it, close`.
  - After a successful create, the Add dialog closes.

**Brief vs wireframe, resolved for the tests (the brief wins):**
1. The generic `scripts/prepare-tart-runner-image.sh` text **stays in the Tart base VM row**. The brief moves only the Bitbucket script, and the unchanged first case of `runner-pool-tart-image.test.ts` still asserts the generic text. Wireframe F1's fully neutral row is not adopted.
2. The settings form offers only the pool's own kind, whatever `offeredIn` is called (A finding 2).
3. The Read-only callout appears exactly once for members, and the route owns it (A finding 4a).

## Payload-equality capture record (frozen old-form output)
- **When and where:** at tip `ef65745`, in this worktree, before any test or `src/` change; `git diff 5550cf5 ef65745 --stat -- src tests` was **empty**.
- **Command:** `CAPTURE_OUT=<scratch>/capture-output.txt npx vitest run tests/zz-capture-old-payloads.test.ts`, which printed `1 passed`. I then **deleted** the harness: it is not committed and does not exist under `tests/` or `docs/`.
- **Re-running it:** to reproduce, `git worktree add <scratch dir> 5550cf5`, run `npm ci`, save the source below as `tests/zz-capture-old-payloads.test.ts` and run the same command. Each printed object must equal its constant in `tests/pool-target-payloads.test.ts`.
- **Recording and comparison:** recording wraps the real actions with `vi.mock`. The printout uses `node:util` `inspect`, which keeps `undefined`-valued keys. The test compares with `toEqual`, never `toStrictEqual`, and says why in a comment.
- **Sanity check:** the stub `fetch` received all six requests (`POST /api/v1/runner-pools` ×4 and `PUT /api/v1/runner-pools/pool-1` ×2), so every captured payload passed the schema.

**Hand cross-check against the brief's excerpts:**
- Case 1:
  - `installationId: 1` comes from the first selected repository in loaded-list order (lines 172 and 233-235), although `acme/web` (installation 2) was clicked first.
  - `repositoryIds: [20, 10]` is the click order (line 236).
  - `runnerGroupId: 1` is the repository default (line 263).
- Case 2: `installationId: 2` is the chosen installation (line 235), `repositoryIds: []` (line 236) and `runnerGroupId: 7` (line 262).
- Case 3: `bitbucketConnectionIds` is in click order (line 237), and `providers: ["docker", "tart"]` reflects the auto-add (line 378). `tartImage` is the default because the runtime is a VM (line 251).
- Case 4: `repositoryIds`, the filtered `priorityRepositoryIds` and `priorityWaitMinutes: 15` from the field's default value (settings lines 125-127), with `runnerGroupId: 1` (line 143). There is no `installationId`, `scope` or `bitbucketConnectionIds`.
- Case 5: `repositoryIds`, `priorityRepositoryIds` and `priorityWaitMinutes` are all `undefined` (lines 125-127), and `runnerGroupId: 7` (line 143).
- Case 6: `installationId: 3` and the manual `runnerGroupId: 42` (lines 235 and 262).
- The shared-row values in all six cases follow the fixtures' defaults and the `Number(form.get(...))` reads in lines 238-260 and 128-142.

Capture output (verbatim):

    === REPOSITORY_CREATE ===
    {
      installationId: 1,
      repositoryIds: [ 20, 10 ],
      bitbucketConnectionIds: [],
      name: 'linux-app',
      scope: 'repository',
      mode: 'ephemeral',
      provider: 'docker',
      providers: [ 'docker' ],
      labels: [ 'gridops' ],
      image: 'runner:default',
      dockerImage: 'runner:default',
      tartImage: '',
      macosRuntime: 'vm',
      desiredCount: 0,
      minCount: 0,
      maxCount: 10,
      autoscalingEnabled: true,
      queueScaleFactor: 1,
      idleTimeoutMinutes: 5,
      cpuLimit: 2,
      memoryLimitMb: 2048,
      runnerGroupId: 1
    }

    === RUNNER_GROUP_CREATE ===
    {
      installationId: 2,
      repositoryIds: [],
      bitbucketConnectionIds: [],
      name: 'acme-release',
      scope: 'organization',
      mode: 'ephemeral',
      provider: 'docker',
      providers: [ 'docker' ],
      labels: [ 'gridops' ],
      image: 'runner:default',
      dockerImage: 'runner:default',
      tartImage: '',
      macosRuntime: 'vm',
      desiredCount: 0,
      minCount: 0,
      maxCount: 10,
      autoscalingEnabled: true,
      queueScaleFactor: 1,
      idleTimeoutMinutes: 5,
      cpuLimit: 2,
      memoryLimitMb: 2048,
      runnerGroupId: 7
    }

    === REPOSITORIES_AND_WORKSPACES_CREATE ===
    {
      installationId: 1,
      repositoryIds: [ 10 ],
      bitbucketConnectionIds: [ '6a1c1b2e-0000-4000-8000-000000000002', '6a1c1b2e-0000-4000-8000-000000000001' ],
      name: 'mobile-app',
      scope: 'repository',
      mode: 'ephemeral',
      provider: 'docker',
      providers: [ 'docker', 'tart' ],
      labels: [ 'gridops' ],
      image: 'runner:default',
      dockerImage: 'runner:default',
      tartImage: 'fixture-default-tahoe',
      macosRuntime: 'vm',
      desiredCount: 0,
      minCount: 0,
      maxCount: 10,
      autoscalingEnabled: true,
      queueScaleFactor: 1,
      idleTimeoutMinutes: 5,
      cpuLimit: 2,
      memoryLimitMb: 2048,
      runnerGroupId: 1
    }

    === REPOSITORY_POOL_UPDATE ===
    {
      poolId: 'pool-1',
      repositoryIds: [ 10, 11 ],
      priorityRepositoryIds: [ 10 ],
      priorityWaitMinutes: 15,
      name: 'macos-app',
      mode: 'ephemeral',
      provider: 'tart',
      providers: [ 'tart' ],
      labels: [ 'build' ],
      image: 'acme-xcode-26',
      dockerImage: '',
      tartImage: 'acme-xcode-26',
      macosRuntime: 'vm',
      desiredCount: 1,
      minCount: 0,
      maxCount: 4,
      autoscalingEnabled: true,
      queueScaleFactor: 1,
      idleTimeoutMinutes: 5,
      cpuLimit: 4,
      memoryLimitMb: 8192,
      runnerGroupId: 1
    }

    === ORGANIZATION_POOL_UPDATE ===
    {
      poolId: 'pool-1',
      repositoryIds: undefined,
      priorityRepositoryIds: undefined,
      priorityWaitMinutes: undefined,
      name: 'acme-linux',
      mode: 'ephemeral',
      provider: 'docker',
      providers: [ 'docker' ],
      labels: [ 'build' ],
      image: 'runner:acme',
      dockerImage: 'runner:acme',
      tartImage: '',
      macosRuntime: 'vm',
      desiredCount: 1,
      minCount: 0,
      maxCount: 4,
      autoscalingEnabled: true,
      queueScaleFactor: 1,
      idleTimeoutMinutes: 5,
      cpuLimit: 4,
      memoryLimitMb: 8192,
      runnerGroupId: 7
    }

    === MANUAL_RUNNER_GROUP_CREATE ===
    {
      installationId: 3,
      repositoryIds: [],
      bitbucketConnectionIds: [],
      name: 'jdoe-runners',
      scope: 'organization',
      mode: 'ephemeral',
      provider: 'docker',
      providers: [ 'docker' ],
      labels: [ 'gridops' ],
      image: 'runner:default',
      dockerImage: 'runner:default',
      tartImage: '',
      macosRuntime: 'vm',
      desiredCount: 0,
      minCount: 0,
      maxCount: 10,
      autoscalingEnabled: true,
      queueScaleFactor: 1,
      idleTimeoutMinutes: 5,
      cpuLimit: 2,
      memoryLimitMb: 2048,
      runnerGroupId: 42
    }

    === POST/PUT REQUESTS SENT === POST /api/v1/runner-pools, POST /api/v1/runner-pools, POST /api/v1/runner-pools, PUT /api/v1/runner-pools/pool-1, PUT /api/v1/runner-pools/pool-1, POST /api/v1/runner-pools

Capture harness source (verbatim, never committed):

    // @vitest-environment jsdom
    /**
     * ONE-OFF CAPTURE HARNESS (#145) -- never committed. Drives the OLD pool forms at 5550cf5
     * and prints the `data` object each passes to createRunnerPoolAction / updateRunnerPoolAction.
     */
    import { appendFileSync, writeFileSync } from "node:fs";
    import { inspect } from "node:util";
    import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
    import { act, createElement, type ComponentType } from "react";
    import { createRoot, type Root } from "react-dom/client";
    import { afterEach, beforeEach, it, vi } from "vitest";

    const loaderData = vi.hoisted(() => ({ current: undefined as unknown }));
    const recorded = vi.hoisted(() => ({ create: [] as unknown[], update: [] as unknown[] }));

    vi.mock("@tanstack/react-router", () => ({
      createFileRoute: () => (options: Record<string, unknown>) => ({ options, useLoaderData: () => loaderData.current }),
      getRouteApi: () => ({ useLoaderData: () => loaderData.current }),
      Link: ({ children, to }: { children?: unknown; to?: string }) => createElement("a", { href: String(to) }, children as never),
      useNavigate: () => vi.fn(),
    }));
    vi.mock("sonner", () => ({ toast: { success: vi.fn(), error: vi.fn() } }));
    vi.mock("../src/features/runner-pools/runner-pools.functions", async (importOriginal) => {
      const actual = await importOriginal<typeof import("../src/features/runner-pools/runner-pools.functions")>();
      return {
        ...actual,
        createRunnerPoolAction: (args: Parameters<typeof actual.createRunnerPoolAction>[0]) => { recorded.create.push(args.data); return actual.createRunnerPoolAction(args); },
        updateRunnerPoolAction: (args: Parameters<typeof actual.updateRunnerPoolAction>[0]) => { recorded.update.push(args.data); return actual.updateRunnerPoolAction(args); },
      };
    });

    import { Route as NewPoolRoute } from "../src/routes/_app/runner-pools.new";
    import { Route as PoolSettingsRoute } from "../src/routes/_app/runner-pools.$poolId.settings";

    declare global {
      var IS_REACT_ACT_ENVIRONMENT: boolean | undefined;
    }

    class NoopResizeObserver { observe() {} unobserve() {} disconnect() {} }

    // ---- Fixed inputs (identical in tests/pool-target-payloads.test.ts) ----
    const INSTALLATIONS = [
      { id: 1, accountLogin: "octo-org", accountType: "Organization" },
      { id: 2, accountLogin: "acme", accountType: "Organization" },
      { id: 3, accountLogin: "jdoe", accountType: "User" },
    ];
    const REPOSITORIES = [
      { id: 10, installationId: 1, accountLogin: "octo-org", accountType: "Organization", fullName: "octo-org/app", private: true },
      { id: 11, installationId: 1, accountLogin: "octo-org", accountType: "Organization", fullName: "octo-org/api", private: false },
      { id: 20, installationId: 2, accountLogin: "acme", accountType: "Organization", fullName: "acme/web", private: true },
    ];
    const RUNNER_GROUPS: Record<number, Array<{ id: number; name: string; visibility: string; isDefault: boolean }>> = {
      1: [{ id: 1, name: "Default", visibility: "all", isDefault: true }, { id: 4, name: "Linux builds", visibility: "selected", isDefault: false }],
      2: [{ id: 1, name: "Default", visibility: "all", isDefault: true }, { id: 7, name: "Release", visibility: "private", isDefault: false }],
    };
    const BITBUCKET = [
      { id: "6a1c1b2e-0000-4000-8000-000000000001", name: "Acme Bitbucket", workspace: "acme", workspaceUuid: "{acme}" },
      { id: "6a1c1b2e-0000-4000-8000-000000000002", name: "Mobile delivery", workspace: "acme-mobile", workspaceUuid: "{acme-mobile}" },
    ];
    const createOptions = () => ({
      authenticated: true,
      installUrl: "https://github.com/apps/gridops/installations/new",
      builtinRunnerHostEnabled: true,
      installations: INSTALLATIONS,
      repositories: REPOSITORIES.map(({ id, installationId, fullName, private: isPrivate }) => ({ id, installationId, fullName, private: isPrivate })),
      runnerGroups: Object.entries(RUNNER_GROUPS).flatMap(([installationId, groups]) => groups.map((group) => ({ installationId: Number(installationId), ...group }))),
      bitbucketConnections: BITBUCKET,
      defaults: {
        provider: "docker", providers: ["docker"], image: "runner:default", dockerImage: "runner:default", tartImage: "fixture-default-tahoe",
        macosRuntime: "vm", labels: ["gridops"], cpuLimit: 2, memoryLimitMb: 2048, desiredCount: 0, minCount: 0, maxCount: 10,
        autoscalingEnabled: true, queueScaleFactor: 1, idleTimeoutMinutes: 5, runnerGroupId: 1, maxCpuLimit: 16, maxMemoryLimitMb: 65_536,
      },
    });
    const settingsPool = (overrides: Record<string, unknown>) => ({
      id: "pool-1", installationId: 1, repositoryId: null, repository: null, repositoryIds: [], repositories: [],
      priorityRepositoryIds: [], priorityWaitMinutes: 10, bitbucketConnections: [], bitbucketConnectionIds: [],
      accountLogin: "octo-org", name: "linux-app", scope: "organization", mode: "ephemeral", provider: "docker",
      providers: ["docker"], labels: ["build"], image: "runner:acme", dockerImage: "runner:acme", tartImage: "",
      macosRuntime: "vm", desiredCount: 1, minCount: 0, maxCount: 4, cpuLimit: 4, memoryLimitMb: 8192,
      maxCpuLimit: 16, maxMemoryLimitMb: 65_536, builtinRunnerHostEnabled: true, runnerGroupId: 1, paused: false,
      state: "active", autoscalingEnabled: true, queueScaleFactor: 1, idleTimeoutMinutes: 5, configurationVersion: 3,
      provisionFailureCount: 0, provisionRetryAt: null, provisionCircuitOpen: false, canManage: true,
      ...overrides,
    });
    const REPOSITORY_POOL = settingsPool({
      name: "macos-app", scope: "repository", installationId: 1, repositoryIds: [10], repositories: [{ ...REPOSITORIES[0], prioritised: true }],
      priorityRepositoryIds: [10], priorityWaitMinutes: 15, bitbucketConnections: [BITBUCKET[0]], bitbucketConnectionIds: [BITBUCKET[0]?.id],
      provider: "tart", providers: ["tart"], labels: ["build"], image: "acme-xcode-26", dockerImage: "", tartImage: "acme-xcode-26",
    });
    const ORGANIZATION_POOL = settingsPool({ name: "acme-linux", scope: "organization", installationId: 2, accountLogin: "acme", runnerGroupId: 1 });

    function fetchStub(url: string, init?: RequestInit) {
      const method = init?.method ?? "GET";
      if (method === "GET" && url === "/api/v1/runner-pools/repositories") return Response.json({ items: REPOSITORIES });
      const repos = /^\/api\/v1\/installations\/(\d+)\/repositories$/.exec(url);
      if (method === "GET" && repos) return Response.json({ items: REPOSITORIES.filter((repository) => repository.installationId === Number(repos[1])) });
      const groups = /^\/api\/v1\/installations\/(\d+)\/runner-groups$/.exec(url);
      if (method === "GET" && groups) {
        const items = RUNNER_GROUPS[Number(groups[1])];
        return items ? Response.json({ items }) : Response.json({ error: "Runner groups are listed for organizations only." }, { status: 404 });
      }
      if (method === "GET" && url === "/api/v1/platform-connections/bitbucket") return Response.json({ canManage: true, items: [] });
      if (method === "POST" && url === "/api/v1/runner-pools") return Response.json({ id: "pool-new" }, { status: 201 });
      if (method === "PUT" && url === "/api/v1/runner-pools/pool-1") return Response.json({ ok: true, configurationVersion: 4, rollingReplacement: false });
      return Response.json({ error: `not in this test: ${method} ${url}` }, { status: 404 });
    }

    // ---- DOM helpers ----
    let container: HTMLDivElement;
    let root: Root;
    beforeEach(() => {
      globalThis.IS_REACT_ACT_ENVIRONMENT = true;
      vi.stubGlobal("ResizeObserver", NoopResizeObserver);
      vi.stubGlobal("fetch", vi.fn(async (url: string, init?: RequestInit) => fetchStub(url, init)));
      recorded.create.length = 0;
      recorded.update.length = 0;
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
    async function renderPage(route: unknown, data: unknown) {
      loaderData.current = data;
      const page = (route as { options: { component: ComponentType } }).options.component;
      const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
      await act(async () => root.render(createElement(QueryClientProvider, { client }, createElement(page))));
      await settle();
    }
    function trigger(name: string) {
      const found = document.querySelector<HTMLButtonElement>(`button[aria-label="${name}"]`);
      if (!found) throw new Error(`No "${name}" control`);
      return found;
    }
    async function pick(control: string, ...parts: string[]) {
      const button = trigger(control);
      if (button.getAttribute("aria-expanded") !== "true") act(() => button.click());
      const scope = document.getElementById(button.getAttribute("aria-controls") ?? "") ?? document;
      const found = [...scope.querySelectorAll<HTMLElement>("[role=option]")].find((element) => parts.every((part) => element.textContent?.includes(part)));
      if (!found) throw new Error(`No option with ${parts.join(" + ")} in "${control}"`);
      act(() => found.click());
      await settle();
    }
    function type(input: HTMLInputElement, value: string) {
      const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")?.set;
      act(() => { setter?.call(input, value); input.dispatchEvent(new Event("input", { bubbles: true })); });
    }
    async function submit() {
      const form = document.querySelector("form");
      if (!form) throw new Error("No form");
      await act(async () => { form.dispatchEvent(new Event("submit", { bubbles: true, cancelable: true })); });
      await settle();
    }
    const OUT = process.env.CAPTURE_OUT ?? "capture-output.txt";
    writeFileSync(OUT, "");
    function print(label: string, value: unknown) {
      appendFileSync(OUT, `\n=== ${label} ===\n${inspect(value, { depth: null, sorted: false, breakLength: 120 })}\n`);
    }

    it("captures the old new-pool and pool-settings payloads", async () => {
      // Case 1: GitHub repositories from two installations, clicked in reverse list order.
      await renderPage(NewPoolRoute, createOptions());
      type(document.querySelector<HTMLInputElement>("#pool-name")!, "linux-app");
      await pick("Repositories", "acme/web");
      await pick("Repositories", "octo-org/app");
      await submit();
      print("REPOSITORY_CREATE", recorded.create.at(-1));
      act(() => root.unmount()); root = createRoot(container);

      // Case 2: GitHub organization runner group (installation 2, non-default group).
      await renderPage(NewPoolRoute, createOptions());
      type(document.querySelector<HTMLInputElement>("#pool-name")!, "acme-release");
      await pick("Runner pool scope", "Organization");
      await pick("GitHub installation", "acme");
      await pick("GitHub runner group", "Release");
      await submit();
      print("RUNNER_GROUP_CREATE", recorded.create.at(-1));
      act(() => root.unmount()); root = createRoot(container);

      // Case 3: one GitHub repository plus two Bitbucket workspaces (clicked in reverse list order).
      await renderPage(NewPoolRoute, createOptions());
      type(document.querySelector<HTMLInputElement>("#pool-name")!, "mobile-app");
      await pick("Repositories", "octo-org/app");
      await pick("Bitbucket Cloud workspaces", "Mobile delivery");
      await pick("Bitbucket Cloud workspaces", "Acme Bitbucket");
      await submit();
      print("REPOSITORIES_AND_WORKSPACES_CREATE", recorded.create.at(-1));
      act(() => root.unmount()); root = createRoot(container);

      // Case 4: repository pool settings save (adds octo-org/api; Bitbucket workspace stays as it is).
      await renderPage(PoolSettingsRoute, REPOSITORY_POOL);
      await pick("Pool repositories", "octo-org/api");
      await submit();
      print("REPOSITORY_POOL_UPDATE", recorded.update.at(-1));
      act(() => root.unmount()); root = createRoot(container);

      // Case 5: organization pool settings save (switches to the Release runner group).
      await renderPage(PoolSettingsRoute, ORGANIZATION_POOL);
      await pick("GitHub runner group", "Release");
      await submit();
      print("ORGANIZATION_POOL_UPDATE", recorded.update.at(-1));
      act(() => root.unmount()); root = createRoot(container);

      // Case 6 (additional): manual runner group ID on a user-account installation.
      await renderPage(NewPoolRoute, createOptions());
      type(document.querySelector<HTMLInputElement>("#pool-name")!, "jdoe-runners");
      await pick("Runner pool scope", "Organization");
      await pick("GitHub installation", "jdoe");
      type(document.querySelector<HTMLInputElement>('input[name="runnerGroupId"]')!, "42");
      await submit();
      print("MANUAL_RUNNER_GROUP_CREATE", recorded.create.at(-1));

      const posts = (vi.mocked(fetch).mock.calls as Array<[string, RequestInit | undefined]>).filter(([, init]) => init?.method === "POST" || init?.method === "PUT");
      appendFileSync(OUT, `\n=== POST/PUT REQUESTS SENT === ${posts.map(([url, init]) => `${init?.method} ${url}`).join(", ")}\n`);
    });

## RED confirmation
**Command:** `npx vitest run` (JSON reporter, summarised per test below).

**Overall:** 405 tests, 360 passed, 45 failed. All 37 pre-existing files still pass, except the 9 updated `runner-pool-tart-image` cases, which now go through `Targets`.

**Why each failure is the right reason:**
- **Rendered-form and route tests** fail on the missing feature, not on setup:
  - `No "Targets" control` / `No Targets picker on the form`: the Targets row does not exist yet.
  - Redirect targets: `/settings/integrations` and `/settings/github` are wrong today, and the two old pages have no `beforeLoad`, so the test gets `expected 'undefined' to be 'function'`.
  - Link and nav hrefs still point at the old pages.
  - No platform badge renders in the runner row.
  - The read-only pool view has no per-platform text.
  - Each source-scan case lists the real lines that branch on a platform today.
- **The four files that import only new modules** (`platform-registry`, `pool-targets-rules`, `targets-row-forward-compat`, `connections-page`) fail at import with `Cannot find module …/registry` or `…/connections-section` (and `github-app-section`). Those modules are the feature, and the brief names them by path. I checked that these files are otherwise correct: they typecheck with `tsc` exit 0 and lint clean against the contract sketch above. So once the modules exist, their only failures will be behavioural.
- **`npm run typecheck`** in this worktree fails only with `TS2307` for those new modules, plus the implicit-`any` errors that follow from them. The brief expects this in RED.
- **`npm run lint`** exits 0.

**Characterization guards.** These pass before and after the change, on purpose. Each protects preserved behaviour the refactor moves, and none of them is meant to be RED:
- the six unchanged Tart cases that do not create a pool;
- (a)/(b) "Install the GitHub App" with 0 installations, with and without Bitbucket;
- `/settings` without `appError` goes to `/settings/general`;
- `appError` is read only in `settings.index.tsx`;
- the Bitbucket label regex exists only in `schemas.ts`.

Per-test summary:

    tests 405 passed 360 failed 45 | files failed: 25 of 131

    ## connections-page.test.ts | Failed to resolve import "../src/features/platform-connections/github-app-section" from "tests/connections-page.test.ts". Does the file exist?

    ## connections-routes.test.ts | 
      FAIL | old routes are redirect-only and point at Settings → Connections > /settings/integrations replace-redirects to /settings/connections, with n | AssertionError: expected 'undefined' to be 'function' // Object.is equality
      FAIL | old routes are redirect-only and point at Settings → Connections > /settings/github replace-redirects to /settings/connections, with no page | AssertionError: expected 'undefined' to be 'function' // Object.is equality
      FAIL | old routes are redirect-only and point at Settings → Connections > /platform-connections replace-redirects to /settings/connections, with no | AssertionError: expected { replace: true, …(2) } to match object { to: '/settings/connections', …(1) }
      FAIL | /settings?appError= > toasts the error once and replace-redirects to /settings/connections with no search | AssertionError: expected { replace: true, …(2) } to match object { to: '/settings/connections', …(1) }
      FAIL | /settings?appError= > redirects the same way without toasting on preload | AssertionError: expected { replace: true, …(2) } to match object { to: '/settings/connections', …(1) }
      PASS | /settings?appError= > still sends /settings without appError to /settings/general | 
      FAIL | in-app links point straight at /settings/connections > the settings navigation has one Connections item | AssertionError: expected [ …(8) ] to deep equally contain [ '/settings/connections', …(1) ]
      FAIL | in-app links point straight at /settings/connections > the webhooks empty state links to GitHub settings on the Connections page | AssertionError: expected [ [ '/settings/github', …(1) ] ] to deep equally contain [ '/settings/connections', …(1) ]
      FAIL | in-app links point straight at /settings/connections > the new-pool form's connect-a-workspace link goes to the Connections page | Error: No "Targets" control

    ## platform-neutral-source.test.ts | 
      FAIL | no link or redirect targets a redirecting route > names /settings/github or /settings/integrations only in the two redirect-only route decla | AssertionError: expected [ …(6) ] to deeply equal []
      PASS | no link or redirect targets a redirecting route > reads appError only in settings.index.tsx (the Connections page does not consume it) | 
      FAIL | shared rows do not branch on a platform > src/routes/_app/runner-pools.new.tsx compares against no platform id, kind id, scope or platform-n | AssertionError: expected [ …(11) ] to deeply equal []
      FAIL | shared rows do not branch on a platform > src/routes/_app/runner-pools.$poolId.settings.tsx compares against no platform id, kind id, scope  | AssertionError: expected [ …(14) ] to deeply equal []
      FAIL | shared rows do not branch on a platform > the Tart base VM row and the plan preview carry no platform text or prop | AssertionError: expected 'import { SettingsRow } from "~/compon…' not to match /bitbucket/i
      FAIL | shared rows do not branch on a platform > the Targets row, Platform options section and Connections list contain no platform-specific code | AssertionError: src/features/runner-pools/targets-row.tsx exists: expected false to be true // Object.is equality
      FAIL | shared rows do not branch on a platform > the runner row takes platform names from the registry, with no platform ternary | AssertionError: expected 'import { Link } from "@tanstack/react…' not to match /runner\.platform\s*[!=]==/
      PASS | the Bitbucket label rule has one home > appears only in schemas.ts | 

    ## platform-registry.test.ts | Cannot find module '../src/features/platform-connections/registry' imported from /Users/andreangelantoni/Projects/gridops/.claude/worktrees/ui-145-platform-neutral-cf5832/tests/platform-registry.test.ts

    ## pool-target-payloads.test.ts | 
      FAIL | New pool: the Targets row sends what the old form sent > GitHub repositories from two installations: click order kept, installation from the | Error: No Targets picker on the form
      FAIL | New pool: the Targets row sends what the old form sent > GitHub organization runner group: installation and group come from the picked group | Error: No Targets picker on the form
      FAIL | New pool: the Targets row sends what the old form sent > GitHub repository plus Bitbucket workspaces | Error: No Targets picker on the form
      FAIL | New pool: the Targets row sends what the old form sent > manual runner group ID for a user-account installation | Error: No Targets picker on the form
      FAIL | Pool settings: the Targets row saves what the old form saved > repository pool: repositories, priority and fair-share wait; the Bitbucket wo | Error: No Targets picker on the form
      FAIL | Pool settings: the Targets row saves what the old form saved > organization pool: the picked runner group, and no repository or priority fie | Error: No Targets picker on the form

    ## pool-targets-form.test.ts | 
      FAIL | New pool Targets row: mixing rules > (a) shows every Bitbucket workspace disabled with a reason until a GitHub repository or runner group is | Error: No "Targets" control
      FAIL | New pool Targets row: mixing rules > (b) disables runner groups while a repository is picked, and repositories while a runner group is picke | Error: No "Targets" control
      FAIL | New pool Targets row: mixing rules > (c) caps repositories and workspaces at the maximum runner count separately | Error: No "Targets" control
      FAIL | New pool Targets row: mixing rules > (d) adds macOS · Tart when a Bitbucket workspace is picked | Error: No "Targets" control
      FAIL | New pool Targets row: mixing rules > (e) keeps the workspace when the last GitHub target is removed, and submit fails in the schema with no  | Error: No "Targets" control
      FAIL | New pool Targets row: mixing rules > raises the Maximum floor for selected repositories only | Error: No "Targets" control
      FAIL | New pool: create-button gate (today: submitting || installations.length === 0) > (c)/(d)/(e) is enabled with an installation and no Bitbucke | AssertionError: expected null not to be null
      FAIL | New pool: create-button gate (today: submitting || installations.length === 0) > (c)/(d)/(e) is enabled with an installation and Bitbucket c | AssertionError: expected null not to be null
      PASS | New pool: create-button gate (today: submitting || installations.length === 0) > (a)/(b) with no installation and no Bitbucket connection, s | 
      PASS | New pool: create-button gate (today: submitting || installations.length === 0) > (a)/(b) with no installation and Bitbucket connections, sho | 
      FAIL | New pool: create-button gate (today: submitting || installations.length === 0) > (f) is disabled while the pool is being created | Error: No "Targets" control
      FAIL | New pool: Platform options > is absent with no target, collapsed once a platform with options is targeted, and follows the selection | Error: No "Targets" control
      FAIL | Pool settings Targets row: only the pool's own kind > repository pool: offers repositories only, shows its Bitbucket workspace locked, keeps | Error: No "Targets" control
      FAIL | Pool settings Targets row: only the pool's own kind > organization pool: offers runner groups of its own installation only, and no Priority | Error: No "Targets" control
      FAIL | Pool settings Targets row: only the pool's own kind > read-only pool: lists its targets per platform through the registry, Priority only for | AssertionError: expected 'ConfigurationAn installation administ…' to contain 'GitHub'

    ## pool-targets-rules.test.ts | Cannot find module '../src/features/platform-connections/registry' imported from /Users/andreangelantoni/Projects/gridops/.claude/worktrees/ui-145-platform-neutral-cf5832/tests/pool-targets-rules.test.ts

    ## runner-pool-tart-image.test.ts | 
      PASS | New runner pool: Tart base VM > asks for the Tart base VM for a macOS · Tart pool, prefilled with the server default | 
      PASS | New runner pool: Tart base VM > has no Tart base VM for a Docker-only pool, and shows it once macOS · Tart is added | 
      PASS | New runner pool: Tart base VM > hides the Tart base VM for native execution, which clones no VM | 
      FAIL | New runner pool: Tart base VM > points a Bitbucket pool at the Bitbucket preparation script in Platform options | Error: No "Targets" button
      FAIL | New runner pool: Tart base VM > sends the typed Tart base VM, not the default, for a Tart pool | Error: No "Targets" button
      FAIL | New runner pool: Tart base VM > sends both images for a pool with Docker and Tart | Error: No "Targets" button
      PASS | Pool settings: Tart base VM > uses the same row and wording as the create form | 
      FAIL | New runner pool: images of providers the pool does not use > sends no Tart base VM for a Docker-only pool | Error: No "Targets" button
      FAIL | New runner pool: images of providers the pool does not use > sends no Docker image for a Tart-only pool | Error: No "Targets" button
      FAIL | New runner pool: images of providers the pool does not use > drops the Tart base VM when macOS · Tart is taken out again | Error: No "Targets" button
      PASS | Pool settings: adding a provider asks for its image > starts the Tart base VM empty and required when Tart is added, even if the pool still  | 
      PASS | Pool settings: adding a provider asks for its image > starts the Docker image empty and required when Docker is added to a Tart pool | 
      PASS | Pool settings: adding a provider asks for its image > needs no Tart base VM when Tart is added for native execution | 
      PASS | Pool settings: adding a provider asks for its image > sends no image for a provider the pool does not use | 
      PASS | Pool settings: adding a provider asks for its image > drops the Tart base VM when Tart is taken out of a pool | 
      FAIL | Native Tart: no base VM is stored > sends no Tart base VM for a native Tart pool | Error: No "Targets" button
      FAIL | Native Tart: no base VM is stored > sends no Tart base VM for a native Tart pool that also uses Docker | Error: No "Targets" button
      FAIL | Native Tart: no base VM is stored > still sends the base VM when the form is switched back to Virtual machine | Error: No "Targets" button
      PASS | Native Tart: no base VM is stored > asks for a base VM, empty and required, when such a pool is switched to Virtual machine in Settings | 

    ## runner-row-platform.test.ts | 
      FAIL | runner row platform badge > shows the registry name for github | AssertionError: expected 'Idlemac-runner-1acme/webmacOS · arm64…' to contain 'GitHub'
      FAIL | runner row platform badge > shows the registry name for bitbucket | AssertionError: expected 'Idlemac-runner-1acme/webmacOS · arm64…' to contain 'Bitbucket'
      FAIL | runner row platform badge > shows the registry name for gitlab | AssertionError: expected 'Idlemac-runner-1acme/webmacOS · arm64…' to contain 'gitlab'

    ## targets-row-forward-compat.test.ts | Failed to resolve import "../src/features/platform-connections/registry" from "tests/targets-row-forward-compat.test.ts". Does the file exist?

## Source facts in unchanged code that the tests rely on (for the diff-gate `evidence.md`, Phase 6)
- `src/routes/_app/runner-pools.new.tsx`: 71 ("Install the GitHub App" empty state), 489 (`Could not create the pool` callout), 494-496 (`Create pool` / `Creating…`).
- `src/routes/settings.integrations.tsx`: 44 (`Bitbucket workspace connected.`), 73 (`Could not connect`), 110 (`Connection updated.`), 136 (the remove confirm text), 155 (`New API token`).
- `src/features/fleet/api-tokens-section.tsx`: 197-198 (Escape and outside clicks ignored), 219 (`I’ve stored it, close`).
- `src/features/runner-pools/schemas.ts`: 180-181 (the Bitbucket label rule).
- `src/lib/api.ts`: 35-37 (a non-2xx response throws `ApiError`).
- `src/components/confirm-dialog.tsx`: 27 (without a ConfirmHost, `confirmAction` falls back to `window.confirm`).
- `src/routes/__root.tsx`: 49 (the app wraps everything in `TooltipProvider`; the tests mirror it).

## Ready for F
**RED is valid.** F may implement against these tests. F must not edit any test file, `tests/fixtures/pool-forms.ts` or the frozen constants. A test that looks wrong goes back to T.
