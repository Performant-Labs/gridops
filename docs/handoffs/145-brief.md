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
- [ ] No in-app link or redirect targets a redirecting route: the Bitbucket/Targets "connect first" link in `runner-pools.new.tsx` (today line 368, `to="/settings/integrations"`), the webhooks empty-state link in `src/routes/_app/webhooks.tsx` (line 83, `to="/settings/github"`), the `settings.index.tsx` `?appError=` redirect (line 10) and the settings nav (`settings.tsx` lines 41-42) all point directly at `/settings/connections`. Verified by a Vitest assertion on the rendered `href`/redirect target, plus a grep over `src/` showing no remaining `"/settings/github"` or `"/settings/integrations"` string outside the two redirect-only route files.
- [ ] The new-pool form and pool settings have no platform-specific rows outside Targets and Platform options (no `bitbucket`/`github` branching in shared rows).
- [ ] `runner-row.tsx` reads its platform badge from the registry.
- [ ] Vitest covers the registry (lookups, fallback, every descriptor well-formed), the targets picker's mixing rules and payload mapping, and the conditional Platform options section. The mixing-rules cases include at least: (a) with no GitHub target selected, every Bitbucket workspace option is disabled and shows a reason (Bitbucket is not standalone), and becomes selectable once a GitHub repository or runner group is selected; (b) selecting a GitHub repository disables runner-group options and vice versa; (c) the per-kind caps are separate (repositories ≤ `maxCount` and workspaces ≤ `maxCount`, each counted on its own); (d) picking a Bitbucket workspace adds the `tart` provider, as today (`runner-pools.new.tsx` line 378); (e) selecting a GitHub repository, then a Bitbucket workspace, then removing the repository leaves the workspace selected, and submitting sends no `POST /api/v1/runner-pools` (the stubbed `fetch` records none) and shows the parse error in the "Could not create the pool" callout, as the Target-kind contract's `requires`/`standalone` entry states.
- [ ] Vitest covers partial load failure on the Connections page: when one platform's list binding rejects and the other resolves, the page still renders the resolved platform's connections, shows a per-platform error with a Retry control for the failed one, and Retry re-runs only that load.
- [ ] Vitest payload-equality cases (deep-equal `data` objects, as defined in Required design 3) for: GitHub repository pool, GitHub organization runner-group pool, GitHub repositories plus Bitbucket workspaces, and pool-settings saves for a repository pool and an organization pool. The expected objects are the frozen old-form constants described in "Payload-equality reference (frozen old-form output)" below, not values computed by the new code.
- [ ] Vitest (or the Playwright walkthrough) covers the Connections page's permission gating — Add, Edit and Remove are hidden when the descriptor reports the user cannot manage, shown when it can — and the post-create one-time reveal dialog, driven by a test-only descriptor whose post-create hook returns a value: the value is shown once after create and not again after the dialog closes.
- [ ] `docs/handoffs/145/survey.md` exists and contains the GitLab forward-compat table described in the Forward-compat section.
- [ ] `npm run lint`, `npm run typecheck`, `npm test` and `npm run build` pass.
- [ ] A headless Playwright walkthrough exercises every control on the Connections page and on the new-pool and pool-settings forms (no visible browser windows).

### Handoff locations
Write handoffs under `docs/handoffs/145/` (the run directory; the survey is `docs/handoffs/145/survey.md`).

### Operating rules
- Read the survey + Reuse map before writing code; extend existing components (searchable select, settings layout, BitbucketSettings logic) rather than building parallel ones.
- Implement against the failing tests from Phase 5 — F writes no tests. Handoff path for those tests: T writes them in this same worktree and branch (`claude/ui-145-platform-neutral-cf5832`), as `tests/**/*.test.ts` files (the only pattern Vitest collects, per `vitest.config.ts` `include: ["tests/**/*.test.ts"]`), together with any fixtures under `tests/fixtures/`, and commits them before F starts; T's handoff note goes under `docs/handoffs/145/` (see Handoff locations) and lists the test files. F reads them from the branch; F must not edit them, including the frozen old-form payload constants. T captures those constants before any `src/` change, per the "Sequencing gate" in "Payload-equality reference (frozen old-form output)"; T's commits change nothing under `src/`.
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
- `src/components/pool-plan-preview.tsx` (read in full in this amendment pass)
- `src/lib/api.ts`
- `tests/searchable-select.test.ts`
- `docs/handoffs/145/survey.md` — not found (to be created by this story)
- Added in round-2 amendment pass: `src/routes/settings.tsx` (full), `src/routes/settings.github.tsx` (full), `src/components/pool-plan-preview.tsx` (full), `src/features/runner-pools/runner-pools.functions.ts` (full), `src/lib/api.ts` (full), `src/features/operations/operations.functions.ts` (lines 130-235), `src/routes/_app/runner-pools.new.tsx` (lines 120-280 and 360-390, plus grep for the submit button and error callout, lines 489 and 494), `src/features/runner-pools/schemas.ts` (lines 120-194), `tests/runner-pool-tart-image.test.ts` (lines 1-110), `package.json` (`zod` version), `src/routes/settings.integrations.tsx` (line 17), and `git diff 5550cf5 HEAD --stat` (only `docs/handoffs/` changed).
- Added in the round-1 amendment pass (read in full unless noted): `src/routes/_app/webhooks.tsx` (lines 1-90), `src/components/settings-ui.tsx`, `src/features/fleet/api-tokens-section.tsx` (lines 1-60 and 170-230, covering `SecretDialog`), `tests/api-tokens-section.test.ts` (test names, and lines 140-182), `tests/runner-pool-tart-image.test.ts`, `vitest.config.ts`, `package.json` (scripts), `docs/handoffs/145/decisions.md`, `docs/handoffs/145/handoff-A.md`, `docs/handoffs/145/handoff-D.md`, `docs/handoffs/145/wireframe.html` (grep for C4 / Esc only). Not found (to be created by this story): `src/components/secret-reveal-dialog.tsx`, `src/features/platform-connections/registry.tsx`, `github.tsx`, `bitbucket.tsx`, `github-app-section.tsx`, `src/features/runner-pools/targets.ts`, `targets-row.tsx`, `platform-options-section.tsx`, `src/routes/settings.connections.tsx`.

