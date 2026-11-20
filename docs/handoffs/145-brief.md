## Phase — UI: one Connections page and one Targets picker for every CI platform (#145)

**Issue:** Performant-Labs/gridops#145 (part of epic #136, wave 0)
**Branch:** claude/ui-145-platform-neutral-cf5832 (this worktree)
**Review-rigor:** second-opinion — foundational, component-creating: the platform registry is a contract that #141 (GitLab) and later platforms consume.
**Forward-compat:** required — see the "Forward-compat" section below; record the table in `docs/handoffs/145/survey.md`.
**Design (Phase 3):** UI surface — wireframe the Connections page, the Targets row and the Platform options section.

### Objective
Make the web UI platform-neutral: one Settings → Connections page, one Targets multi-select and one collapsed "Platform options" section in the pool forms, and registry-driven runner badges, all reading a single frontend platform registry — so a new CI platform is added by registering one descriptor, with no new pages or form rows.

### Scope and hard limits
- Frontend only (`src/`, `tests/`, and docs under `docs/` if needed). **No API, database or OpenAPI schema change. Do not touch `crates/`, `migrations/` or `docs/openapi/`** ("schema" here means the server API / DB / OpenAPI contract; the frontend zod module `src/features/runner-pools/schemas.ts` MAY be edited to export existing rules for reuse, without changing what it accepts or rejects) (another story, #137, is changing those in parallel).
- Use the existing endpoints and functions (`src/features/platform-connections/platform-connections.functions.ts`, the GitHub settings functions, `src/features/runner-pools/*`).
- GitHub and Bitbucket only. Do not add a GitLab descriptor (that is #141).

### Codebase starting points
- `src/routes/settings.github.tsx` — GitHub App + credentials, installations/repository listings.
- `src/routes/settings.integrations.tsx` — actually the Bitbucket page (`BitbucketSettings`, titled "Bitbucket"). Verified: line 21 `createFileRoute("/settings/integrations")`, line 23 `component: BitbucketSettings`, line 54 `<SettingsLayout ... title="Bitbucket">`; it also holds `ConnectionRow` (inline edit/remove, gated on `canManage`).
- `src/routes/_app/platform-connections.tsx` — redirect to `/settings/integrations` (line 6: `throw redirect({ replace: true, to: "/settings/integrations" })`).
- `src/routes/settings.tsx` / `settings.index.tsx` — settings nav (`settings.tsx` lines 38-45: "Integrations" section with `GitHub` → `/settings/github` and `Bitbucket` → `/settings/integrations`); `settings.index.tsx` line 10 redirects `?appError=` to `/settings/github`.
- `src/routes/_app/runner-pools.new.tsx` — GitHub rows (Scope, Installation, Repositories, Runner group), separate "Bitbucket workspaces" picker (~line 369), platform-branching "Additional labels" help (~line 448), `TartBaseVmRow bitbucket=…`.
- `src/routes/_app/runner-pools.$poolId.settings.tsx` — repeats the GitHub scope branching (the scope is fixed by the pool: Repositories row when `pool.scope === "repository"`, Runner group row otherwise) and `TartBaseVmRow bitbucket=…`. Note: this form has **no** Bitbucket workspaces picker today and its submit (lines 122-144) does not send `bitbucketConnectionIds`.
- `src/features/runner-pools/tart-base-vm-row.tsx`, `schemas.ts`.
- `src/features/runners/runner-row.tsx:121` — hard-coded `runner.platform === "bitbucket" ? "Bitbucket" : "GitHub"`.
- `src/components/ui/searchable-select.tsx` (single select; exports `filterSearchableOptions`, `SearchableSelectOption` = `{ value, label, description?, keywords? }`, tested by `tests/searchable-select.test.ts`) and `src/components/ui/searchable-multi-select.tsx` (`SearchableMultiSelect`: props `options`, `values`, `onValueChange`, `ariaLabel`, `placeholder`, `searchPlaceholder`, `emptyMessage`, `loading`, `disabled`, `maxSelected`, `selectedNoun`, `selectAll`; no grouping today). Extend `SearchableMultiSelect` (e.g. an optional group field on options and group headings in the listbox) for the grouped multi-select rather than adding a parallel widget.
- `src/features/runner-pools/runner-pools.functions.ts` — `createRunnerPoolAction({ data })` / `updateRunnerPoolAction({ data })` parse `data` with the zod schemas and POST/PUT it through `api()` (`src/lib/api.ts`, which sends `JSON.stringify(body)`).
- `src/features/operations/operations.functions.ts` — the GitHub settings functions: `getSettingsPage()` (its `data.githubApp`, `data.installations`, `data.user.role`), `syncGitHubAction()`, `createGitHubAppManifestAction()`.

### Required design
1. **Platform registry** in `src/features/platform-connections/registry.tsx` (`.tsx` because descriptors carry icon components and an optional platform-options component slot): one exported descriptor type (name it `PlatformDescriptor`) and one descriptor per platform, plus lookup helpers (`getPlatform(id)`, `platforms` list, a safe fallback for an unknown platform id). The descriptor must be able to carry — as data/functions, not as branches elsewhere — at least:
   - `id`, display `name`, `icon`, and badge styling;
   - **connection form fields** (declarative field list: key, label, type incl. secret, required, help, editable-on-update) plus list/create/update/remove bindings to the existing API functions, and whether the current user can manage; each binding is optional per descriptor (see "Verified API surface" below: GitHub has no create/update/remove connection endpoint);
   - **target kinds** (e.g. GitHub repository, GitHub organization runner group, Bitbucket workspace): id, label, group heading, how to load options, how a selected target maps onto the existing pool payload fields, and compatibility (which kinds may be mixed in one pool);
   - **label rules** (help text and an optional validator for "Additional labels");
   - **platform-options content** (help text such as Bitbucket's label character rules and its Tart image script, plus an optional component slot for future per-platform settings);
   - a **runner link builder** (given a runner, return the external URL/label or null);
   - **post-create hooks** (after a connection is created, an optional hook that returns something to show once — e.g. a webhook secret — which the Connections page renders in a one-time reveal dialog). GitHub/Bitbucket may not use it today, but the page must support it generically.
2. **Connections page** at Settings → Connections (new route, e.g. `/settings/connections`), replacing Integrations in the settings nav. Lists every connection (each GitHub App installation, each Bitbucket workspace) with platform badge, scope and status. One **Add connection** action asks for the platform first, then renders only that descriptor's fields. Edit and remove work through the descriptor. GitHub App setup (admin-only) may stay as a section on this page. `/settings/integrations`, `/settings/github` and `/platform-connections` redirect (replace) to it. Each redirect points directly at `/settings/connections` — no redirect chains: update `_app/platform-connections.tsx` in place (it currently targets `/settings/integrations`), and repoint the `settings.index.tsx` `?appError=` redirect and the `Settings → Integrations` link in `runner-pools.new.tsx` (line 368) at the Connections page.
3. **Targets row** in both `runner-pools.new.tsx` and `runner-pools.$poolId.settings.tsx`: one searchable multi-select grouped by platform/target kind, replacing the separate GitHub Scope/Installation/Repositories/Runner-group rows and the Bitbucket workspaces picker. Existing rules preserved: a pool cannot mix incompatible platforms/kinds; GitHub repository vs organization runner group is the kind of target picked. The submitted payload must be what the old form would have sent for the same selection, defined as: the `data` object passed to `createRunnerPoolAction` (new-pool form) or `updateRunnerPoolAction` (pool settings) is deep-equal (Vitest `toEqual`) to the object the old form built for the same selection. This is the comparison point because `api()` sends exactly `JSON.stringify` of the parsed object; key order is not significant, and a key set to `undefined` counts as absent (as `JSON.stringify` drops it — e.g. the settings form's `repositoryIds: undefined` for organization pools).
4. **Platform options** section, collapsed by default, rendered only for the platforms the pool targets, filled from the descriptors. Shared rows (name, providers, mode, images, counts, CPU/memory, autoscaling) no longer branch on platform.
5. **Runner row**: platform badge and any platform text come from the registry; no platform ternaries remain in `runner-row.tsx`.

### Verified facts from the source (for the designer and implementer)
- **Connection API surface.** `platform-connections.functions.ts` exports `getBitbucketConnections()` → `{ canManage, items: BitbucketConnection[] }`, `createBitbucketConnection({ name, workspace, accessToken })` → `Pick<BitbucketConnection, "id" | "provider" | "name" | "workspace" | "workspaceUuid">`, `updateBitbucketConnection(id, { name, accessToken? })`, `removeBitbucketConnection(id)`. GitHub has no connection create/update/remove function: installations are read from `getSettingsPage()` (`data.installations`), are added through the external `githubApp.installUrl` link and managed through each installation's `manageUrl`; `syncGitHubAction()` refreshes them and `createGitHubAppManifestAction()` starts App setup (admin only, `user.role === "admin"`). So the GitHub descriptor binds list only (plus its existing external links), and the Bitbucket descriptor binds all four.
- **Post-create hook.** No existing create response carries a secret (`createBitbucketConnection` returns only id/provider/name/workspace/workspaceUuid). The hook is frontend-only generic support; neither the GitHub nor the Bitbucket descriptor uses it in this story, so it is exercised with a test-only descriptor (see acceptance criteria).
- **Current target rules** (`src/features/runner-pools/schemas.ts` and `runner-pools.new.tsx`): `createRunnerPoolSchema` requires `installationId` (positive int) and `scope: "repository" | "organization"`; `scope === "repository"` requires ≥1 `repositoryIds`; `scope === "organization"` requires `repositoryIds` empty and uses `runnerGroupId`; `repositoryIds.length` and `bitbucketConnectionIds.length` must each be ≤ `maxCount`. `validateBitbucketConnections`: any Bitbucket connection requires the `tart` provider and labels matching `/^[a-z0-9.]+$/`. The form auto-adds `tart` when a workspace is picked (line 378). Bitbucket workspaces are sent alongside either GitHub scope (line 237), so today GitHub repositories and an organization runner group are mutually exclusive, while Bitbucket workspaces combine with either.
- **Current platform-branching sites in shared rows** (baseline for the "no branching in shared rows" criterion; each must be removed or moved into Targets / Platform options / descriptors):
  - `runner-pools.new.tsx`: Scope row (283-294), Installation row (295-315), Repositories row (316-343), Runner group row (344-364), Bitbucket workspaces row with `/settings/integrations` link (365-386), `TartBaseVmRow bitbucket={bitbucketConnectionIds.length > 0}` (446), Additional labels description and placeholder on `bitbucketConnectionIds.length` (448-449), empty states "Connect GitHub first" / "Install the GitHub App" (53-77).
  - `runner-pools.$poolId.settings.tsx`: "GitHub accounts"/"Installation" row (161-163), Repositories vs Runner group rows (164-212), `TartBaseVmRow bitbucket={pool.bitbucketConnectionIds.length > 0}` (271), Priority section shown for `pool.scope === "repository"` (302-325).
  - `src/features/runner-pools/tart-base-vm-row.tsx`: `bitbucket` prop choosing the Tart image script text (lines 9, 15-17).
  - `src/components/pool-plan-preview.tsx` line 50: GitHub runner-group text on `scope === "organization"`.
  - `src/features/runners/runner-row.tsx` line 121: `runner.platform === "bitbucket" ? "Bitbucket" : "GitHub"`.

### Forward-compat (consumer: #141 GitLab)
#141 will add a GitLab descriptor with no layout changes. It needs: connection fields (instance URL, token, group/project), target kinds (GitLab group, GitLab project), label rules (GitLab tag rules + a `run_untagged` toggle as platform-options content), a runner link builder, and a post-create hook that shows a webhook secret once. Confirm in `docs/handoffs/145/survey.md` (a table: one row per need above, naming the descriptor field that carries it) that the descriptor type satisfies each without changes to the page, picker or options components.

### Acceptance criteria
- [ ] Every existing flow still works: GitHub repository pools; GitHub runner-group pools; Bitbucket workspace pools; creating, editing and removing a Bitbucket connection; managing the GitHub App.
- [ ] `/settings/integrations`, `/settings/github` and `/platform-connections` redirect to the Connections page.
- [ ] The new-pool form and pool settings have no platform-specific rows outside Targets and Platform options (no `bitbucket`/`github` branching in shared rows).
- [ ] `runner-row.tsx` reads its platform badge from the registry.
- [ ] Vitest covers the registry (lookups, fallback, every descriptor well-formed), the targets picker's mixing rules and payload mapping, and the conditional Platform options section.
- [ ] Vitest payload-equality cases (deep-equal `data` objects, as defined in Required design 3) for: GitHub repository pool, GitHub organization runner-group pool, GitHub repositories plus Bitbucket workspaces, and pool-settings saves for a repository pool and an organization pool.
- [ ] Vitest (or the Playwright walkthrough) covers the Connections page's permission gating — Add, Edit and Remove are hidden when the descriptor reports the user cannot manage, shown when it can — and the post-create one-time reveal dialog, driven by a test-only descriptor whose post-create hook returns a value: the value is shown once after create and not again after the dialog closes.
- [ ] `docs/handoffs/145/survey.md` exists and contains the GitLab forward-compat table described in the Forward-compat section.
- [ ] `npm run lint`, `npm run typecheck`, `npm test` and `npm run build` pass.
- [ ] A headless Playwright walkthrough exercises every control on the Connections page and on the new-pool and pool-settings forms (no visible browser windows).

### Handoff locations
Write handoffs under `docs/handoffs/145/` (the run directory; the survey is `docs/handoffs/145/survey.md`).

### Operating rules
- Read the survey + Reuse map before writing code; extend existing components (searchable select, settings layout, BitbucketSettings logic) rather than building parallel ones.
- Implement against the failing tests from Phase 5 — F writes no tests.
- Follow project conventions; read source before referencing types/APIs.
- Stage files by explicit path, never `git add .`.
- All `gh` calls pass `--repo Performant-Labs/gridops`; never touch upstream iamngoni/gridops.

## Source material read
- `docs/handoffs/145-brief.md`
- `src/routes/settings.github.tsx`
- `src/routes/settings.integrations.tsx`
- `src/routes/_app/platform-connections.tsx`
- `src/routes/settings.tsx`
- `src/routes/settings.index.tsx`
- `src/routes/_app/runner-pools.new.tsx`
- `src/routes/_app/runner-pools.$poolId.settings.tsx`
- `src/features/runner-pools/tart-base-vm-row.tsx`
- `src/features/runner-pools/schemas.ts`
- `src/features/runner-pools/runner-pools.functions.ts`
- `src/features/runners/runner-row.tsx`
- `src/features/platform-connections/platform-connections.functions.ts`
- `src/features/operations/operations.functions.ts`
- `src/components/ui/searchable-select.tsx`
- `src/components/ui/searchable-multi-select.tsx`
- `src/components/pool-plan-preview.tsx` (line 50, via search)
- `src/lib/api.ts`
- `tests/searchable-select.test.ts`
- `docs/handoffs/145/survey.md` — not found (to be created by this story)

## Architecture decisions (resolve the round-1 Phase-4 BLOCK; these are binding for D, T and F)

### Reuse map (EXTEND / NEW per surface)
| Surface | Decision | Object |
|---|---|---|
| One-time reveal dialog | **EXTEND** (extract) | `SecretDialog` in `src/features/fleet/api-tokens-section.tsx:182-226`. Extract it to `src/components/secret-reveal-dialog.tsx` with generic props (`open`/`value`, title, description, secret label, optional details slot, close-button text, `onClose`, `onClosed`). API tokens and the Connections page both use it. Behaviour is unchanged: **Escape and outside clicks are ignored**, the only way out is the explicit button, the secret leaves the DOM when it leaves state, and it is never kept in a react-query cache. `tests/api-tokens-section.test.ts` ("shows the secret once, ignores Escape, and forgets it on close") must stay green unmodified in intent. The wireframe's C4 note "(or Esc)" is wrong: only the button closes it. |
| Bitbucket label rule / connection validation | **EXTEND** | `validateBitbucketConnections` in `src/features/runner-pools/schemas.ts:168-183` stays the submit-time authority. Export the label predicate (e.g. `bitbucketLabelError(labels): string \| null`) from `schemas.ts` and have the Bitbucket descriptor's `labelRules.validate` call it. No copy of the regex anywhere else. |
| Grouped multi-select | **EXTEND** | `SearchableMultiSelect` (`src/components/ui/searchable-multi-select.tsx`) gets **optional** props only: option `group`, option `disabled` + `disabledReason`, option `icon`, option `locked`, and a per-group status slot (loading / error+retry / not connected). It does not import the registry and stays generic; existing fleet callers need no changes. Its single `maxSelected` is NOT used for per-kind caps. |
| Settings shell, rows, callouts | **EXTEND** | `SettingsLayout` / `SettingsSection` / `SettingsRow` (`src/components/settings-ui.tsx`), `Callout`, `Badge`, `EmptyState`, `useAction` confirm. |
| Bitbucket connection list/edit/remove | **EXTEND** (move) | `BitbucketSettings` / `ConnectionRow` logic from `src/routes/settings.integrations.tsx` moves into the Connections page, driven by the Bitbucket descriptor's fields and bindings. |
| GitHub App + installations sections | **EXTEND** (move) | The sections of `src/routes/settings.github.tsx` move verbatim to `src/features/platform-connections/github-app-section.tsx`, receiving settings data from the route through props. |
| Tart base VM row | **EXTEND** | `src/features/runner-pools/tart-base-vm-row.tsx` loses its `bitbucket` prop; the Bitbucket Tart image script moves to the Bitbucket descriptor's platform-options content. |
| Plan preview | **EXTEND** | `src/components/pool-plan-preview.tsx` takes the target sentence as a prop (computed from target kinds) instead of branching on `scope`; it never imports the registry. |
| Platform registry | **NEW** | `src/features/platform-connections/registry.tsx` (type `PlatformDescriptor`, `platforms` list, `getPlatform(id)` with fallback); one module per descriptor: `github.tsx`, `bitbucket.tsx`. #141 adds `gitlab.tsx` and one line in the list. |
| Targets rules + payload builder | **NEW** | `src/features/runner-pools/targets.ts` — pure functions (no React): selection validity, mixing rules, per-kind limits, payload building. |
| Targets row | **NEW** | `src/features/runner-pools/targets-row.tsx`, shared by `runner-pools.new.tsx` and `runner-pools.$poolId.settings.tsx`. |
| Platform options section | **NEW** | `src/features/runner-pools/platform-options-section.tsx`, shared by both forms. |
| Connections page | **NEW** route, reusing the above | `src/routes/settings.connections.tsx`; old routes become redirect-only. |

### Target-kind contract (replaces the loose "how a target maps onto the payload" wording)
Each `TargetKind` in a descriptor declares:
- `id` (e.g. `github.repository`, `github.runner-group`, `bitbucket.workspace`), `label`, `group` heading;
- **namespaced option keys**: an option's value is `"<kindId>:<nativeId>"` (repository ids and runner-group ids are both numbers and GitHub's default runner group is id 1 in every org, so raw ids collide);
- `cardinality`: `"many"` or `"one"` (the organization runner group is single-select, with today's manual "Runner group ID" fallback for user-account installations);
- `limit(context)`: per-kind cap — repositories ≤ `maxCount`, workspaces ≤ `maxCount`, separately (as today);
- `excludes`: kinds that cannot be selected together (GitHub repository ⟂ GitHub runner group; a repository selection also pins one installation as today);
- `requires`/`standalone`: Bitbucket workspace is **not standalone** — it requires a GitHub target, because `createRunnerPoolSchema` requires `installationId > 0` and a `scope`. A Bitbucket-only pool stays impossible in this story; the picker shows Bitbucket options as disabled-with-reason until a GitHub target is chosen. (Wireframe D5's "If only one platform has targets, the page renders" must be corrected: with no GitHub installation the form shows the existing "Connect GitHub first" / "Install the GitHub App" empty state, now worded neutrally via the descriptor.)
- `load(context)`: takes the already-loaded data (`getCreateRunnerPoolOptions()` result, or the pool detail on settings) and any lazily-fetched lists (repositories, runner groups — today's queries); no duplicate fetching of installations or workspaces.

Each `PlatformDescriptor` declares `toCreatePayload(selection, context)` and `toUpdatePayload(selection, pool, context)` returning **partials**; one neutral builder in `targets.ts` merges them with the shared rows. GitHub's partials reproduce today's exact fields, including defaults on an empty or runner-group selection: create sends `installationId`, `scope`, `repositoryIds: []` for organization, and `runnerGroupId` (`runnerGroupId || defaultRunnerGroup?.id || 1` for organization, `1` for repository); update omits `installationId`, `scope` and `bitbucketConnectionIds`, sends `repositoryIds`/`priorityRepositoryIds`/`priorityWaitMinutes` only for repository pools (else `undefined`), and `runnerGroupId: runnerGroupId || pool.runnerGroupId` for organization pools, else `1`. Bitbucket's create partial is `bitbucketConnectionIds`; its update partial is empty (pool settings shows Bitbucket targets as locked, display-only chips — the old settings form never sent them). A descriptor flag `supportsPriority` on the GitHub repository kind drives the Priority section in pool settings.

### Import direction
Descriptors import only `runner-pools.functions.ts`, `platform-connections.functions.ts`, `operations.functions.ts` and type-only `schemas.ts`. Shared UI components (`SearchableMultiSelect`, `pool-plan-preview`, `secret-reveal-dialog`) take values as props and never import the registry. No file-level import cycles.

### Connections page loading
Route loader uses `Promise.allSettled` per platform so one platform's load failure shows a per-platform error with Retry and does not hide the others; mutations call `router.invalidate()`. GitHub data reuses the `/settings` loader data (`getSettingsPage()`), not a second fetch. `canManage` comes from loaded data. The page component takes the descriptor list as a prop (default: the production `platforms`) so tests can inject a test-only descriptor with a post-create hook without adding it to production.

### Other sites now in scope
- `src/routes/_app/webhooks.tsx:83` links to `/settings/github` → repoint to `/settings/connections` (no redirect hop).
- `ReadOnlyPool` in `runner-pools.$poolId.settings.tsx` branches on `pool.scope` → render its targets through the same registry-driven target display (read-only).
- Update `src/components/pool-plan-preview.tsx` line 50 as per the Reuse map.

### Tests known to break (T must update them in the RED phase, not F)
- `tests/runner-pool-tart-image.test.ts` uses the "Repositories" and "Bitbucket Cloud workspaces" controls, and its case at ~lines 184-189 expects the Bitbucket script in the Tart base VM row; it must move to the Targets row and the Platform options section.
- `tests/api-tokens-section.test.ts` must keep passing after the `SecretDialog` extraction.

### Forward-compat note
The forward-compat table in `docs/handoffs/145/survey.md` must also state plainly that a GitLab pool will still need `schemas.ts` and API changes (#137), and that the descriptor contract above (namespaced keys, per-platform payload partials, `requires`/`standalone`, per-kind limits, platform-options slot, post-create hook, runner link builder) is what lets #141 add GitLab without touching the page, picker or options components.
