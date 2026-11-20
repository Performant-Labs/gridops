## Phase — UI: one Connections page and one Targets picker for every CI platform (#145)

**Issue:** Performant-Labs/gridops#145 (part of epic #136, wave 0)
**Branch:** claude/ui-145-platform-neutral-cf5832 (this worktree)
**Review-rigor:** in-session — operator decision 2026-11-20: the outside brief reviewer did not converge over 5 runs (repeated structural objections to test-first criteria); in-session A/T/U/S review, the up-front architecture review and the anti-duplication gate still run.
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
- **Connection API surface.** `platform-connections.functions.ts` exports `getBitbucketConnections()` → `{ canManage, items: BitbucketConnection[] }`, `createBitbucketConnection({ name, workspace, accessToken })` → `Pick<BitbucketConnection, "id" | "provider" | "name" | "workspace" | "workspaceUuid">`, `updateBitbucketConnection(id, { name, accessToken? })`, `removeBitbucketConnection(id)`. GitHub has no connection create/update/remove function: installations are read from `getSettingsPage()` (`data.installations`), are added through the external `githubApp.installUrl` link and managed through each installation's `manageUrl`; `syncGitHubAction()` refreshes them and `createGitHubAppManifestAction()` starts App setup (admin only, `user.role === "admin"`). So the GitHub descriptor binds list only (plus its existing external links), and the Bitbucket descriptor binds all four. Completeness, verified: `platform-connections.functions.ts` is 28 lines and its only exports are `type BitbucketConnection` (line 3), `getBitbucketConnections` (14), `createBitbucketConnection` (17), `updateBitbucketConnection` (24) and `removeBitbucketConnection` (27); `grep -rni "github" src/features/platform-connections/` matches nothing; and `src/features/operations/operations.functions.ts`'s GitHub mutations are `syncGitHubAction` (line 206) and `createGitHubAppManifestAction` (lines 230-235), neither of which creates, updates or removes an installation. Re-verified across all of `src/` in the round-2 brief-gate amendment pass: `grep -rnE "export (const|function|async function) [A-Za-z]*(Installation|GitHub|Github)" src` finds only `getInstallationRunnerGroups` and `getInstallationRepositories` (`runner-pools.functions.ts` lines 120 and 122, both GETs), `syncGitHubAction` (206) and `createGitHubAppManifestAction` (230); and `grep -rn "api/v1/[a-z-]*install" src` finds only those two GETs (`/api/v1/installations/${installationId}/runner-groups` and `/repositories`). No GitHub connection mutation exists anywhere in `src/`.
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
- [ ] No in-app link or redirect targets a redirecting route: the Bitbucket/Targets "connect first" link in `runner-pools.new.tsx` (today line 368, `to="/settings/integrations"`), the webhooks empty-state link in `src/routes/_app/webhooks.tsx` (line 83, `to="/settings/github"`), the `settings.index.tsx` `?appError=` redirect (line 10) and the settings nav (`settings.tsx` lines 41-42) all point directly at `/settings/connections`. Verified by a Vitest assertion on the rendered `href`/redirect target, plus a grep over `src/` showing no remaining `"/settings/github"` or `"/settings/integrations"` string outside the two redirect-only route files. The "two redirect-only route files" are exactly `src/routes/settings.github.tsx` and `src/routes/settings.integrations.tsx`, whose only permitted hits are their own `createFileRoute("/settings/github")` / `createFileRoute("/settings/integrations")` declarations. The exclusion is defined **by pattern, not by line number**: in those two files only, a hit is permitted exactly when the whole matching line matches the regex `^export const Route = createFileRoute\("/settings/(github|integrations)"\)\(\{$` (today that is line 20 of `settings.github.tsx` and line 21 of `settings.integrations.tsx`, verbatim `export const Route = createFileRoute("/settings/github")({` and `export const Route = createFileRoute("/settings/integrations")({`; after this story both files are redirect-only and their line numbers will change, which does not affect the check). Any other hit in those two files (a comment, a link, a second string) fails, like a hit anywhere else. **Declaration-line invariant (pinned, part of this criterion).** Because the exclusion matches the whole line, each of those two files must keep its route declaration as exactly one line with exactly this text — `export const Route = createFileRoute("/settings/github")({` in `settings.github.tsx` and `export const Route = createFileRoute("/settings/integrations")({` in `settings.integrations.tsx` — with no added or removed whitespace and `({` on the same line, while the rest of the route object (the redirect-only `beforeLoad`) follows on later lines. The repository has no code formatter that would rewrite that line (no Prettier, Biome or dprint configuration or dependency; `npm run lint` is ESLint only, `package.json` line 20: `"lint": "eslint src tests *.config.ts eslint.config.js --max-warnings=0"`), so the line changes only if someone edits it by hand. Evidence for that tooling claim is quoted verbatim under "Verified source excerpts → Formatter and lint tooling" (`package.json` scripts and full `devDependencies`, the whole `eslint.config.js`, and the empty formatter-config search, all re-read at tip `07768aa`): the lint script has no `--fix`, so ESLint reports but never rewrites a file, and the config enables only `eslint.configs.recommended`, `tseslint.configs.recommended`, the React-hooks rules and three named non-formatting rules, with no stylistic plugin. **No comment on the declaration line:** a trailing comment (e.g. `({ // redirect`) or any other text added to that line is also a change to it and fails the criterion; an explanatory comment, if wanted, goes on a separate line of those two files — but a comment containing `settings/github` or `settings/integrations` is itself a hit, so it must not name the path. A reformatted or split declaration makes the grep print a hit and fails the criterion; the fix is to restore the exact line, never to widen the exclusion regex. The check is `grep -rn -e "settings/github" -e "settings/integrations" src --exclude=routeTree.gen.ts | grep -v -E '^src/routes/settings\.(github|integrations)\.tsx:[0-9]+:export const Route = createFileRoute\("/settings/(github|integrations)"\)\(\{$'`, which must print nothing; the gitignored generated `src/routeTree.gen.ts` (`.gitignore` line 42) is also excluded (it is absent from the worktree today — `ls src/routeTree.gen.ts` reports no such file at tip `07768aa` — so the exclusion is defensive, for a checkout where the router plugin has generated it). The command is run from the repository (worktree) root, as its relative `src` argument and the `^src/routes/…` anchor of the second regex require. `--exclude=routeTree.gen.ts` is deliberately the base-name form: grep's `--exclude` glob is matched against each file's base name during recursion, so it excludes `src/routeTree.gen.ts` whether or not the file exists at run time (a path form such as `--exclude=src/routeTree.gen.ts` would not match a base name and must not be substituted); no other file named `routeTree.gen.ts` exists in the repository outside `node_modules` (verified: `find . -name routeTree.gen.ts -not -path "./node_modules/*"` prints nothing at tip `955098d`). The third redirect-only route, `src/routes/_app/platform-connections.tsx`, is **not** excluded: its redirect target changes from `"/settings/integrations"` to `"/settings/connections"`, so after the change it contains neither string. Its line-3 comment today reads, verbatim: `// Platform connections moved into Settings → Integrations; keep old links working.` — it names the "Settings → Integrations" page but not the path, so it does not match either grep pattern (`settings/github`, `settings/integrations`); the change updates it to name the Connections page (e.g. "Settings → Connections"). Every grep hit counts, comments included: a comment or string anywhere in `src/` (outside the two excluded route declarations and `routeTree.gen.ts`) that matches either pattern fails the criterion, so a new comment such as "redirects /settings/integrations" must not be written outside the two redirect-only files. The grep is the unquoted form given under "References to the redirecting routes". There is **no** `/api/v1/` exclusion for the two pass/fail patterns: no `/api/v1/` path in `src/` contains `settings/github` or `settings/integrations` (verified: `grep -rn "api/v1" src | grep -e "settings/github" -e "settings/integrations"` prints nothing), so every hit outside the two excluded route declarations and `routeTree.gen.ts` fails, whatever string it is in. The `/api/v1/` exclusion mentioned under "References to the redirecting routes" applies only to the `/platform-connections` review-aid pattern.
- [ ] Vitest asserts the `settings.index.tsx` `?appError=` path: calling the route's `beforeLoad` with `location.search = { appError: "<message>" }` and `preload: false` calls `toast.error("<message>")` once and throws a redirect whose target is `/settings/connections` with `replace: true` and no `search`; with `preload: true` it throws the same redirect without toasting; without `appError` it still redirects to `/settings/general` with `replace: true` — the unchanged fallback at `settings.index.tsx` line 12, verbatim `    throw redirect({ replace: true, to: "/settings/general" });` (the full file, lines 1-14, is quoted under "Other sites now in scope → `?appError=` handling"). (Behaviour as specified under "Other sites now in scope → `?appError=` handling".) That the Connections page does not consume `appError` is checked after the change by `grep -rn appError src`, which must match only `src/routes/settings.index.tsx` (today it matches exactly lines 7-9 of that file, verified at tip `955098d`; `tests/` has no match today) — so no other file, `src/routes/settings.connections.tsx` included, reads `appError`.
- [ ] The new-pool form and pool settings have no platform-specific rows outside Targets and Platform options (no `bitbucket`/`github` branching in shared rows). Meaning, precisely: "shared rows" are every row of the two forms other than the Targets row and the Platform options section (name, providers, mode, images, counts, CPU/memory, autoscaling, labels, Priority, plan preview), plus the shared components they render (`tart-base-vm-row.tsx`, `pool-plan-preview.tsx`). In `runner-pools.new.tsx`, `runner-pools.$poolId.settings.tsx` and those components, no expression compares against a platform id or kind id (`"github"`, `"bitbucket"`, `"github.repository"`, …), tests `pool.scope`, or tests a platform-named field (`bitbucketConnectionIds.length`, `scope === …`) to choose what a shared row renders; each listed baseline site under "Current platform-branching sites in shared rows" is gone from them. What a shared row needs per platform comes from a descriptor/kind property or a `targets.ts` result (e.g. `supportsPriority`, `minMaxCount`, `targetPlanCheck`, `labelRules`). Platform-specific logic is allowed inside the descriptor modules (`github.tsx`, `bitbucket.tsx`); `targets-row.tsx` and `platform-options-section.tsx` contain no platform-specific code (Target-kind contract); `targets.ts` and the generic picker logic may read kind properties such as `standalone`, `requires`, `excludes` and `limit`, which is not platform branching.
- [ ] `runner-row.tsx` reads its platform badge from the registry.
- [ ] Vitest covers the registry (lookups, fallback, every descriptor well-formed), the targets picker's mixing rules and payload mapping, and the conditional Platform options section. The mixing-rules cases include at least: (a) with no GitHub target selected, every Bitbucket workspace option is disabled and shows a reason (Bitbucket is not standalone), and becomes selectable once a GitHub repository or runner group is selected; (b) selecting a GitHub repository disables runner-group options and vice versa; (c) the per-kind caps are separate (repositories ≤ `maxCount` and workspaces ≤ `maxCount`, each counted on its own); (d) picking a Bitbucket workspace adds the `tart` provider, as today (`runner-pools.new.tsx` line 378); (e) selecting a GitHub repository, then a Bitbucket workspace, then removing the repository leaves the workspace selected, and submitting sends no `POST /api/v1/runner-pools` (the stubbed `fetch` records none) and shows the parse error in the "Could not create the pool" callout, as the Target-kind contract's `requires`/`standalone` entry states.
- [ ] Vitest covers the new-pool form's create-button gate (the standalone-target gate in `targets.ts`, see "Remaining GitHub couplings in shared rows"), asserting the derived disabled state equals today's `submitting || options.installations.length === 0` (`runner-pools.new.tsx` line 494) in at least these cases: (a) 0 installations, 0 Bitbucket connections → disabled; (b) 0 installations, ≥1 Bitbucket connection → disabled; (c) ≥1 installation, 0 Bitbucket connections → enabled; (d) ≥1 installation, ≥1 Bitbucket connection → enabled; (e) ≥1 installation with no target selected, and the "last GitHub target removed while Bitbucket workspaces stay selected" state → enabled; (f) while submitting → disabled. Availability is judged from the already-loaded `context` (the installation count in `context.options`, i.e. the route loader's `getCreateRunnerPoolOptions()` result, present before the form renders; "loaded" here never means a lazy, on-demand load state), never from whether a lazy repository or runner-group list has loaded.
- [ ] Vitest covers partial load failure on the Connections page: when one platform's list binding rejects and the other resolves, the page still renders the resolved platform's connections, shows a per-platform error with a Retry control for the failed one, and Retry re-runs only that load. Since GitHub's list binding is page-derived and cannot reject (see "Connections page loading → What 'list binding' means"), the test renders `ConnectionsSection` with an injected `descriptors` list containing two descriptors with fetching list bindings (one rejecting, one resolving; test-only descriptors, or the Bitbucket descriptor with a stubbed failing fetch plus a test-only resolving one) and asserts the call count of each binding before and after Retry; it also passes page data with ≥1 installation and the production GitHub descriptor, and asserts the GitHub rows render with no error or Retry control of their own. In addition, a **separate** case exercises the production failure path: it renders `ConnectionsSection` with the default production `platforms` list (GitHub + Bitbucket, no test-only descriptor), page data with ≥1 installation, and a stubbed `fetch` that rejects for `GET /api/v1/platform-connections/bitbucket` (the `getBitbucketConnections()` endpoint, `platform-connections.functions.ts` lines 14-15; `api()` also throws an `ApiError` on a non-2xx response, `src/lib/api.ts` lines 35-38, so a stubbed non-2xx response is an acceptable alternative), and asserts that the Bitbucket per-platform error with a Retry control renders, that the GitHub rows render with no error or Retry of their own, and that clicking Retry makes exactly one more request to that endpoint and no other request.
- [ ] Vitest payload-equality cases (deep-equal `data` objects, as defined in Required design 3) for: GitHub repository pool, GitHub organization runner-group pool, GitHub repositories plus Bitbucket workspaces, and pool-settings saves for a repository pool and an organization pool. The expected objects are the frozen old-form constants described in "Payload-equality reference (frozen old-form output)" below, not values computed by the new code. Every case drives the rendered form (not `targets.ts` or a descriptor partial alone) and compares the `data` argument recorded at `createRunnerPoolAction` / `updateRunnerPoolAction`. For the two pool-settings cases that recorded `data` object contains `poolId: pool.id` (added by the settings form itself, not by the builder; see "`poolId` on update" in the Target-kind contract), and the frozen constant contains the same `poolId`, so the comparison includes it. **When this criterion becomes checkable (deferred to Phase 5, by design):** the frozen constants do not exist at brief-gate time and this brief deliberately does not contain their values — they must come from the capture against the `5550cf5` source (step 5 of the Sequencing gate: hand-derived values are not acceptable, because the concrete shared-row values depend on code and defaults the excerpts do not show). What the brief fixes now is how they are derived, where they live and who may edit them; the criterion itself is verified from T's Phase-5 commit onward: (1) T's handoff note under `docs/handoffs/145/` names the payload-equality test file and lists the five constants with their commit/line comments, and records `git diff 5550cf5 <T's last commit> -- src` as empty; (2) each constant is cross-checked by hand against the target-field excerpts ("Old new-pool submit", "Old pool-settings submit"); (3) the cases pass after F's change with the constants untouched (`git diff <T's last commit> HEAD -- <that test file>` empty at the review gate). A reviewer at the brief gate checks the derivation rules, not constant values. **What is verifiable at the brief gate (the brief-gate property of this criterion):** the brief-gate check is that the "Payload-equality reference (frozen old-form output)" section exists and is internally consistent with this criterion and the excerpts, i.e. each of these holds by reading this brief alone: (A) the five cases named here (repository create, organization runner-group create, repositories plus Bitbucket workspaces create, repository-pool settings save, organization-pool settings save) are exactly the five constants listed under "Where they live"; (B) the provenance line ranges named in invariant (i) cover the quoted excerpts — `runner-pools.new.tsx` 233-263 spans "Old new-pool submit, target fields" (233-237 and 261-263, shared rows 238-260) and `runner-pools.$poolId.settings.tsx` 122-144 is exactly "Old pool-settings submit"; (C) the `undefined`-valued keys invariant (iii) names for the organization settings save (`repositoryIds`, `priorityRepositoryIds`, `priorityWaitMinutes`) are exactly the three `pool.scope === "repository" ? … : undefined` keys of that excerpt (its lines 251-253 as printed); (D) the derivation steps (capture against `5550cf5` before any `src/` change, `vi.mock` recording of `data`, no import from the new implementation, `toEqual` not `toStrictEqual`, F never edits the constants) are stated without contradiction between this criterion, "Payload-equality reference" and the Operating rules. The constant values themselves are, by design, checked only from T's Phase-5 commit onward by the invariants below. **Checkable invariants (the acceptance checks for this criterion from T's Phase-5 commit onward; the criterion stays in this list, its gate is Phase 5 and the review gate, not the brief gate):** (i) **provenance comment** — each of the five constants is immediately preceded by a comment naming commit `5550cf5`, the route file and the submit lines it was captured from (`runner-pools.new.tsx` lines 233-263 or `runner-pools.$poolId.settings.tsx` lines 122-144); (ii) **independence** — the payload-equality test file imports nothing from the new implementation's rule or descriptor modules: `grep -nE "runner-pools/targets|platform-connections/(registry|github|bitbucket|types)" <that test file>` prints nothing (it imports the route components it renders and the mocked `runner-pools.functions` only); (iii) **literal keys** — each constant contains every key the old form's `data` object literal writes, including the keys whose value is `undefined` in that case (written out as `key: undefined`, e.g. `repositoryIds: undefined`, `priorityRepositoryIds: undefined`, `priorityWaitMinutes: undefined` for the organization-pool settings save), exactly as recorded — not normalised through `JSON.stringify`, which would drop them. A key that the new code adds with the value `undefined` is, by the definition in Required design 3, not a difference (`api()` sends `JSON.stringify` of the object, which drops it), so `toEqual` ignoring it is the intended behaviour; any key the new code adds or drops with a defined value, or any differing defined value, fails `toEqual`; (iv) **reproducible capture record** — T's handoff note under `docs/handoffs/145/` records, for each case, the recorded `data` object printed with `util.inspect` (`node:util`, which keeps `undefined`-valued keys), the capture harness's full source text (as an indented block in the note, not as a file), the command used to run it, and the tip hash with the empty `git diff 5550cf5 <tip> --stat -- src tests` result, so a reviewer can re-run that harness in a `git worktree add <scratch dir> 5550cf5` checkout and compare its output with the constants; each constant must equal its recorded object.
- [ ] Vitest (or the Playwright walkthrough) covers the Connections page's permission gating — Add, Edit and Remove are hidden when the descriptor reports the user cannot manage, shown when it can — and the post-create one-time reveal dialog, driven by a test-only descriptor whose post-create hook returns a value: the value is shown once after create and not again after the dialog closes. Concretely, the test-only descriptor's `postCreate` returns an object typed as `PostCreateReveal` (imported from `src/features/platform-connections/types.ts`, shape `{ title: string; description: string; secretLabel: string; value: string; details?: ReactNode; closeButtonText?: string }`, per "Post-create hook contract" below) with a fixed `value` string; the test file must typecheck against that `types.ts` declaration (`npm run typecheck`), and it asserts that the fixed `value` is in the dialog after create, that Escape does not close the dialog, and that the `value` is absent from the DOM after the close button is clicked. A hook returning `null` shows no dialog.
- [ ] `src/features/platform-connections/types.ts` exports `PostCreateReveal` with exactly the shape given under "Post-create hook contract" — `{ title: string; description: string; secretLabel: string; value: string; details?: ReactNode; closeButtonText?: string }` (`ReactNode` from `react`), no field more or fewer, no field with another type or optionality — and declares the descriptor's optional `postCreate?: (created: TCreated) => PostCreateReveal | null | Promise<PostCreateReveal | null>`. This is enforced independently of the dialog test by a compile-time assertion that T writes in a Vitest file in Phase 5: `expectTypeOf<PostCreateReveal>().toEqualTypeOf<{ title: string; description: string; secretLabel: string; value: string; details?: ReactNode; closeButtonText?: string }>()` (Vitest's `expectTypeOf`, imported from `vitest`; the repository uses `vitest` `^4.0.18`, `package.json` line 70). `npm run typecheck` (`tsc --noEmit`, `package.json` line 19) checks it, because `tsconfig.json` `include` covers `tests/**/*.ts` (`tsconfig.json` line 2, verbatim under "Verified source excerpts → Test collection and typecheck scope"). Which command checks it, precisely: `expectTypeOf(...).toEqualTypeOf<…>()` is a type-level assertion — a mismatch is a TypeScript compile error in the test file, and at run time the call is a no-op — so the pass condition is `npm run typecheck` exiting 0. `npm test` (`vitest run`) does **not** check it: `vitest.config.ts` (quoted in full under that excerpt) sets no `typecheck` option, and Vitest strips types without type-checking when it runs a test; `npm test` only has to keep passing as for every other test file. **Testable invariant (stated independently of implementation state):** a committed `tests/**/*.test.ts` file contains exactly the assertion `expectTypeOf<PostCreateReveal>().toEqualTypeOf<{ title: string; description: string; secretLabel: string; value: string; details?: ReactNode; closeButtonText?: string }>()`, with `PostCreateReveal` imported from `src/features/platform-connections/types.ts`, `ReactNode` from `react` and `expectTypeOf` from `vitest`, and `npm run typecheck` exits 0. **Gates:** at the brief gate the verifiable property is that this expected type is identical, field by field, to the `PostCreateReveal` declaration under "Connections page loading → Post-create hook contract" and to the shape quoted in the post-create reveal criterion above (it is); the assertion itself is written by T in Phase 5 and the invariant is checked from T's commit onward and at the review gate. In the RED phase this assertion fails typecheck (the module does not exist yet); after F's change it must pass, with the assertion's expected type left exactly as written here.
- [ ] `docs/handoffs/145/survey.md` exists and contains the GitLab forward-compat table described in the Forward-compat section, plus the two statements required by the "Forward-compat note" at the end of this brief: that a GitLab pool will still need `schemas.ts` and API changes (#137), and which descriptor-contract features let #141 add GitLab without touching the page, picker or options components. Each table row names its descriptor field by the exact property name declared in `src/features/platform-connections/types.ts`.
- [ ] Vitest exercises the forward-compat claim with a test-only third descriptor (not a production GitLab descriptor; nothing GitLab is added to `registry.tsx`, per Scope): rendering `TargetsRow` and `PlatformOptionsSection` directly with an injected `descriptors` list of the production descriptors plus that test-only one (see "Registry access in the forms (injection point)"), the test asserts that its target kind's group heading and options appear in the Targets multi-select, and that once one of its targets is selected its platform-options content renders in the Platform options section — with no change to `targets-row.tsx`, `platform-options-section.tsx` or `connections-section.tsx` beyond what the GitHub/Bitbucket descriptors already need. Stated without reference to the two-descriptor implementation: those three files contain no platform-specific code, checked independently of the test by `grep -niE "github|bitbucket" src/features/runner-pools/targets-row.tsx src/features/runner-pools/platform-options-section.tsx src/features/platform-connections/connections-section.tsx`, which must print nothing (comments included).
- [ ] `npm run lint`, `npm run typecheck`, `npm test` and `npm run build` pass.
- [ ] A headless Playwright walkthrough exercises every control on the Connections page and on the new-pool and pool-settings forms (no visible browser windows).

### Handoff locations
Write handoffs under `docs/handoffs/145/` (the run directory; the survey is `docs/handoffs/145/survey.md`). The two paths are distinct on purpose: this brief is `docs/handoffs/145-brief.md` (hyphen, beside the run directory), while the survey and every other handoff of this story live inside the run directory `docs/handoffs/145/`; there is no `docs/handoffs/145-survey.md` and no `docs/handoffs/145/brief.md`.

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
- `docs/handoffs/145/survey.md` — not found, so not read (it is created by this story; listed here only to record that it does not exist yet)
- Added in round-2 amendment pass: `src/routes/settings.tsx` (full), `src/routes/settings.github.tsx` (full), `src/components/pool-plan-preview.tsx` (full), `src/features/runner-pools/runner-pools.functions.ts` (full), `src/lib/api.ts` (full), `src/features/operations/operations.functions.ts` (lines 130-235), `src/routes/_app/runner-pools.new.tsx` (lines 120-280 and 360-390, plus grep for the submit button and error callout, lines 489 and 494), `src/features/runner-pools/schemas.ts` (lines 120-194), `tests/runner-pool-tart-image.test.ts` (lines 1-110), `package.json` (`zod` version), `src/routes/settings.integrations.tsx` (line 17), and `git diff 5550cf5 HEAD --stat` (only `docs/handoffs/` changed).
- Added in the round-1 amendment pass (read in full unless noted): `src/routes/_app/webhooks.tsx` (lines 1-90), `src/components/settings-ui.tsx`, `src/features/fleet/api-tokens-section.tsx` (lines 1-60 and 170-230, covering `SecretDialog`), `tests/api-tokens-section.test.ts` (test names, and lines 140-182), `tests/runner-pool-tart-image.test.ts`, `vitest.config.ts`, `package.json` (scripts), `docs/handoffs/145/decisions.md`, `docs/handoffs/145/handoff-A.md`, `docs/handoffs/145/handoff-D.md`, `docs/handoffs/145/wireframe.html` (grep for C4 / Esc only). Not found (to be created by this story): `src/components/secret-reveal-dialog.tsx`, `src/features/platform-connections/registry.tsx`, `github.tsx`, `bitbucket.tsx`, `github-app-section.tsx`, `src/features/runner-pools/targets.ts`, `targets-row.tsx`, `platform-options-section.tsx`, `src/routes/settings.connections.tsx`.
- Added in the round-1 brief-review amendment pass (this pass): `src/routes/settings.index.tsx` (full), `src/routes/_app/platform-connections.tsx` (full), `src/routes/settings.github.tsx` (full), `src/components/ui/searchable-select.tsx` (lines 1-40), `src/features/runner-pools/runner-pools.functions.ts` (lines 1-75), `src/features/operations/operations.functions.ts` (lines 125-235), `src/routes/_app/runner-pools.new.tsx` (lines 480-500), `src/routes/_app/runner-pools.$poolId.settings.tsx` (lines 122-125), `src/routes/_app/webhooks.tsx` (lines 74-90), `src/features/fleet/api-tokens-section.tsx` (lines 170-245), `tests/api-tokens-section.test.ts` (imports and lines 140-182), `docs/handoffs/145/decisions.md`, `docs/handoffs/145/wireframe.html` (grep for the webhooks link), `.gitignore` (line 42, `src/routeTree.gen.ts`), and greps over `src/` and `tests/` for `appError`, `settings/github` and `settings/integrations`. `src/routeTree.gen.ts`: not found in the worktree (generated, gitignored).
- Added in the round-2 brief-review amendment pass (this pass): `docs/handoffs/145-brief.md` (full), `src/routes/_app/platform-connections.tsx` (full, for the line-3 comment), `src/routes/settings.index.tsx` (full), `src/routes/settings.integrations.tsx` (lines 1-30), `src/features/runner-pools/runner-pools.functions.ts` (lines 55-70 and the `export type` list, for `RunnerPoolDetail`), `tests/runner-pool-tart-image.test.ts` (lines 160-200), `src/components/ui/searchable-multi-select.tsx` (lines 200-250), `tests/api-tokens-section.test.ts` (lines 1-20, 147-182, and a grep for `render`/`SecretDialog`/close), `src/features/fleet/api-tokens-section.tsx` (lines 135-140, 167-173 and 180-230: the `SecretDialog` call site and body), a grep over `src/` for `settings/github` and `settings/integrations`, and `git diff 5550cf5 HEAD --stat -- src tests` at tip `436159d` (empty). `src/features/platform-connections/types.ts`: not found (created by this story).
- Added in the round-1 brief-gate amendment pass: `docs/handoffs/145-brief.md` (full), `src/routes/settings.index.tsx` (full, 14 lines), `src/routes/_app/platform-connections.tsx` (full), `src/features/platform-connections/platform-connections.functions.ts` (full, 28 lines), `src/components/pool-plan-preview.tsx` (lines 1-8), `src/features/runner-pools/runner-pools.functions.ts` (lines 1-75 and its `export` list), `src/features/fleet/api-tokens-section.tsx` (lines 1-30 and 130-260: `CreateTokenSection`'s dialog call site, `SecretDialog`, `TokenListSection`), `src/components/ui/searchable-multi-select.tsx` (lines 1-60), `src/routes/settings.integrations.tsx` (lines 1-25), `src/features/operations/operations.functions.ts` (its `export` list, lines 188-235), `src/features/runner-pools/schemas.ts` (grep for `bitbucket`), and greps over `src/` for `settings/github` / `settings/integrations` (including within `api/v1` paths: none), for `github` under `src/features/platform-connections/` (none) and for installation mutation endpoints (none). Not found (created by this story): `src/features/platform-connections/types.ts`, `registry.tsx`, `connections-section.tsx`, `src/components/secret-reveal-dialog.tsx`, `src/features/runner-pools/targets.ts`, `targets-row.tsx`, `platform-options-section.tsx`, `src/routes/settings.connections.tsx`.
- Added in the round-2 brief-gate amendment pass (this pass): `docs/handoffs/145-brief.md` (full), `src/components/pool-plan-preview.tsx` (lines 1-8), `src/features/runner-pools/runner-pools.functions.ts` (lines 50-70, and lines 120-123 via grep), `src/features/platform-connections/platform-connections.functions.ts` (full, 28 lines), `src/routes/settings.integrations.tsx` (lines 8-18), `src/routes/settings.api-tokens.tsx` (lines 1-30), the `export` list of `src/features/operations/operations.functions.ts`, a listing of `src/routes/settings.*.tsx`, greps over `src/` for GitHub/installation export names and `api/v1/...install...` endpoints, and `git diff 5550cf5 HEAD --stat -- src tests` at tip `e6ae29e` (empty). Still not found (created by this story): `src/features/platform-connections/types.ts`, `registry.tsx`, `connections-section.tsx`, `src/features/runner-pools/targets.ts`, `targets-row.tsx`, `platform-options-section.tsx`, `src/routes/settings.connections.tsx`, `docs/handoffs/145/survey.md`.
- Added in the round-1 brief-gate amendment pass after commit `955098d` (this pass; tip `955098d`): `docs/handoffs/145-brief.md` (full), `src/routes/_app/runner-pools.$poolId.settings.tsx` (full, 399 lines), `src/routes/_app/runner-pools.new.tsx` (lines 360-392), `src/features/platform-connections/platform-connections.functions.ts` (full, 28 lines), a listing of `src/features/platform-connections/` (`ls -a`), `src/features/runner-pools/runner-pools.functions.ts` (lines 1-140), `src/routes/settings.github.tsx` (full), `src/routes/settings.integrations.tsx` (full), `src/routes/settings.index.tsx` (full), `src/routes/settings.tsx` (full), `src/routes/settings.api-tokens.tsx` (full), `src/routes/_app/platform-connections.tsx` (full), `docs/handoffs/145/decisions.md` (full), `src/lib/api.ts` (lines 25-45), `vitest.config.ts` (full), `tsconfig.json` (full), `package.json` (scripts and dev dependencies, for the formatter, `vitest` version and `lint`/`typecheck` scripts), a listing of `tests/` and `tests/fixtures/`, `grep -rn appError src tests`, and `find . -name routeTree.gen.ts -not -path "./node_modules/*"` (no match). Not found: `src/features/platform-connections/types.ts`, `connections-section.tsx`, `github-app-section.tsx`, `src/routes/settings.connections.tsx` (all created by this story), and `node_modules/` in this worktree (so TanStack Router's source could not be read for the parent-loader question).

- Added in the round-2 brief-gate amendment pass after commit `07768aa` (this pass; tip `07768aa`): `docs/handoffs/145-brief.md` (full), `package.json` (full, 72 lines), `eslint.config.js` (full, 21 lines), `vitest.config.ts` (full, 15 lines), `tsconfig.json` (full, 22 lines), `src/features/platform-connections/platform-connections.functions.ts` (full, 28 lines), `src/routes/settings.api-tokens.tsx` (full, 36 lines), `src/routes/settings.integrations.tsx` (full, 168 lines), `src/routes/_app/runner-pools.$poolId.settings.tsx` (lines 1-30, plus `wc -l` and `grep -niE "bitbucket|platform-connections"`), `tests/api-tokens-section.test.ts` (lines 1-30, plus greps for `import` and `SecretDialog`), a listing of the repository root (`ls -a`) and of `docs/handoffs/145/`, a `find` for Prettier / Biome / dprint config files (none), `git config core.hooksPath` and the shared git hooks directory (only samples), a grep over `src/` for `settings/github` / `settings/integrations`, and `git diff 5550cf5 HEAD --name-only` (only `docs/handoffs/` files). Not found: `src/routeTree.gen.ts` (generated, gitignored), `.husky/`, `.editorconfig`, any formatter config, `docs/handoffs/145/survey.md` and `docs/handoffs/145-survey.md` (the survey is created by this story at `docs/handoffs/145/survey.md`).

## Architecture decisions (resolve the round-1 Phase-4 BLOCK; these are binding for D, T and F)

### Reuse map (EXTEND / NEW per surface)
| Surface | Decision | Object |
|---|---|---|
| One-time reveal dialog | **EXTEND** (extract) | `SecretDialog` in `src/features/fleet/api-tokens-section.tsx:182-226`. Extract it to `src/components/secret-reveal-dialog.tsx` with generic props (`open`/`value`, title, description, secret label, optional details slot, close-button text, `onClose`, `onClosed`). API tokens and the Connections page both use it. Behaviour is unchanged: **Escape and outside clicks are ignored**, the only way out is the explicit button, the secret leaves the DOM when it leaves state, and it is never kept in a react-query cache. `tests/api-tokens-section.test.ts` ("shows the secret once, ignores Escape, and forgets it on close") must stay green unmodified in intent. The wireframe's C4 note "(or Esc)" is wrong: only the button closes it. (The wireframe `docs/handoffs/145/wireframe.html` is the Phase-3 historical record and is not edited in this story; where it and this brief disagree, this brief wins.) |
| Bitbucket label rule / connection validation | **EXTEND** | `validateBitbucketConnections` in `src/features/runner-pools/schemas.ts:168-183` stays the submit-time authority. Export the label predicate (e.g. `bitbucketLabelError(labels): string \| null`) from `schemas.ts` and have the Bitbucket descriptor's `labelRules.validate` call it. No copy of the regex anywhere else. |
| Grouped multi-select | **EXTEND** | `SearchableMultiSelect` (`src/components/ui/searchable-multi-select.tsx`) gets **optional** props only: option `group`, option `disabled` + `disabledReason`, option `icon`, option `locked`, and a per-group status slot (loading / error+retry / not connected). It does not import the registry and stays generic; existing fleet callers need no changes. Its single `maxSelected` is NOT used for per-kind caps. |
| Settings shell, rows, callouts | **EXTEND** | `SettingsLayout` / `SettingsSection` / `SettingsRow` (`src/components/settings-ui.tsx`), `Callout`, `Badge`, `EmptyState`, `useAction` confirm. |
| Bitbucket connection list/edit/remove | **EXTEND** (move) | `BitbucketSettings` / `ConnectionRow` logic from `src/routes/settings.integrations.tsx` moves into the Connections page, driven by the Bitbucket descriptor's fields and bindings. |
| GitHub App + installations sections | **EXTEND** (move) | The sections of `src/routes/settings.github.tsx` move verbatim except for data access (props instead of `useLoaderData`) to `src/features/platform-connections/github-app-section.tsx`, receiving settings data from the route through props. "Verbatim" covers the JSX, copy, the `CopyRow` / `CreateAppSection` helpers and the action calls; the data access is the one structural change: instead of `settingsRoute.useLoaderData()` inside the component (`settings.github.tsx` lines 24-29), the section receives `configuration`, `githubApp`, `installations` and `user` as props (see "GitHub App section wiring"). |
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
- `requires`/`standalone`: **which kinds are standalone:** `github.repository` and `github.runner-group` declare `standalone: true`; `bitbucket.workspace` declares `standalone: false` with `requires: ["github.repository", "github.runner-group"]` (any one of them satisfies it). Bitbucket workspace is **not standalone** — it requires a GitHub target, because `createRunnerPoolSchema` requires `installationId > 0` and a `scope`. A Bitbucket-only pool stays impossible in this story; the picker shows Bitbucket options as disabled-with-reason until a GitHub target is chosen. **When the last GitHub target is removed while Bitbucket workspaces are selected** (new-pool form), the behaviour is today's, unchanged: the picker does not block the deselection and does not drop the selected workspaces (they stay selected; unselected Bitbucket options become disabled-with-reason again), and the submit button stays enabled as today (`runner-pools.new.tsx` line 494 disables it only on `submitting || options.installations.length === 0`). Submit builds the same payload the old form builds for that state — no repository selected, so `scope: "repository"`, `installationId: selectedRepositories[0]?.installationId ?? 0` = `0`, `repositoryIds: []`, `runnerGroupId: 1`, plus the selected `bitbucketConnectionIds` — and passes it to `createRunnerPoolAction`, whose `parseCreateRunnerPoolInput` (`schemas.ts` lines 160-166) rejects it (`installationId: z.number().int().positive()`, line 127) and throws before `api()` is called, so no POST is sent; the form shows the thrown message in its existing "Could not create the pool" callout (line 489). `schemas.ts` stays the submit-time authority; the picker adds no second validator for this state. Pool settings cannot reach this state: its Bitbucket targets are locked display-only chips and its GitHub kind is fixed by `pool.scope`. (Wireframe D5's "If only one platform has targets, the page renders" must be corrected: with no GitHub installation the form shows the existing "Connect GitHub first" / "Install the GitHub App" empty state, now worded neutrally via the descriptor.)
- `load(context, signal)`: **async and owned by the kind** (the descriptor module), not by `targets-row.tsx`. `context` carries the already-loaded data (`getCreateRunnerPoolOptions()` result, or the pool detail on settings) so installations/workspaces are never fetched twice; kinds whose lists are lazy (GitHub repositories, GitHub runner groups) call today's functions from `runner-pools.functions.ts` inside their own `load`. `targets-row.tsx` keeps **one load state per kind** (loading / error+retry / not connected) and contains no platform-specific code, so #141's GitLab lists need no picker change;
- **`context` (one object, one type).** `TargetContext` (declared in `types.ts`) is a discriminated union built once by the form and passed, as the **same object**, to every kind's `load(context, signal)`, every kind's `available(context)`, `anyStandaloneKindAvailable(context)` and the descriptor payload partials: on the new-pool form `{ form: "create"; options: RunnerPoolOptions }`, where `options` is the route loader's `getCreateRunnerPoolOptions()` result (`runner-pools.new.tsx` line 30 `loader: () => getCreateRunnerPoolOptions()`; `RunnerPoolOptions` in `runner-pools.functions.ts` lines 11-16 and 34-50, whose `BaseOptions` carries `installations: Array<{ id: number; accountLogin: string; accountType: string }>` and `bitbucketConnections: Array<{ id: string; name: string; workspace: string; workspaceUuid: string }>`, lines 12 and 15); on pool settings `{ form: "update"; pool: RunnerPoolDetail }` plus the update-only form values listed below for `toUpdatePayload`. `context` holds no load state: the per-kind loading / error / not-connected state lives only in `targets-row.tsx` and is never read by `available` or the gate;
- `available(context): boolean` — **synchronous**, reads only `context`, and answers "does this kind's platform have at least one connection in the already-loaded data". `github.repository` and `github.runner-group` return `context.options.installations.length > 0` on create; `bitbucket.workspace` returns `context.options.bitbucketConnections.length > 0` (used only for its "not connected" group status, never for the gate, since it is not standalone). It is independent of `load`: a kind can be available while its lazy option list is still loading, failed or empty;
- `manualEntry` (optional, kind-level): `{ label, parse(input): nativeId | null }` — carries today's "Enter runner group ID" fallback for user-account installations on the `github.runner-group` kind;
- `requiresProviders` (optional): providers the kind forces on (Bitbucket workspace → `["tart"]`, preserving the form's auto-add of `tart` at `runner-pools.new.tsx` line 378);
- `planNote` (optional): the sentence the plan preview shows for the kind (feeds `targetPlanCheck`).

Each `PlatformDescriptor` declares `toCreatePayload(selection, context)` and `toUpdatePayload(selection, pool, context)` returning **partials**; one neutral builder in `targets.ts` merges them with the shared rows. The builder calls **every** registered descriptor's partial, whether or not that platform has a selected target, because today's payload always carries `bitbucketConnectionIds` (`[]` when none) and GitHub's empty-selection defaults. Therefore every partial is a **total** function: it never throws, and when its platform has no selected target it returns its platform's empty/default partial (Bitbucket create: `{ bitbucketConnectionIds: [] }`; GitHub: the empty-selection defaults above; a platform with nothing to send: `{}`). Validation stays in `schemas.ts` at submit time, never inside a partial. #141's GitLab partials must follow the same rule. The update `context` carries the pool-settings form values the GitHub update partial needs: `priorityRepositoryIds` and `priorityWaitMinutes` (and `runnerGroupId`). GitHub's partials reproduce today's exact fields, including defaults on an empty or runner-group selection: create sends `installationId`, `scope`, `repositoryIds: []` for organization, and `runnerGroupId` (`runnerGroupId || defaultRunnerGroup?.id || 1` for organization, `1` for repository); update omits `installationId`, `scope` and `bitbucketConnectionIds`, sends `repositoryIds`/`priorityRepositoryIds`/`priorityWaitMinutes` only for repository pools (else `undefined`), and `runnerGroupId: runnerGroupId || pool.runnerGroupId` for organization pools, else `1`. Bitbucket's create partial is `bitbucketConnectionIds`; its update partial is empty (pool settings shows Bitbucket targets as locked, display-only chips — the old settings form never sent them). The chips' labels come from the pool detail itself, with no extra fetch: `RunnerPoolDetail` (`runner-pools.functions.ts` lines 61-62) carries `bitbucketConnections: Array<{ id: string; name: string; workspace: string; workspaceUuid: string }>` alongside `bitbucketConnectionIds: string[]`, verbatim (`runner-pools.functions.ts` lines 61-62, inside `export type RunnerPoolDetail`):

        52  export type RunnerPoolDetail = {
        53    id: string;
        54    installationId: number;
        55    repositoryId: number | null;
        56    repository: string | null;
        57    repositoryIds: number[];
        58    repositories: Array<RepositoryOption & { prioritised?: boolean }>;
        59    priorityRepositoryIds: number[];
        60    priorityWaitMinutes: number;
        61    bitbucketConnections: Array<{ id: string; name: string; workspace: string; workspaceUuid: string }>;
        62    bitbucketConnectionIds: string[];
        63    accountLogin: string;
        64    name: string;
        65    scope: "repository" | "organization";

(re-read at tip `e6ae29e` in the round-2 brief-gate amendment pass: line 52 opens `export type RunnerPoolDetail = {` and lines 61-62 are the two Bitbucket fields, unchanged), so the `bitbucket.workspace` kind's `load` on settings reads `context` (the pool detail) and labels each chip from the matching `bitbucketConnections` entry; `getBitbucketConnections()` is not called on the settings page (today's settings file neither imports nor calls it, verified by reading the whole file: see "Whole-file verification" under "Old pool-settings submit"). **`poolId` on update:** neither the neutral builder nor any descriptor partial produces `poolId` (it is not a target or shared-row field). The pool-settings form adds it itself, exactly as today (`poolId: pool.id`, settings line 124), and passes `{ poolId: pool.id, ...<builder output> }` as `data` to `updateRunnerPoolAction`; so the compared `data` object contains `poolId`, and `updateRunnerPoolAction` strips it before parsing (line 133). The builder's update output therefore has no `poolId` key, and the payload-equality test drives the form (not the builder alone) for the two settings cases. A descriptor flag `supportsPriority` on the GitHub repository kind drives the Priority section in pool settings.

### Import direction
The descriptor types (`PlatformDescriptor`, `TargetKind`, `ConnectionField`, payload partial types) live in `src/features/platform-connections/types.ts`; `registry.tsx` imports the descriptor modules and they import only `types.ts`, so there is no file-level back-edge. Descriptors import only `types.ts`, `runner-pools.functions.ts`, `platform-connections.functions.ts`, `operations.functions.ts`, and from `schemas.ts` types plus the exported Bitbucket label predicate (a value import, explicitly allowed). Shared UI components (`SearchableMultiSelect`, `pool-plan-preview`, `secret-reveal-dialog`) take values as props and never import the registry. No file-level import cycles.

### Registry access in the forms (injection point)
The forms reach the registry the same way `ConnectionsSection` does: through a `descriptors` prop that defaults to the production `platforms` list.
- `targets-row.tsx` (`TargetsRow`), `platform-options-section.tsx` (`PlatformOptionsSection`) and the neutral builder / rule functions in `targets.ts` each take `descriptors: readonly PlatformDescriptor[]` (components: optional prop, default `platforms`; `targets.ts` functions: explicit parameter, with the route passing `platforms`). They never read a module-level registry themselves, so a test can render them with any descriptor list.
- The two route components (`runner-pools.new.tsx`, `runner-pools.$poolId.settings.tsx`) import `platforms` from `registry.tsx` and pass that one list to `TargetsRow`, `PlatformOptionsSection` and the `targets.ts` builder/gate, so the Targets row, Platform options and the submitted payload always use the same descriptor list. The routes take no injection prop (TanStack route components receive none).
- **Which tests use which list.** The payload-equality cases and the create-button-gate cases drive the rendered route forms, so they run against the production `platforms` (GitHub + Bitbucket) — the same descriptor code path production uses; no test-only descriptor is involved. The mixing-rules cases (a)-(e) also need only the production descriptors (Bitbucket's `standalone: false` is production behaviour) and drive the rendered new-pool form. A test that needs a test-only descriptor in the picker or the Platform options section renders `TargetsRow` / `PlatformOptionsSection` directly with an injected `descriptors` list; the post-create test renders `ConnectionsSection` with an injected list, as stated under "Connections page loading". `vi.mock` of `registry.tsx` is not needed for any acceptance case.

### Connections page loading
**(Round-2 Phase-4 decision; supersedes the earlier route-loader wording.)** `src/routes/settings.connections.tsx` has **no route loader** and stays thin, like `settings.api-tokens.tsx`: it reads `getRouteApi("/settings").useLoaderData()` and composes `SettingsLayout`, `ConnectionsSection` and `GitHubAppSection`.
- Loading lives in `src/features/platform-connections/connections-section.tsx`. It takes `descriptors` (default: the production `platforms`) and the `/settings` page data as props. Tests render `ConnectionsSection` directly with an injected test-only descriptor and fixture page data; no router loaderData shape is involved.
- Each platform that has a list binding gets **its own load state**, using the existing component-level pattern: `useQuery` keyed per platform plus `InlineError onRetry={refetch}` (as in `src/features/fleet/api-tokens-section.tsx:229-241`). Precisely what that precedent shows and what is new: `TokenListSection` has **one** `useQuery` for **one** list (line 229, `useQuery({ queryKey: apiTokensQueryKey, queryFn: ({ signal }) => listApiTokens(signal) })`) rendering `LoadingRows` while pending and `InlineError ... onRetry={() => void tokens.refetch()}` on error (lines 238-241). No existing component keys queries per platform; the per-platform part is new in this story and is specified here, not copied: `connections-section.tsx` renders one child component per descriptor that has a fetching list binding, and each child calls that same single-`useQuery` shape once, with query key `["platform-connections", descriptor.id]` and `queryFn: ({ signal }) => <that descriptor's list binding>(signal)`, rendering its own `LoadingRows` / `InlineError onRetry={() => void query.refetch()}`. Because each platform's query lives in its own child, `refetch` on one never re-runs another. Retry re-runs only that platform's binding, and only that platform shows a loading row; one platform's failure never hides the others. `Promise.allSettled` is not used.
- GitHub rows are derived from the `/settings` page data props (`page.data.installations`) — no fetch, no Retry for GitHub.
- **What "list binding" means (two forms, declared by the descriptor, never inferred from its id).** A descriptor's list binding is either *fetching* — an async function of an `AbortSignal` that can reject (Bitbucket: `getBitbucketConnections()`) — or *page-derived* — a synchronous function of the `/settings` page data that cannot reject (GitHub: rows from `page.data.installations`). Which form a descriptor uses is part of its declared shape in `types.ts` (the exact property names are D/F's choice; the survey table names them), so `connections-section.tsx` chooses between "render one `useQuery` child" and "render rows directly" by that declared form, not by platform id — this is not platform-specific code. Only fetching bindings get a load state, a loading row, an `InlineError` and Retry; a page-derived binding has none of these. In the partial-load acceptance criterion, "list binding rejects" therefore always means a fetching binding.
- After a create/update/remove, reload only that platform via `useAction({ after })` (invalidate that platform's query key). `router.invalidate()` is still allowed to refresh `/settings` data where GitHub data changes (e.g. after "Refresh access").
- The create call and any post-create value (the one-time secret) are kept out of react-query: plain async call + component state, handed to the extracted secret-reveal dialog.
- **Post-create hook contract (types in `types.ts`).** The descriptor field is optional, `postCreate?: (created: TCreated) => PostCreateReveal | null | Promise<PostCreateReveal | null>`, where `TCreated` is whatever that descriptor's create binding resolves to (for Bitbucket, the `createBitbucketConnection` return type quoted below), and
  `PostCreateReveal = { title: string; description: string; secretLabel: string; value: string; details?: ReactNode; closeButtonText?: string }`.
  The fields map one-to-one onto the extracted `SecretDialog` props listed in the Reuse map (`value` is the secret string shown and copied; `details` fills the optional details slot; `closeButtonText` defaults to the dialog's own button text when absent). Flow in `connections-section.tsx`: `const created = await descriptor.create(values)`; then `const reveal = descriptor.postCreate ? await descriptor.postCreate(created) : null`; `reveal` goes into component state (`useState<PostCreateReveal | null>`), the dialog renders with `open={reveal !== null}` and `value={reveal?.value ?? ""}` etc. (see "Extracted dialog prop interface" below), and its `onClose` sets the state back to `null`, so the value leaves the DOM and is never shown again. A hook that returns `null` shows no dialog. The value is a `string` only (no other value types). The test-only descriptor in the acceptance criterion declares a create binding resolving to a fixture object and a `postCreate` returning a `PostCreateReveal` with a fixed `value`; the test asserts that `value` is in the dialog after create, that Escape does not close it, and that it is absent from the DOM after the close button is clicked.
- **Extracted dialog prop interface (`src/components/secret-reveal-dialog.tsx`).** Exactly the Reuse-map list, typed:
  `SecretRevealDialogProps = { open: boolean; value: string; title: string; description: ReactNode; secretLabel: string; copyLabel?: string; details?: ReactNode; closeButtonText?: string; onClose: () => void; onClosed?: () => void }`.
  `secretLabel` is the `<code aria-label>`; `copyLabel` is the `CopyButton` `label` and defaults to `secretLabel` when absent (the Copy button's text stays `Copy`); `closeButtonText` defaults to the string `I’ve stored it, close` (today's hard-coded text at `api-tokens-section.tsx` line 219, now the default value of the prop rather than a literal in the JSX); `onClosed` is called from `onCloseAutoFocus` after `event.preventDefault()`, as lines 193-196 do today. The dialog renders its body only while `open` is true (as `{revealed ? … : null}` does today, line 200), keeps `instant`, and keeps `onEscapeKeyDown` / `onInteractOutside` calling `event.preventDefault()` (lines 197-198). `PostCreateReveal`'s fields are a subset of these props, so `connections-section.tsx` passes `open={reveal !== null}`, `value={reveal?.value ?? ""}`, `title`/`description`/`secretLabel`/`details`/`closeButtonText` from `reveal`, and no `copyLabel` (it defaults to `secretLabel`).
  **`CreateTokenSection` call site** (replacing lines 167-173): it passes `open={flow.revealed !== null}`, `value={flow.revealed?.secret ?? ""}`, `title="Copy your new token"`, the description text quoted under "Tests known to break" (interpolating `flow.revealed.token.name`), `secretLabel="API token secret"`, `copyLabel="API token"`, `details` = today's Scopes/Expires `<dl>` (lines 212-217), the existing `onClose` and `onClosed={() => nameRef.current?.focus()}`, and **omits** `closeButtonText`, relying on the default `I’ve stored it, close`. So the rendered dialog is unchanged and `tests/api-tokens-section.test.ts` needs no edit.
- `canManage` comes from each platform's loaded data (`getBitbucketConnections().canManage`; GitHub from `user.role`).
- The wireframe's B5b note that Retry "re-runs only that load" is consistent with this; its note B5 (a descriptor `settingsSection` slot for the GitHub App) is **superseded**: the GitHub App section is composed by the route, not hung off the descriptor.

**GitHub App section wiring (how its data arrives and how its actions are invoked).** Verified from `src/routes/settings.github.tsx` (the verbatim lines 17, 20-30, 47 and 136-140 are quoted under "Verified source excerpts → `settings.github.tsx` route and data source" below): the page has **no route loader and no route-level action functions**. It reads the parent `/settings` layout's loader data (`const settingsRoute = getRouteApi("/settings")`, line 24; `const page = settingsRoute.useLoaderData()`, line 27; the parent loader is `loader: () => getSettingsPage()` in `src/routes/settings.tsx` line 10), and its two actions are plain async calls made from components: `syncGitHubAction()` from the "Refresh access" `AsyncActionButton` (line 47: `action={() => syncGitHubAction()}`), and `createGitHubAppManifestAction({ data: { ownerType, organization, name } })` from `CreateAppSection`'s button (lines 133-155, which then builds and submits the manifest form to GitHub). Binding wiring after the move:
- `src/routes/settings.connections.tsx` (a child of `/settings`: TanStack file-based routing makes `settings.<name>.tsx` a child of `settings.tsx`, as with the existing `settings.api-tokens.tsx`, which declares `createFileRoute("/settings/api-tokens")` (line 8) and reads `const settingsRoute = getRouteApi("/settings")` / `settingsRoute.useLoaderData()` (lines 12 and 15); the new file declares `createFileRoute("/settings/connections")`) reads `getRouteApi("/settings").useLoaderData()` exactly as `settings.github.tsx` does today and passes `page.data` (`configuration`, `githubApp`, `installations`, `user`) to `GitHubAppSection` as props. The section never calls `getRouteApi` itself. When `page.data` is null it keeps today's "GitHub unavailable" empty state (line 28) for the GitHub part only.
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

**Old pool-settings submit** (`src/routes/_app/runner-pools.$poolId.settings.tsx` lines 122-144). This settles NV-4. Besides `poolId` (stripped by `updateRunnerPoolAction`, `runner-pools.functions.ts` line 133 `const { poolId, ...values } = data;`), the `data` object carries no `installationId`, no `scope` and no `bitbucketConnectionIds`. The form has no Bitbucket workspaces control: its only use of `bitbucketConnectionIds` is line 271 (`<TartBaseVmRow bitbucket={pool.bitbucketConnectionIds.length > 0} … />`). **Whole-file verification** (the file was read in full, all 399 lines, at tip `955098d` in the round-1 brief-gate amendment pass after commit `955098d`): its imports (lines 1-27) contain nothing from `~/features/platform-connections/` — in particular no `getBitbucketConnections` — and its `runner-pools.functions` import is exactly `type RepositoryOption`, `type RunnerGroupOption`, `type RunnerPoolDetail`, `getInstallationRunnerGroups`, `getRunnerPoolRepositories` and `updateRunnerPoolAction` (lines 20-27); `grep -ni bitbucket` on the file matches only line 271, so neither `pool.bitbucketConnections` nor any Bitbucket state, control or fetch appears anywhere else; its `useState` hooks (lines 47-73) hold no Bitbucket ids; and its "Destination" section (lines 160-213) contains only the "GitHub accounts"/"Installation" row (161-163) and the Repositories (164-186) or Runner group (187-211) row. So today's settings page never fetches Bitbucket connections and never edits or submits Bitbucket ids.

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

**Old Bitbucket workspaces row and the `tart` auto-add** (`src/routes/_app/runner-pools.new.tsx` lines 365-379, verbatim, read at tip `955098d`; lines 380-386 — the `options` mapping, placeholders, `selectedNoun`, `values` and the closing tags — are elided). Line 378 is the auto-add of `tart` when a workspace is picked; line 368 is the `/settings/integrations` link that the redirect criterion repoints:

        365            <SettingsRow
        366              description={options.bitbucketConnections.length
        367                ? "Optional. Each workspace shares this pool’s capacity with a managed macOS runner for Bitbucket Pipelines."
        368                : <>Optional. Connect a workspace first in <Link className="text-foreground underline-offset-2 hover:underline" to="/settings/integrations">Settings → Integrations</Link>.</>}
        369              label="Bitbucket workspaces"
        370              stacked
        371            >
        372              <SearchableMultiSelect
        373                ariaLabel="Bitbucket Cloud workspaces"
        374                emptyMessage="No Bitbucket connections are configured"
        375                maxSelected={maxCount}
        376                onValueChange={(nextConnections) => {
        377                  setBitbucketConnectionIds(nextConnections);
        378                  if (nextConnections.length && !providers.includes("tart")) setProviders((current) => [...current, "tart"]);
        379                }}

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

**The rest of `platform-connections.functions.ts`, and its directory** (read in full, 28 lines, at tip `955098d`). Lines 1-15 and 23-28 verbatim (lines 17-21 are quoted just above; line 16 and line 22 are blank):

         1  import { api } from "~/lib/api";
         2
         3  export type BitbucketConnection = {
         4    id: string;
         5    provider: "bitbucket";
         6    name: string;
         7    workspace: string;
         8    workspaceUuid: string;
         9    createdAt: string;
        10    updatedAt: string;
        11    canManage: boolean;
        12  };
        13
        14  export const getBitbucketConnections = () =>
        15    api<{ canManage: boolean; items: BitbucketConnection[] }>("/api/v1/platform-connections/bitbucket");
        ...
        23  /** Renames a connection and, when `accessToken` is set, replaces its token. */
        24  export const updateBitbucketConnection = (connectionId: string, data: { name: string; accessToken?: string }) =>
        25    api<BitbucketConnection>(`/api/v1/platform-connections/bitbucket/${encodeURIComponent(connectionId)}`, { method: "PUT", body: data });
        26
        27  export const removeBitbucketConnection = (connectionId: string) =>
        28    api<void>(`/api/v1/platform-connections/bitbucket/${encodeURIComponent(connectionId)}`, { method: "DELETE" });

Together the block above (lines 17-21) and this block (lines 1-15 and 23-28, with blank lines 16 and 22) are the **whole 28-line file**; re-read in full at tip `07768aa` (round-2 brief-gate NV-2), unchanged. So the file's only exports are the five named under "Verified facts" (no GitHub type or function). Fact for D/F (no decision taken here): `getBitbucketConnections` takes no parameter today (line 14), so it does not accept the `queryFn`'s `signal` as written. `ls -a src/features/platform-connections/` at the same tip lists only `.`, `..` and `platform-connections.functions.ts`: there is no barrel/`index` file, stale component or placeholder in that directory, so the new `types.ts`, `registry.tsx`, `github.tsx`, `bitbucket.tsx`, `connections-section.tsx` and `github-app-section.tsx` are the only other files it will hold, and the Import-direction rule has no existing file to account for.

**GitHub App manifest return type** (`src/features/operations/operations.functions.ts` lines 230-235; round-1 brief-review NV-2). This is App setup, not a connection create, and its response carries no secret either: it returns the GitHub form `action` URL, a `state` token for the redirect and the `manifest` JSON, which `CreateAppSection` posts to GitHub (`settings.github.tsx` lines 141-150). GitHub returns the App's credentials directly to the server after that redirect; the UI never sees them. So "No existing create response carries a secret" holds, and the post-create hook has no production consumer in this story.

    export const createGitHubAppManifestAction = ({ data }: { data: {
      ownerType: "user" | "organization"; organization?: string; name?: string;
    } }) => api<{ action: string; state: string; manifest: string; webhookActive: boolean }>(
      "/api/v1/github-app/manifest",
      { method: "POST", body: data },
    );

**`SearchableSelectOption` type** (`src/components/ui/searchable-select.tsx` lines 13-20; round-1 brief-review NV-3). It has none of the fields the Reuse map adds (`group`, `disabled`, `disabledReason`, `icon`, `locked`), so the extension adds them all as new optional fields:

    13  export type SearchableSelectValue = string | number;
    14
    15  export type SearchableSelectOption<TValue extends SearchableSelectValue> = {
    16    value: TValue;
    17    label: string;
    18    description?: string;
    19    keywords?: string[];
    20  };

**Repository and runner-group option ids** (`src/features/runner-pools/runner-pools.functions.ts` lines 18-32; round-1 brief-review NV-8). Both id spaces are `number`:

    18  export type RunnerGroupOption = {
    19    id: number;
    20    name: string;
    21    visibility: string;
    22    isDefault: boolean;
    23  };
    24
    25  export type RepositoryOption = {
    26    id: number;
    27    installationId: number;
    28    accountLogin: string;
    29    accountType: string;
    30    fullName: string;
    31    private: boolean;
    32  };

**Create button gate** (`src/routes/_app/runner-pools.new.tsx` line 494; round-1 brief-review NV-4):

   494            <Button disabled={submitting || options.installations.length === 0} type="submit">

**`SearchableMultiSelect` props** (`src/components/ui/searchable-multi-select.tsx` lines 14-28; settles NV-5). Options are `SearchableSelectOption<TValue>` = `{ value: TValue; label: string; description?: string; keywords?: string[] }` (`searchable-select.tsx` lines 15-20), with no group, disabled or icon field. The listbox (lines 209-245) renders one flat list with no group headings. Verbatim core of that listbox (line 225, the option `key`, is omitted; lines 218-222 (class names) and 229-242 (the remaining button attributes and the checkbox and label spans) are elided):

    209            <div aria-label={ariaLabel} className="max-h-72 overflow-y-auto p-1" id={listboxId} role="listbox" aria-multiselectable="true">
    210              {filteredOptions.length === 0 ? (
    211                <div className="px-3 py-6 text-center text-xs text-muted-foreground">{emptyMessage}</div>
    212              ) : filteredOptions.map((option, index) => {
    213                const selected = values.includes(option.value);
    214                const atLimit = !selected && maxSelected !== undefined && values.length >= maxSelected;
    215                return (
    216                  <button
    217                    aria-selected={selected}
    ...
    223                    disabled={atLimit}
    224                    id={`${listboxId}-option-${index}`}
    226                    onClick={() => toggle(option.value)}
    227                    onMouseEnter={() => setActiveIndex(index)}
    228                    role="option"
    ...
    243                );
    244              })}
    245            </div>

So each option is one `role="option"` button mapped straight from `filteredOptions`, with `disabled` driven only by `atLimit` and the active index counted across the whole flat list. The grouped extension must keep this rendering (and keyboard index) unchanged for options without `group`, so existing fleet callers are unaffected; group headings appear only when options carry `group`.

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

The file imports nothing from the registry or from `runner-pools` (its imports are `lucide-react`, `~/components/ui/badge` and `~/lib/utils`, lines 1-4). Verbatim, lines 1-4 (re-read at tip `e6ae29e` in the round-2 brief-gate amendment pass; line 6 is the local `type Provider`, and there are no other imports):

     1  import { CircleCheck, CircleX } from "lucide-react";
     2
     3  import { Badge } from "~/components/ui/badge";
     4  import { cn } from "~/lib/utils";

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

**References to the redirecting routes** (W-2). `grep -rn -e "settings/github" -e "settings/integrations" -e "/platform-connections" src` (unquoted, so it also catches template strings and route constants), excluding `routeTree.gen.ts` (and, for the `/platform-connections` review-aid pattern only, `/api/v1/` paths; no `/api/v1/` path matches `settings/github` or `settings/integrations`, verified by `grep -rn "api/v1" src | grep -e "settings/github" -e "settings/integrations"` printing nothing), finds only: `settings.index.tsx:10`, `settings.tsx:41-42`, `_app/webhooks.tsx:83`, `_app/runner-pools.new.tsx:368`, the two route declarations (`settings.github.tsx:20`, `settings.integrations.tsx:21`), `_app/platform-connections.tsx:4,6`, and the module import path `~/features/platform-connections/platform-connections.functions` (`settings.integrations.tsx:17`, not a link). Verbatim, `settings.integrations.tsx` lines 11-17 are one complete `import { … } from` statement (it opens at line 11 with `import {`; line 10 is the separate, complete `import { Input } from "~/components/ui/input";`), so line 17 is a module specifier, not a `Link`/`redirect` target:

        11  import {
        12    type BitbucketConnection,
        13    createBitbucketConnection,
        14    getBitbucketConnections,
        15    removeBitbucketConnection,
        16    updateBitbucketConnection,
        17  } from "~/features/platform-connections/platform-connections.functions";

  It matches only the `/platform-connections` review-aid pattern, never the pass/fail `settings/github` / `settings/integrations` patterns. No link to them is built dynamically today. The acceptance grep is run in this unquoted form. After the change, the `settings/github` and `settings/integrations` patterns may match only `settings.github.tsx:20` and `settings.integrations.tsx:21` (the route declarations); there is no `/api/v1/` allowance for these two patterns (none exists today, and a new one would fail the criterion); any hit in `_app/platform-connections.tsx` fails. The `/platform-connections` pattern is a review aid, not part of the pass/fail string check: it will legitimately match the `_app/platform-connections.tsx` route declaration, `/api/v1/platform-connections/...` paths and every `~/features/platform-connections/...` import (including the new registry modules); its pass condition is only that no `Link`/`redirect` `to` targets `"/platform-connections"`.

**Formatter and lint tooling** (round-2 brief-gate B-1 / NV-1 / NV-5 / NV-10; read at tip `07768aa`, where `git diff 5550cf5 HEAD --name-only` lists only files under `docs/handoffs/`, so these files are as at `5550cf5`). `package.json` lines 19-21 (scripts) and 54-71 (the whole `devDependencies`), verbatim; `dependencies` (lines 27-53) contains no formatter either, and the file has no `prettier`, `husky` or `lint-staged` key:

    19      "typecheck": "tsc --noEmit",
    20      "lint": "eslint src tests *.config.ts eslint.config.js --max-warnings=0",
    21      "test": "vitest run",
    ...
    54    "devDependencies": {
    55      "@eslint/js": "^9.39.5",
    56      "@redocly/cli": "2.72.1",
    57      "@tailwindcss/vite": "^4.2.2",
    58      "@tanstack/router-plugin": "^1.168.23",
    59      "@types/node": "^22.19.19",
    60      "@types/react": "^19.2.14",
    61      "@types/react-dom": "^19.2.3",
    62      "@vitejs/plugin-react": "^6.0.2",
    63      "eslint": "^9.39.4",
    64      "eslint-plugin-react-hooks": "^7.0.1",
    65      "jsdom": "^27.2.0",
    66      "tailwindcss": "^4.2.2",
    67      "typescript": "^6.0.2",
    68      "typescript-eslint": "^8.65.0",
    69      "vite": "^8.0.14",
    70      "vitest": "^4.0.18"
    71    }

`eslint.config.js`, the whole file (21 lines), verbatim:

     1  import eslint from "@eslint/js";
     2  import reactHooks from "eslint-plugin-react-hooks";
     3  import tseslint from "typescript-eslint";
     4
     5  export default tseslint.config(
     6    {
     7      ignores: [".output/**", "node_modules/**", "src/routeTree.gen.ts", "data/**"],
     8    },
     9    eslint.configs.recommended,
    10    ...tseslint.configs.recommended,
    11    {
    12      files: ["src/**/*.{ts,tsx}", "tests/**/*.ts", "*.config.{ts,js}"],
    13      plugins: { "react-hooks": reactHooks },
    14      rules: {
    15        ...reactHooks.configs.flat.recommended.rules,
    16        "@typescript-eslint/no-explicit-any": "error",
    17        "@typescript-eslint/no-unused-vars": ["error", { argsIgnorePattern: "^_", varsIgnorePattern: "^_" }],
    18        "no-console": ["error", { allow: ["warn", "error", "info"] }],
    19      },
    20    },
    21  );

No formatter configuration exists: `find . -maxdepth 3 \( -name ".prettierrc*" -o -name "prettier.config.*" -o -name "biome.json*" -o -name "dprint.json" -o -name ".dprint.json" \) -not -path "./node_modules/*"` prints nothing, the repository root has no `.editorconfig` or `.husky/`, `git config core.hooksPath` is unset and the shared hooks directory holds only `*.sample` files, and `grep -niE "prettier|biome|dprint|stylistic|format" package.json eslint.config.js` prints nothing. The lint script has no `--fix`, so even a formatting rule could only report, never rewrite; and the config loads no stylistic plugin.

**Test collection and typecheck scope** (NV-8 / NV-9; read at tip `07768aa`). `vitest.config.ts`, the whole file, verbatim — line 11 is the only collection pattern, and there is no `typecheck` option:

     1  import { fileURLToPath } from "node:url";
     2
     3  import { defineConfig } from "vitest/config";
     4
     5  export default defineConfig({
     6    resolve: {
     7      alias: { "~": fileURLToPath(new URL("./src", import.meta.url)) },
     8    },
     9    test: {
    10      environment: "node",
    11      include: ["tests/**/*.test.ts"],
    12      restoreMocks: true,
    13      unstubEnvs: true,
    14    },
    15  });

`tsconfig.json` line 2, verbatim (so `tsc --noEmit` typechecks every `tests/**/*.ts`):

     2    "include": ["src/**/*.ts", "src/**/*.tsx", "tests/**/*.ts", "vite.config.ts", "vitest.config.ts"],

**`settings.api-tokens.tsx` route and data source** (NV-7; read in full, 36 lines, at tip `07768aa`), lines 8-10 and 12-15 verbatim:

     8  export const Route = createFileRoute("/settings/api-tokens")({
     9    component: ApiTokensSettings,
    10  });
    ...
    12  const settingsRoute = getRouteApi("/settings");
    13
    14  function ApiTokensSettings() {
    15    const page = settingsRoute.useLoaderData();

**`ConnectionRow` in `settings.integrations.tsx`** (NV-4; read in full, 168 lines, at tip `07768aa`). Lines 84-85 (signature) and 130-145 (the `canManage`-gated Edit / Remove controls) verbatim; lines 147-165 render the inline edit form (`Connection name`, `New API token`, Cancel / Save) only while `editing` is true, and `BitbucketSettings` passes `canManage` from its loader data (`const { canManage, items: connections } = Route.useLoaderData();`, line 27):

     84  /** One connected workspace, with inline editing and removal for administrators. */
     85  function ConnectionRow({ connection, canManage }: { connection: BitbucketConnection; canManage: boolean }) {
    ...
    130          {canManage && !editing ? (
    131            <div className="flex shrink-0 items-center gap-1">
    132              <Button onClick={startEditing} size="sm" variant="ghost">Edit</Button>
    133              <Button
    134                onClick={() => void run({
    135                  action: () => removeBitbucketConnection(connection.id),
    136                  confirm: `Remove ${connection.name}? GridOps deletes its stored token. Pools that use it must be changed first.`,
    137                  success: `${connection.name} removed.`,
    138                })}
    139                size="sm"
    140                variant="ghost"
    141              >
    142                Remove
    143              </Button>
    144            </div>
    145          ) : null}

**Pool-settings imports and Bitbucket uses** (NV-3; `src/routes/_app/runner-pools.$poolId.settings.tsx`, 399 lines, re-read at tip `07768aa`). Lines 20-27 verbatim are its only `runner-pools.functions` import; no line imports from `~/features/platform-connections/`, and `grep -niE "bitbucket|platform-connections"` on the file prints exactly one line, 271:

     20  import {
     21    type RepositoryOption,
     22    type RunnerGroupOption,
     23    type RunnerPoolDetail,
     24    getInstallationRunnerGroups,
     25    getRunnerPoolRepositories,
     26    updateRunnerPoolAction,
     27  } from "~/features/runner-pools/runner-pools.functions";
    ...
    271              <TartBaseVmRow bitbucket={pool.bitbucketConnectionIds.length > 0} onChange={setTartImage} value={tartImage} />

So the settings form neither imports nor calls `getBitbucketConnections()` and does not read `pool.bitbucketConnections` today; the new settings form's Bitbucket chips label from `pool.bitbucketConnections` (the `RunnerPoolDetail` field quoted under the Target-kind contract), not from any fetch.

**`tests/api-tokens-section.test.ts` import block** (NV-6; lines 1-7 verbatim, re-read at tip `07768aa`; `grep -n "SecretDialog" tests/api-tokens-section.test.ts` prints nothing):

     1  // @vitest-environment jsdom
     2  /** Drives the API token sections in a DOM: retries, the one-time secret and revoking. */
     3  import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
     4  import { createElement, act, type ReactElement } from "react";
     5  import { createRoot, type Root } from "react-dom/client";
     6  import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
     7  import { CreateTokenSection, TokenListSection } from "../src/features/fleet/api-tokens-section";

### Payload-equality reference (frozen old-form output)
This section defines what the payload-equality acceptance criterion compares against, so the comparison does not depend on the old rows still existing.
- **Where they live:** as inline `const` objects in the Phase-5 test file that holds the payload-equality cases (for example `tests/pool-target-payloads.test.ts`), one per case: GitHub repository create, GitHub organization runner-group create, GitHub repositories plus Bitbucket workspaces create, repository-pool settings save, and organization-pool settings save. Each case also holds its fixed inputs (the `getCreateRunnerPoolOptions()` or `RunnerPoolDetail` fixture, the selection, and the shared-row field values), so the new form can be driven with identical inputs. Shared fixtures may go in `tests/fixtures/`.
- **How they are derived (from pre-change behaviour only):** T captures them in the RED phase, before any route or component is changed. T renders the **old** forms at the pre-implementation state of this branch (today `5550cf5`) with the harness that `tests/runner-pool-tart-image.test.ts` already uses (lines 16-28 and 90-104: a mocked `@tanstack/react-router` serving `loaderData`, and a stubbed `fetch`). T drives the old controls ("Runner pool scope", "Repositories", "GitHub installation", "GitHub runner group", "Bitbucket Cloud workspaces", "Pool repositories"), submits, and records the `data` argument passed to `createRunnerPoolAction` / `updateRunnerPoolAction`. Recording works by `vi.mock` of `~/features/runner-pools/runner-pools.functions` that wraps the real functions and stores `data`. T pastes each recorded object into the test as the constant, with a comment naming the commit and the source lines quoted above. T cross-checks each constant by hand against the "Old new-pool submit" and "Old pool-settings submit" excerpts.
- **Rules:** the constants must never be generated by, imported from or recomputed with the new implementation (`targets.ts`, descriptors, the new forms). F must not edit them. A mismatch is fixed in the implementation, not in the constant. Comparison is Vitest `toEqual` on the `data` object (keys set to `undefined` count as absent, as Required design 3 states). `toEqual` is intentional: `toStrictEqual` must not be substituted, because it treats `{ repositoryIds: undefined }` and `{}` as different, while `JSON.stringify` (what `api()` sends) does not. The test file says so in a comment next to the assertions.
- **Sequencing gate (no circular dependency).** The capture needs the old forms, so it happens before any `src/` change on this branch, and it does not depend on the old forms surviving afterwards:
  1. Starting state, verified: at the branch tip `ff6583a`, `git diff 5550cf5 HEAD --stat -- src tests` is empty — the only commits after `5550cf5` touch `docs/handoffs/` — so the worktree's `src/` is still the pre-change code the excerpts below quote. Re-verified at branch tip `436159d` (same empty result), and again at branch tip `e6ae29e` in the round-2 brief-gate amendment pass (same empty result). The reproducible reference is the baseline commit **`5550cf5`**, not any branch tip (tips move as handoff docs are committed): T and F both read it here, T re-runs `git diff 5550cf5 HEAD --stat -- src tests` at the start of Phase 5 and records the then-current tip hash and the (empty) result in T's handoff note under `docs/handoffs/145/`.
  2. T runs the capture first, against that unchanged `src/`, with a one-off capture harness (the driving-and-recording code described above). The harness is not committed as a collected test, because it drives the old controls and would fail once F removes them; only its output, the pasted constants with their commit/line comments, is committed in the payload-equality test file. Where the harness lives: it is a **temporary, never-committed** file — either an uncommitted `tests/<name>.test.ts` in this worktree that T runs with `npx vitest run tests/<name>.test.ts` and deletes before committing, or the same file inside the step-4 scratch worktree. No committed path under `tests/` may hold it: Vitest collects every `tests/**/*.test.ts` (`vitest.config.ts` line 11, `include: ["tests/**/*.test.ts"]`) and `tsc` typechecks every `tests/**/*.ts` (`tsconfig.json` `include`), so a committed harness would be collected (and fail once F removes the old controls). Its source text is preserved only as an indented block in T's handoff note under `docs/handoffs/145/` (see the payload-equality criterion's invariant (iv)), never as a file under `docs/` or `tests/`.
  3. T's Phase-5 commit(s) change nothing under `src/` (T writes tests only); `git diff 5550cf5 <T's last commit> -- src` must be empty. F makes no `src/` change until T's commits are on the branch (the Operating rules already require T to commit before F starts).
  4. If `src/` has already changed when T captures (for example a re-run of Phase 5 after F's work), T does not capture from the worktree: T checks out the pre-change tree separately, `git worktree add <scratch dir> 5550cf5` (detached, read-only use), runs the same capture harness there, and removes that worktree afterwards. The constants are always taken from the `5550cf5` source, whichever way it is reached.
  5. The excerpts in "Verified source excerpts" are the cross-check, not a substitute for the capture: they quote only the target fields and the shared-row expressions (`poolImages(...)`, label splitting, `Number(form.get(...))`), and the concrete values those produce depend on code and defaults the excerpts do not show. A constant that disagrees with the excerpts is a capture error and is re-captured, never hand-edited to match.

### Remaining GitHub couplings in shared rows (round-2 W5)
- Maximum count `min={Math.max(1, repositoryIds.length)}` (new form ~line 458, pool settings ~line 283) → derive from `targets.ts` (e.g. `minMaxCount(selection)` = max(1, number of selected targets of kinds whose `limit` is bound to `maxCount`)), preserving today's value for every GitHub/Bitbucket selection (today Bitbucket workspaces do not raise the minimum; keep that).
- Create button `disabled={submitting || options.installations.length === 0}` (~line 494) → derive from the standalone-target gate in `targets.ts` (disabled when no standalone kind has any loadable target), equal to today's value for the current platforms. Precisely: the gate is (for example) `submitting || !anyStandaloneKindAvailable(context)`, where `anyStandaloneKindAvailable(context)` is `true` when some registered kind with `standalone: true` returns `true` from its `available(context)` (Target-kind contract), and `context` is the same `TargetContext` object the Targets row passes to each kind's `load` — so a standalone kind is "available" when its platform has at least one connection in the already-loaded `context` — for both GitHub kinds (the only standalone kinds, per the Target-kind contract) that is `options.installations.length > 0`. Availability is **not** whether a kind's lazy option list (repositories, runner groups) has loaded or is non-empty, and Bitbucket connections never count (Bitbucket is not standalone). So for every GitHub/Bitbucket selection and options state the derived value equals today's `submitting || options.installations.length === 0`: with ≥1 installation it is enabled (regardless of selection, including the "last GitHub target removed" state above); with 0 installations it is disabled even if Bitbucket connections exist. Vitest covers both installation counts, with and without Bitbucket connections. Verbatim (`runner-pools.new.tsx` line 494): `<Button disabled={submitting || options.installations.length === 0} type="submit">`.

### Other sites now in scope
- `src/routes/_app/webhooks.tsx:83` links to `/settings/github` → repoint to `/settings/connections` (no redirect hop).
- **`?appError=` handling in `settings.index.tsx`** (verified, lines 4-13). Today the error is surfaced by a toast in `beforeLoad` and the redirect passes no `search`, so the query parameter is **not** carried to the target page, and nothing else in `src/` or `tests/` reads `appError` (grep finds only `settings.index.tsx` lines 7-9). The change keeps this behaviour exactly: the `toast.error(search.appError)` call (guarded by `!preload`) stays in `settings.index.tsx`, and only the redirect target changes from `"/settings/github"` to `"/settings/connections"`, still with no `search`. The Connections page does not read or consume `appError`. The line-7 comment is updated to name the Connections page. The fallback redirect (line 12) is not changed. Current code, the whole file verbatim (lines 1-14):

        1  import { createFileRoute, redirect } from "@tanstack/react-router";
        2  import { toast } from "sonner";
        3
        4  export const Route = createFileRoute("/settings/")({
        5    beforeLoad: ({ location, preload }) => {
        6      const search = location.search as Record<string, unknown>;
        7      // GitHub App setup failures land on /settings?appError=…; surface them on the GitHub page.
        8      if (typeof search.appError === "string") {
        9        if (!preload) toast.error(search.appError);
       10        throw redirect({ replace: true, to: "/settings/github" });
       11      }
       12      throw redirect({ replace: true, to: "/settings/general" });
       13    },
       14  });
- `ReadOnlyPool` in `runner-pools.$poolId.settings.tsx` branches on `pool.scope` → render its targets through the same registry-driven target display (read-only).
- Update `src/components/pool-plan-preview.tsx` line 50 as per the Reuse map.

### Tests known to break (T must update them in the RED phase, not F)
- `tests/runner-pool-tart-image.test.ts` uses the "Repositories" and "Bitbucket Cloud workspaces" controls, and its case at ~lines 184-189 expects the Bitbucket script in the Tart base VM row; it must move to the Targets row and the Platform options section. Verbatim (lines 184-189):

        184    it("points a Bitbucket pool at the Bitbucket preparation script", async () => {
        185      await renderCreateForm({ providers: ["docker"] });
        186      await choose("Bitbucket Cloud workspaces", "Acme Bitbucket");
        187      expect(tartInput()?.value).toBe(SERVER_DEFAULT_VM);
        188      expect(descriptionFor("tart-image")).toContain("scripts/prepare-tart-bitbucket-runner-image.sh");
        189    });

  The case drives the "Bitbucket Cloud workspaces" control (line 186) and asserts the script path in the `tart-image` row's description (line 188). Line 187 also shows the `tart` provider auto-add today (the pool starts Docker-only, yet a Tart base VM input appears after picking a workspace). In the new layout T picks the workspace through the Targets row (after a GitHub target, since Bitbucket is not standalone) and asserts the script path in the Platform options section instead of the Tart base VM row.
- `tests/api-tokens-section.test.ts` must keep passing after the `SecretDialog` extraction. Verified: it does not import `SecretDialog` (a private function today); its only source import is `import { CreateTokenSection, TokenListSection } from "../src/features/fleet/api-tokens-section";` (line 7), and it drives the dialog through `CreateTokenSection`. Verbatim from the case "shows the secret once, ignores Escape, and forgets it on close" (lines 147-180; the lines that build the secret fixture and dispatch the keydown are omitted here):

        153      await render(createElement(CreateTokenSection));
        ...
        160      const dialog = document.querySelector("[role=dialog]");
        161      expect(dialog?.textContent).toContain(secret);
        162      expect(dialog?.textContent).toMatch(/only time GridOps shows the secret/);
        163      expect(button("Copy")).toBeTruthy();
        ...
        168      expect(document.querySelector("[role=dialog]")?.textContent).toContain(secret);
        169
        170      click(button("I’ve stored it, close"));
        171      await settle();
        172      expect(document.querySelector("[role=dialog]")).toBeNull();
        173      expect(document.body.innerHTML).not.toContain(secret);
        174      expect(document.activeElement).toBe(byId("api-token-name"));

  Line 174 depends on `onClosed` focusing the name input (`CreateTokenSection` passes `onClosed={() => nameRef.current?.focus()}`, `api-tokens-section.tsx` line 171), so the extracted dialog must keep calling `onClosed` from `onCloseAutoFocus` as today. The strings `CreateTokenSection` must pass to the extracted component are today's hard-coded ones in `SecretDialog` (`api-tokens-section.tsx` lines 186-226): title `Copy your new token`; description `This is the only time GridOps shows the secret for “{revealed.token.name}”. Store it now, for example as a masked CI secret. Once you close this dialog it can’t be shown again.`; secret label `API token secret` (the `<code aria-label>`) with the `CopyButton` `label="API token"` and text `Copy`; details = the Scopes / Expires `<dl>`; close button `I’ve stored it, close`. The extracted component's default close-button text (used when `closeButtonText` is absent, per the Post-create hook contract) is that same `I’ve stored it, close`. So the extraction must keep `CreateTokenSection`'s rendered dialog (title, description, the `aria-label="API token secret"` code element, Scopes/Expires details and the "I’ve stored it, close" button) unchanged, passing them to the extracted component as props; the test then needs no edit.

### Forward-compat note
The forward-compat table in `docs/handoffs/145/survey.md` must also state plainly that a GitLab pool will still need `schemas.ts` and API changes (#137), and that the descriptor contract above (namespaced keys, per-platform payload partials, `requires`/`standalone`, per-kind limits, platform-options slot, post-create hook, runner link builder) is what lets #141 add GitLab without touching the page, picker or options components.