## Architecture decisions (resolve the round-1 Phase-4 BLOCK; these are binding for D, T and F)

### Reuse map (EXTEND / NEW per surface)
| Surface | Decision | Object |
|---|---|---|
| One-time reveal dialog | **EXTEND** (extract) | `SecretDialog` in `src/features/fleet/api-tokens-section.tsx:182-226`. Extract it to `src/components/secret-reveal-dialog.tsx` with generic props (`open`/`value`, title, description, secret label, optional details slot, close-button text, `onClose`, `onClosed`). API tokens and the Connections page both use it. Behaviour is unchanged: **Escape and outside clicks are ignored**, the only way out is the explicit button, the secret leaves the DOM when it leaves state, and it is never kept in a react-query cache. `tests/api-tokens-section.test.ts` ("shows the secret once, ignores Escape, and forgets it on close") must stay green unmodified in intent. The wireframe's C4 note "(or Esc)" is wrong: only the button closes it. (The wireframe `docs/handoffs/145/wireframe.html` is the Phase-3 historical record and is not edited in this story; where it and this brief disagree, this brief wins.) |
| Bitbucket label rule / connection validation | **EXTEND** | `validateBitbucketConnections` in `src/features/runner-pools/schemas.ts:168-183` stays the submit-time authority. Export the label predicate (e.g. `bitbucketLabelError(labels): string \| null`) from `schemas.ts` and have the Bitbucket descriptor's `labelRules.validate` call it. No copy of the regex anywhere else. |
| Grouped multi-select | **EXTEND** | `SearchableMultiSelect` (`src/components/ui/searchable-multi-select.tsx`) gets **optional** props only: option `group`, option `disabled` + `disabledReason`, option `icon`, option `locked`, and a per-group status slot (loading / error+retry / not connected). It does not import the registry and stays generic; existing fleet callers need no changes. Its single `maxSelected` is NOT used for per-kind caps. |
| Settings shell, rows, callouts | **EXTEND** | `SettingsLayout` / `SettingsSection` / `SettingsRow` (`src/components/settings-ui.tsx`), `Callout`, `Badge`, `EmptyState`, `useAction` confirm. |
| Bitbucket connection list/edit/remove | **EXTEND** (move) | `BitbucketSettings` / `ConnectionRow` logic from `src/routes/settings.integrations.tsx` moves into the Connections page, driven by the Bitbucket descriptor's fields and bindings. |
| GitHub App + installations sections | **EXTEND** (move) | The sections of `src/routes/settings.github.tsx` move verbatim to `src/features/platform-connections/github-app-section.tsx`, receiving settings data from the route through props. |
| Tart base VM row | **EXTEND** | `src/features/runner-pools/tart-base-vm-row.tsx` loses its `bitbucket` prop; the Bitbucket Tart image script moves to the Bitbucket descriptor's platform-options content. |
| Plan preview | **EXTEND** | `src/components/pool-plan-preview.tsx` takes the target sentence as a prop (computed from target kinds) instead of branching on `scope`; it never imports the registry. |
| Platform registry | **NEW** | `src/features/platform-connections/registry.tsx` (`platforms` list, `getPlatform(id)` with fallback); one module per descriptor: `github.tsx`, `bitbucket.tsx`; descriptor types in `types.ts`; Connections loading in `connections-section.tsx`. #141 adds `gitlab.tsx` and one line in the list. |
| Targets rules + payload builder | **NEW** | `src/features/runner-pools/targets.ts` — pure functions (no React): selection validity, mixing rules, per-kind limits, payload building. |
| Targets row | **NEW** | `src/features/runner-pools/targets-row.tsx`, shared by `runner-pools.new.tsx` and `runner-pools.$poolId.settings.tsx`. |
| Platform options section | **NEW** | `src/features/runner-pools/platform-options-section.tsx`, shared by both forms. |
| Connections page | **NEW** route, reusing the above | `src/routes/settings.connections.tsx`; old routes become redirect-only. |

### Target-kind contract (replaces the loose "how a target maps onto the payload" wording)
Each `TargetKind` in a descriptor declares:
- `id` (e.g. `github.repository`, `github.runner-group`, `bitbucket.workspace`), `label`, `group` heading;
- **namespaced option keys**: an option's value is `"<kindId>:<nativeId>"` (repository ids and runner-group ids are both numbers and GitHub's default runner group is id 1 in every org, so raw ids collide). Evidence status: the collision risk follows from both id spaces being numbers alone (`RepositoryOption.id: number`, `RunnerGroupOption.id: number` in `runner-pools.functions.ts` lines 18-32). The "default group is id 1 in every org" part is GitHub behaviour that this repository cannot prove; what the source does show is that GridOps uses 1 as its fallback runner-group id (`schemas.ts` line 30 `runnerGroupId: z.number().int().positive().default(1)`; `runner-pools.new.tsx` line 262 `runnerGroupId || defaultRunnerGroup?.id || 1`; settings line 143 `: 1`), and A recorded it as an assumption in `docs/handoffs/145/decisions.md`. Namespacing is required either way;
- `cardinality`: `"many"` or `"one"` (the organization runner group is single-select, with today's manual "Runner group ID" fallback for user-account installations);
- `limit(context)`: per-kind cap — repositories ≤ `maxCount`, workspaces ≤ `maxCount`, separately (as today);
- `excludes`: kinds that cannot be selected together (GitHub repository ⟂ GitHub runner group; a repository selection also pins one installation as today — precisely: the create payload carries one `installationId`, taken from the first selected repository, `selectedRepositories[0]?.installationId ?? 0` (`runner-pools.new.tsx` lines 233-235), while repositories from different installations/accounts may still be selected together; the new form must reproduce exactly that, not add a stricter single-installation rule);
- `requires`/`standalone`: Bitbucket workspace is **not standalone** — it requires a GitHub target, because `createRunnerPoolSchema` requires `installationId > 0` and a `scope`. A Bitbucket-only pool stays impossible in this story; the picker shows Bitbucket options as disabled-with-reason until a GitHub target is chosen. **When the last GitHub target is removed while Bitbucket workspaces are selected** (new-pool form), the behaviour is today's, unchanged: the picker does not block the deselection and does not drop the selected workspaces (they stay selected; unselected Bitbucket options become disabled-with-reason again), and the submit button stays enabled as today (`runner-pools.new.tsx` line 494 disables it only on `submitting || options.installations.length === 0`). Submit builds the same payload the old form builds for that state — no repository selected, so `scope: "repository"`, `installationId: selectedRepositories[0]?.installationId ?? 0` = `0`, `repositoryIds: []`, `runnerGroupId: 1`, plus the selected `bitbucketConnectionIds` — and passes it to `createRunnerPoolAction`, whose `parseCreateRunnerPoolInput` (`schemas.ts` lines 160-166) rejects it (`installationId: z.number().int().positive()`, line 127) and throws before `api()` is called, so no POST is sent; the form shows the thrown message in its existing "Could not create the pool" callout (line 489). `schemas.ts` stays the submit-time authority; the picker adds no second validator for this state. Pool settings cannot reach this state: its Bitbucket targets are locked display-only chips and its GitHub kind is fixed by `pool.scope`. (Wireframe D5's "If only one platform has targets, the page renders" must be corrected: with no GitHub installation the form shows the existing "Connect GitHub first" / "Install the GitHub App" empty state, now worded neutrally via the descriptor.)
- `load(context, signal)`: **async and owned by the kind** (the descriptor module), not by `targets-row.tsx`. `context` carries the already-loaded data (`getCreateRunnerPoolOptions()` result, or the pool detail on settings) so installations/workspaces are never fetched twice; kinds whose lists are lazy (GitHub repositories, GitHub runner groups) call today's functions from `runner-pools.functions.ts` inside their own `load`. `targets-row.tsx` keeps **one load state per kind** (loading / error+retry / not connected) and contains no platform-specific code, so #141's GitLab lists need no picker change;
- `manualEntry` (optional, kind-level): `{ label, parse(input): nativeId | null }` — carries today's "Enter runner group ID" fallback for user-account installations on the `github.runner-group` kind;
- `requiresProviders` (optional): providers the kind forces on (Bitbucket workspace → `["tart"]`, preserving the form's auto-add of `tart` at `runner-pools.new.tsx` line 378);
- `planNote` (optional): the sentence the plan preview shows for the kind (feeds `targetPlanCheck`).

Each `PlatformDescriptor` declares `toCreatePayload(selection, context)` and `toUpdatePayload(selection, pool, context)` returning **partials**; one neutral builder in `targets.ts` merges them with the shared rows. The builder calls **every** registered descriptor's partial, whether or not that platform has a selected target, because today's payload always carries `bitbucketConnectionIds` (`[]` when none) and GitHub's empty-selection defaults. The update `context` carries the pool-settings form values the GitHub update partial needs: `priorityRepositoryIds` and `priorityWaitMinutes` (and `runnerGroupId`). GitHub's partials reproduce today's exact fields, including defaults on an empty or runner-group selection: create sends `installationId`, `scope`, `repositoryIds: []` for organization, and `runnerGroupId` (`runnerGroupId || defaultRunnerGroup?.id || 1` for organization, `1` for repository); update omits `installationId`, `scope` and `bitbucketConnectionIds`, sends `repositoryIds`/`priorityRepositoryIds`/`priorityWaitMinutes` only for repository pools (else `undefined`), and `runnerGroupId: runnerGroupId || pool.runnerGroupId` for organization pools, else `1`. Bitbucket's create partial is `bitbucketConnectionIds`; its update partial is empty (pool settings shows Bitbucket targets as locked, display-only chips — the old settings form never sent them). A descriptor flag `supportsPriority` on the GitHub repository kind drives the Priority section in pool settings.

### Import direction
The descriptor types (`PlatformDescriptor`, `TargetKind`, `ConnectionField`, payload partial types) live in `src/features/platform-connections/types.ts`; `registry.tsx` imports the descriptor modules and they import only `types.ts`, so there is no file-level back-edge. Descriptors import only `types.ts`, `runner-pools.functions.ts`, `platform-connections.functions.ts`, `operations.functions.ts`, and from `schemas.ts` types plus the exported Bitbucket label predicate (a value import, explicitly allowed). Shared UI components (`SearchableMultiSelect`, `pool-plan-preview`, `secret-reveal-dialog`) take values as props and never import the registry. No file-level import cycles.

### Connections page loading
**(Round-2 Phase-4 decision; supersedes the earlier route-loader wording.)** `src/routes/settings.connections.tsx` has **no route loader** and stays thin, like `settings.api-tokens.tsx`: it reads `getRouteApi("/settings").useLoaderData()` and composes `SettingsLayout`, `ConnectionsSection` and `GitHubAppSection`.
- Loading lives in `src/features/platform-connections/connections-section.tsx`. It takes `descriptors` (default: the production `platforms`) and the `/settings` page data as props. Tests render `ConnectionsSection` directly with an injected test-only descriptor and fixture page data; no router loaderData shape is involved.
- Each platform that has a list binding gets **its own load state**, using the existing component-level pattern: `useQuery` keyed per platform plus `InlineError onRetry={refetch}` (as in `src/features/fleet/api-tokens-section.tsx:229-241`). Retry re-runs only that platform's binding, and only that platform shows a loading row; one platform's failure never hides the others. `Promise.allSettled` is not used.
- GitHub rows are derived from the `/settings` page data props (`page.data.installations`) — no fetch, no Retry for GitHub.
- After a create/update/remove, reload only that platform via `useAction({ after })` (invalidate that platform's query key). `router.invalidate()` is still allowed to refresh `/settings` data where GitHub data changes (e.g. after "Refresh access").
- The create call and any post-create value (the one-time secret) are kept out of react-query: plain async call + component state, handed to the extracted secret-reveal dialog.
- `canManage` comes from each platform's loaded data (`getBitbucketConnections().canManage`; GitHub from `user.role`).
- The wireframe's B5b note that Retry "re-runs only that load" is consistent with this; its note B5 (a descriptor `settingsSection` slot for the GitHub App) is **superseded**: the GitHub App section is composed by the route, not hung off the descriptor.

**GitHub App section wiring (how its data arrives and how its actions are invoked).** Verified from `src/routes/settings.github.tsx`: the page has **no route loader and no route-level action functions**. It reads the parent `/settings` layout's loader data (`const settingsRoute = getRouteApi("/settings")`, line 24; `const page = settingsRoute.useLoaderData()`, line 27; the parent loader is `loader: () => getSettingsPage()` in `src/routes/settings.tsx` line 10), and its two actions are plain async calls made from components: `syncGitHubAction()` from the "Refresh access" `AsyncActionButton` (line 47: `action={() => syncGitHubAction()}`), and `createGitHubAppManifestAction({ data: { ownerType, organization, name } })` from `CreateAppSection`'s button (lines 133-155, which then builds and submits the manifest form to GitHub). Binding wiring after the move:
- `src/routes/settings.connections.tsx` (a child of `/settings`) reads `getRouteApi("/settings").useLoaderData()` exactly as `settings.github.tsx` does today and passes `page.data` (`configuration`, `githubApp`, `installations`, `user`) to `GitHubAppSection` as props. The section never calls `getRouteApi` itself. When `page.data` is null it keeps today's "GitHub unavailable" empty state (line 28) for the GitHub part only.
- `src/features/platform-connections/github-app-section.tsx` keeps importing `syncGitHubAction` and `createGitHubAppManifestAction` from `~/features/operations/operations.functions` and calls them from the same controls, with the same arguments, success text and error toast, moved verbatim. No new route action, server function or wrapper is introduced. Admin gating stays `user.role === "admin"` (line 30), passed in through props.
- `/settings/github` becomes redirect-only: `beforeLoad: () => { throw redirect({ replace: true, to: "/settings/connections" }) }`, with no loader and no component. Its data source is the `/settings` loader, which still runs for `/settings/connections`.
- The GitHub descriptor's list binding reads the same `page.data.installations` (no second `getSettingsPage()` fetch). Its `canManage` comes from `user.role` and each installation's `permission`, as today (lines 30, 76 and 95-97).
- Acceptance trace for "managing the GitHub App": a test or the Playwright walkthrough renders the Connections page with admin and member `page.data` fixtures and asserts (a) "Refresh access" calls `POST /api/v1/repositories/sync` (the `syncGitHubAction` endpoint, `operations.functions.ts` line 206); (b) when `configuration.githubAppControl` is false, an admin sees "Create the GitHub App" and submitting calls `POST /api/v1/github-app/manifest` with `{ ownerType, organization?, name? }` (lines 230-235); (c) a member sees the "Read-only access" callout and no Create section; (d) the Install / Manage App / Configure links carry `githubApp.installUrl`, `githubApp.appUrl` and `installation.manageUrl`.

**Plan preview contract (`src/components/pool-plan-preview.tsx`).** Current code, verified (lines 15-16, 23-24, 28-29 and 50):

    repositoryCount,
    scope,
    ...
    repositoryCount: number;
    scope: "repository" | "organization";
    ...
    const repositoryCapacityOk = scope === "organization" || repositoryCount <= maxCount;
    const ready = Boolean(normalizedName) && providers.length > 0 && repositoryCapacityOk;
    ...
    <PlanCheck ok={repositoryCapacityOk} text={scope === "organization" ? "GitHub runner-group access defines which repositories can use the pool." : `${repositoryCount} selected repositories fit within the ${maxCount}-runner maximum.`} />

The `scope` branch drives both the first check's text and its `ok`, and `ok` also feeds `ready` (the "Ready to save" / "Needs attention" badge). Change: replace the `repositoryCount` and `scope` props with one prop `targetCheck: { ok: boolean; text: string }`. Line 28 becomes `const repositoryCapacityOk = targetCheck.ok`, line 50 becomes `<PlanCheck ok={targetCheck.ok} text={targetCheck.text} />`, and the rest of the component (labels grid, desired-count check, footnote) is unchanged. `targetCheck` is computed by a pure function in `src/features/runner-pools/targets.ts` (e.g. `targetPlanCheck(selection, { maxCount })`), which asks the selected GitHub kind's descriptor for its sentence. It must return exactly today's values:
- GitHub runner-group selection (new form) or `pool.scope === "organization"` (settings): `{ ok: true, text: "GitHub runner-group access defines which repositories can use the pool." }`;
- GitHub repository selection, or **no GitHub target yet** (today's new form starts at `scope = "repository"`, `runner-pools.new.tsx` line 145, so an empty form shows the repository sentence with count 0): ``{ ok: repositoryCount <= maxCount, text: `${repositoryCount} selected repositories fit within the ${maxCount}-runner maximum.` }``, where `repositoryCount` is the number of selected repository targets (settings: `repositoryIds.length`);
- Bitbucket targets contribute no sentence and do not change `ok`, as today, where the preview ignores `bitbucketConnectionIds`.
Both call sites (`runner-pools.new.tsx` line 473, `runner-pools.$poolId.settings.tsx` line 298) pass `targetCheck` instead of `repositoryCount`/`scope`. Vitest covers `targetPlanCheck` for these three cases plus the over-cap repository case (`ok: false`).

### Verified source excerpts (the payload contract and the facts above, quoted from the current tree)
All excerpts are verbatim from the branch as of commit `5550cf5`; line numbers refer to that state.

**Update schema** (`src/features/runner-pools/schemas.ts` lines 97-104). The only target fields it accepts are optional `repositoryIds`, `priorityRepositoryIds`, `priorityWaitMinutes` and `bitbucketConnectionIds`. It has no `installationId` and no `scope`. `runnerGroupId` comes from `configurationShape` (line 30: `runnerGroupId: z.number().int().positive().default(1),`).

    export const updateRunnerPoolSchema = z
      .object({
        repositoryIds: repositoryIds.optional(),
        priorityRepositoryIds: repositoryIds.optional(),
        priorityWaitMinutes: z.number().int().min(1).max(1_440).optional(),
        bitbucketConnectionIds: bitbucketConnectionIds.optional(),
        ...configurationShape,
      })

**Create schema** (`schemas.ts` lines 125-132):

    export const createRunnerPoolSchema = z
      .object({
        installationId: z.number().int().positive(),
        repositoryIds: repositoryIds.default([]),
        bitbucketConnectionIds: bitbucketConnectionIds.default([]),
        scope: z.enum(["repository", "organization"]),
        ...configurationShape,
      })

**Old pool-settings submit** (`src/routes/_app/runner-pools.$poolId.settings.tsx` lines 122-144). This settles NV-4. Besides `poolId` (stripped by `updateRunnerPoolAction`, `runner-pools.functions.ts` line 133 `const { poolId, ...values } = data;`), the `data` object carries no `installationId`, no `scope` and no `bitbucketConnectionIds`. The form has no Bitbucket workspaces control: its only use of `bitbucketConnectionIds` is line 271 (`<TartBaseVmRow bitbucket={pool.bitbucketConnectionIds.length > 0} … />`).

    await updateRunnerPoolAction({
      data: {
        poolId: pool.id,
        repositoryIds: pool.scope === "repository" ? repositoryIds : undefined,
        priorityRepositoryIds: pool.scope === "repository" ? priorityRepositoryIds.filter((id) => repositoryIds.includes(id)) : undefined,
        priorityWaitMinutes: pool.scope === "repository" ? Number(form.get("priorityWaitMinutes")) : undefined,
        name: poolName,
        mode,
        provider: primaryProvider,
        providers,
        labels: labelList,
        ...poolImages({ provider: primaryProvider, providers, dockerImage, tartImage }),
        macosRuntime,
        desiredCount: Number(form.get("desiredCount")),
        minCount: Number(form.get("minCount")),
        maxCount: Number(form.get("maxCount")),
        autoscalingEnabled: form.get("autoscalingEnabled") === "on",
        queueScaleFactor: Number(form.get("queueScaleFactor")),
        idleTimeoutMinutes: Number(form.get("idleTimeoutMinutes")),
        cpuLimit: Number(form.get("cpuLimit")),
        memoryLimitMb: Number(form.get("memoryLimitMb")),
        runnerGroupId: pool.scope === "organization" ? runnerGroupId || pool.runnerGroupId : 1,
      },
    });

So the Target-kind contract's update wording is confirmed field by field. The repository pool sends `repositoryIds`, the filtered `priorityRepositoryIds` and `priorityWaitMinutes`, with `runnerGroupId: 1`. The organization pool sends those three as `undefined` (absent after `JSON.stringify`), with `runnerGroupId: runnerGroupId || pool.runnerGroupId`. `poolId` is part of the compared `data` object.

**Old new-pool submit, target fields** (`src/routes/_app/runner-pools.new.tsx` lines 233-237 and 261-263; the shared rows in between are lines 238-260):

    installationId: scope === "repository"
      ? selectedRepositories[0]?.installationId ?? 0
      : installationId,
    repositoryIds: scope === "repository" ? repositoryIds : [],
    bitbucketConnectionIds,
    ...
    runnerGroupId: scope === "organization"
      ? runnerGroupId || defaultRunnerGroup?.id || 1
      : 1,

`scope` is sent as-is (`scope,` line 239). Note that `bitbucketConnectionIds` is always present on create (`[]` when none are picked), and the native-runtime case sends `tartImage: ""` (`poolImages({ …, tartImage: macosRuntime === "native" ? "" : tartImage })`, line 251).

**Settings page data** (`src/features/operations/operations.functions.ts`; settles NV-1). Line 202 is `export const getSettingsPage = () => api<SettingsPage>("/api/v1/settings");`. `SettingsPage` (lines 136-184) is `{ authenticated: boolean; data: null | { configuration; githubApp; installations; manager; settings; user; users } }`, with these fields (lines 144-149 and 180):

    githubApp: null | { slug: string; appUrl: string; installUrl: string };
    installations: Array<{
      id: number; accountLogin: string; accountType: string; accountAvatarUrl: string | null;
      repositorySelection: string; permission: string; suspended: boolean; lastSyncedAt: string | null;
      poolCount: number; manageUrl: string;
    }>;
    ...
    user: { id: string; login: string; role: "admin" | "member" };

`syncGitHubAction` (line 206) is `() => api<{ repositories: number }>("/api/v1/repositories/sync", { method: "POST" })`. `createGitHubAppManifestAction` (lines 230-235) POSTs `{ ownerType: "user" | "organization"; organization?: string; name?: string }` to `/api/v1/github-app/manifest`.

**Bitbucket create return type** (`src/features/platform-connections/platform-connections.functions.ts` lines 17-21; settles NV-2). The return type has no secret field.

    export const createBitbucketConnection = (data: { name: string; workspace: string; accessToken: string }) =>
      api<Pick<BitbucketConnection, "id" | "provider" | "name" | "workspace" | "workspaceUuid">>(
        "/api/v1/platform-connections/bitbucket",
        { method: "POST", body: data },
      );

**`SearchableMultiSelect` props** (`src/components/ui/searchable-multi-select.tsx` lines 14-28; settles NV-5). Options are `SearchableSelectOption<TValue>` = `{ value: TValue; label: string; description?: string; keywords?: string[] }` (`searchable-select.tsx` lines 15-20), with no group, disabled or icon field. The listbox (lines 209-245) renders one flat list with no group headings.

    type SearchableMultiSelectProps<TValue extends SearchableSelectValue> = {
      options: Array<SearchableSelectOption<TValue>>;
      values: TValue[];
      onValueChange: (values: TValue[]) => void;
      ariaLabel: string;
      placeholder?: string;
      searchPlaceholder?: string;
      emptyMessage?: string;
      loading?: boolean;
      disabled?: boolean;
      maxSelected?: number;
      selectedNoun?: string;
      /** Offer selecting or clearing every option that matches the search. */
      selectAll?: boolean;
    };

**Platform text in `runner-row.tsx`** (settles NV-6). A grep for `platform` in `src/features/runners/runner-row.tsx` finds only lines 27-28, 121 and 147. Lines 27-28 are `MachineLink`'s unrelated boolean `platform` prop (OS/architecture detail), not a CI platform. Line 121 is the only CI-platform ternary: `const platform = runner.platform === "bitbucket" ? "Bitbucket" : "GitHub";`. Line 147 uses that variable in the delete confirmation: `` confirm: `Delete ${runner.name} from its provider and ${platform}?` ``. After the change, line 121's name comes from `getPlatform(runner.platform).name` (with the registry fallback for an unknown id), so the line-147 text is registry-driven too. Lines 27-28 are out of scope.

**Parent `/settings` loader** (`src/routes/settings.tsx` lines 9-10; settles NV-2). The settings layout route loads `getSettingsPage()` for every child route, including the new `/settings/connections`:

     9  export const Route = createFileRoute("/settings")({
    10    loader: () => getSettingsPage(),

**`settings.github.tsx` route and data source** (`src/routes/settings.github.tsx` lines 17 and 20-30; settles NV-3). The route declares only `component` (no `loader`, no `beforeLoad`, no action export); the page reads the parent loader through `getRouteApi("/settings")`; the two GitHub actions are imported functions called from components (line 47 and lines 136-140):

    17  import { createGitHubAppManifestAction, syncGitHubAction } from "~/features/operations/operations.functions";
    ...
    20  export const Route = createFileRoute("/settings/github")({
    21    component: GitHubSettings,
    22  });
    23
    24  const settingsRoute = getRouteApi("/settings");
    25
    26  function GitHubSettings() {
    27    const page = settingsRoute.useLoaderData();
    28    if (!page.data) return <EmptyState description="Sign in to manage the GitHub integration." icon={Github} title="GitHub unavailable" />;
    29    const { configuration, githubApp, installations, user } = page.data;
    30    const isAdmin = user.role === "admin";

    47        actions={githubApp ? <AsyncActionButton action={() => syncGitHubAction()} icon={<RefreshCw />} success="GitHub installations and repository access refreshed.">Refresh access</AsyncActionButton> : null}

   136        const setup = await createGitHubAppManifestAction({ data: {
   137          ownerType,
   138          organization: ownerType === "organization" ? organization.trim() : undefined,
   139          name: name.trim() || undefined,
   140        } });

**`PoolPlanPreview` props and target check** (`src/components/pool-plan-preview.tsx` lines 9-29 and 50; settles NV-4 and confirms the "Plan preview contract" quotes above):

     9  export function PoolPlanPreview({
    10    name,
    11    providers,
    12    labels,
    13    desiredCount,
    14    maxCount,
    15    repositoryCount,
    16    scope,
    17  }: {
    18    name: string;
    19    providers: Provider[];
    20    labels: string[];
    21    desiredCount: number;
    22    maxCount: number;
    23    repositoryCount: number;
    24    scope: "repository" | "organization";
    25  }) {
    26    const normalizedName = name.trim();
    27    const customLabels = labels.filter(Boolean);
    28    const repositoryCapacityOk = scope === "organization" || repositoryCount <= maxCount;
    29    const ready = Boolean(normalizedName) && providers.length > 0 && repositoryCapacityOk;
    ...
    50          <PlanCheck ok={repositoryCapacityOk} text={scope === "organization" ? "GitHub runner-group access defines which repositories can use the pool." : `${repositoryCount} selected repositories fit within the ${maxCount}-runner maximum.`} />

The file imports nothing from the registry or from `runner-pools` (its imports are `lucide-react`, `~/components/ui/badge` and `~/lib/utils`, lines 1-4).

**Pool actions** (`src/features/runner-pools/runner-pools.functions.ts` lines 127-139; settles NV-5). Both parse `data` with the zod schemas, then send it through `api()`:

   127  export function createRunnerPoolAction({ data }: { data: CreateRunnerPoolInput }) {
   128    const input = parseCreateRunnerPoolInput(data);
   129    return api<{ id: string; cannotRun?: CannotRun }>("/api/v1/runner-pools", { method: "POST", body: input });
   130  }
   131
   132  export function updateRunnerPoolAction({ data }: { data: UpdateRunnerPoolInput & { poolId: string } }) {
   133    const { poolId, ...values } = data;
   134    const input = parseUpdateRunnerPoolInput(values);
   135    return api<{ ok: true; configurationVersion: number; rollingReplacement: boolean }>(
   136      `/api/v1/runner-pools/${poolId}`,
   137      { method: "PUT", body: input },
   138    );
   139  }

**`api()` body encoding** (`src/lib/api.ts` lines 25-34; settles NV-6):

    25  export async function api<T>(path: string, options: ApiOptions = {}): Promise<T> {
    26    const headers = new Headers(options.headers);
    27    headers.set("Accept", "application/json");
    28    if (options.body !== undefined) headers.set("Content-Type", "application/json");
    29    const response = await fetch(path, {
    30      ...options,
    31      body: options.body === undefined ? undefined : JSON.stringify(options.body),
    32      credentials: "same-origin",
    33      headers,
    34    });

**`SettingsPage` head and `getSettingsPage`** (`src/features/operations/operations.functions.ts` lines 136-138 and 202; with the field excerpt under "Settings page data" above, settles NV-1):

   136  export type SettingsPage = {
   137    authenticated: boolean;
   138    data: null | {
   ...
   202  export const getSettingsPage = () => api<SettingsPage>("/api/v1/settings");

**References to the redirecting routes** (W-2). `grep -rn -e "settings/github" -e "settings/integrations" -e "/platform-connections" src` (unquoted, so it also catches template strings and route constants), excluding `routeTree.gen.ts` and `/api/v1/` paths, finds only: `settings.index.tsx:10`, `settings.tsx:41-42`, `_app/webhooks.tsx:83`, `_app/runner-pools.new.tsx:368`, the two route declarations (`settings.github.tsx:20`, `settings.integrations.tsx:21`), `_app/platform-connections.tsx:4,6`, and the module import path `~/features/platform-connections/platform-connections.functions` (`settings.integrations.tsx:17`, not a link). No link to them is built dynamically today. The acceptance grep is run in this unquoted form.

### Payload-equality reference (frozen old-form output)
This section defines what the payload-equality acceptance criterion compares against, so the comparison does not depend on the old rows still existing.
- **Where they live:** as inline `const` objects in the Phase-5 test file that holds the payload-equality cases (for example `tests/pool-target-payloads.test.ts`), one per case: GitHub repository create, GitHub organization runner-group create, GitHub repositories plus Bitbucket workspaces create, repository-pool settings save, and organization-pool settings save. Each case also holds its fixed inputs (the `getCreateRunnerPoolOptions()` or `RunnerPoolDetail` fixture, the selection, and the shared-row field values), so the new form can be driven with identical inputs. Shared fixtures may go in `tests/fixtures/`.
- **How they are derived (from pre-change behaviour only):** T captures them in the RED phase, before any route or component is changed. T renders the **old** forms at the pre-implementation state of this branch (today `5550cf5`) with the harness that `tests/runner-pool-tart-image.test.ts` already uses (lines 16-28 and 90-104: a mocked `@tanstack/react-router` serving `loaderData`, and a stubbed `fetch`). T drives the old controls ("Runner pool scope", "Repositories", "GitHub installation", "GitHub runner group", "Bitbucket Cloud workspaces", "Pool repositories"), submits, and records the `data` argument passed to `createRunnerPoolAction` / `updateRunnerPoolAction`. Recording works by `vi.mock` of `~/features/runner-pools/runner-pools.functions` that wraps the real functions and stores `data`. T pastes each recorded object into the test as the constant, with a comment naming the commit and the source lines quoted above. T cross-checks each constant by hand against the "Old new-pool submit" and "Old pool-settings submit" excerpts.
- **Rules:** the constants must never be generated by, imported from or recomputed with the new implementation (`targets.ts`, descriptors, the new forms). F must not edit them. A mismatch is fixed in the implementation, not in the constant. Comparison is Vitest `toEqual` on the `data` object (keys set to `undefined` count as absent, as Required design 3 states). `toEqual` is intentional: `toStrictEqual` must not be substituted, because it treats `{ repositoryIds: undefined }` and `{}` as different, while `JSON.stringify` (what `api()` sends) does not. The test file says so in a comment next to the assertions.
- **Sequencing gate (no circular dependency).** The capture needs the old forms, so it happens before any `src/` change on this branch, and it does not depend on the old forms surviving afterwards:
  1. Starting state, verified: at the branch tip `ff6583a`, `git diff 5550cf5 HEAD --stat -- src tests` is empty — the only commits after `5550cf5` touch `docs/handoffs/` — so the worktree's `src/` is still the pre-change code the excerpts below quote.
  2. T runs the capture first, against that unchanged `src/`, with a one-off capture harness (the driving-and-recording code described above). The harness is not committed as a collected test, because it drives the old controls and would fail once F removes them; only its output, the pasted constants with their commit/line comments, is committed in the payload-equality test file.
  3. T's Phase-5 commit(s) change nothing under `src/` (T writes tests only); `git diff 5550cf5 <T's last commit> -- src` must be empty. F makes no `src/` change until T's commits are on the branch (the Operating rules already require T to commit before F starts).
  4. If `src/` has already changed when T captures (for example a re-run of Phase 5 after F's work), T does not capture from the worktree: T checks out the pre-change tree separately, `git worktree add <scratch dir> 5550cf5` (detached, read-only use), runs the same capture harness there, and removes that worktree afterwards. The constants are always taken from the `5550cf5` source, whichever way it is reached.
  5. The excerpts in "Verified source excerpts" are the cross-check, not a substitute for the capture: they quote only the target fields and the shared-row expressions (`poolImages(...)`, label splitting, `Number(form.get(...))`), and the concrete values those produce depend on code and defaults the excerpts do not show. A constant that disagrees with the excerpts is a capture error and is re-captured, never hand-edited to match.

### Remaining GitHub couplings in shared rows (round-2 W5)
- Maximum count `min={Math.max(1, repositoryIds.length)}` (new form ~line 458, pool settings ~line 283) → derive from `targets.ts` (e.g. `minMaxCount(selection)` = max(1, number of selected targets of kinds whose `limit` is bound to `maxCount`)), preserving today's value for every GitHub/Bitbucket selection (today Bitbucket workspaces do not raise the minimum; keep that).
- Create button `disabled={submitting || options.installations.length === 0}` (~line 494) → derive from the standalone-target gate in `targets.ts` (disabled when no standalone kind has any loadable target), equal to today's value for the current platforms.

### Other sites now in scope
- `src/routes/_app/webhooks.tsx:83` links to `/settings/github` → repoint to `/settings/connections` (no redirect hop).
- `ReadOnlyPool` in `runner-pools.$poolId.settings.tsx` branches on `pool.scope` → render its targets through the same registry-driven target display (read-only).
- Update `src/components/pool-plan-preview.tsx` line 50 as per the Reuse map.

### Tests known to break (T must update them in the RED phase, not F)
- `tests/runner-pool-tart-image.test.ts` uses the "Repositories" and "Bitbucket Cloud workspaces" controls, and its case at ~lines 184-189 expects the Bitbucket script in the Tart base VM row; it must move to the Targets row and the Platform options section.
- `tests/api-tokens-section.test.ts` must keep passing after the `SecretDialog` extraction.

### Forward-compat note
The forward-compat table in `docs/handoffs/145/survey.md` must also state plainly that a GitLab pool will still need `schemas.ts` and API changes (#137), and that the descriptor contract above (namespaced keys, per-platform payload partials, `requires`/`standalone`, per-kind limits, platform-options slot, post-create hook, runner link builder) is what lets #141 add GitLab without touching the page, picker or options components.
