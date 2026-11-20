# Handoff-F: Phase 6 - One Connections page and one Targets picker for every CI platform

> **Round 4 (3:05 PM PST):** the implementation of the operator's round-4 decisions (W1, W2, the `postCreate` toast, the runner-link label) is the last section of this file, "Round 4". Everything above is the original cycle's handoff and still stands.
>
> **Round 5, S rework (3:45 PM PST):** the 360 px layout fixes for S's D1 and D2 are the final section, "Round 5 (S rework: D1, D2 at 360 px)". Production code only; no test changed.

**Date:** 2026-11-20 (2:02 PM PST)
**Branch:** claude/ui-145-platform-neutral-cf5832 (on top of `b8cbf8d`, T's RED commit)
**Issue:** #145

## What was done
- `src/features/platform-connections/types.ts` (new): the descriptor contract. It holds `PlatformDescriptor`, `TargetKind`, `TargetOption`, `ResolvedSelection` and `TargetContext`, plus the connection binding types (`ConnectionField`, `ConnectionListBinding`, `ConnectionRowView`). It also holds `PostCreateReveal` (exactly the brief's shape), `ManualEntry`, `TargetLimits`, `TargetUpdateValues` and `PrerequisiteEmptyState`.
- `src/features/platform-connections/github.tsx` (new): the GitHub descriptor.
  - Its list binding is page-derived: installations come from the `/settings` data.
  - It renders the installation rows: avatar, account type, Suspended, Configure or "Owner approval required".
  - Kinds: `github.repository` and `github.runner-group`. Runner-group ids are `<installationId>/<groupId>`, with a per-installation manual "Runner group ID" entry.
  - It has total payload partials, initial selection and priority, a read-only summary, the prerequisite empty states and a runner link.
- `src/features/platform-connections/bitbucket.tsx` (new): the Bitbucket descriptor.
  - Its list binding fetches. It binds create, update and remove, using today's copy and the editable fields.
  - Kind: `bitbucket.workspace`. It is not standalone, needs a GitHub kind and requires `tart`. It is offered on create only; settings shows locked chips.
  - `labelRules` call `bitbucketLabelError`.
  - `platformOptions` carries the capacity note, the label rule and `scripts/prepare-tart-bitbucket-runner-image.sh`.
- `src/features/platform-connections/registry.tsx` (new): `platforms` (`[github, bitbucket]`) and `getPlatform(id)`. For an unknown id, `getPlatform` returns a fallback: name = id, a help icon, no targets.
- `src/features/platform-connections/connections-section.tsx` (new): `ConnectionsSection`, one list for every descriptor.
  - Page-derived bindings render rows directly. Each fetching binding gets its own child with one `useQuery` (`["platform-connections", id]`), `LoadingRows` and `InlineError` with Retry.
  - Add connection asks for the platform first, then shows only that platform's fields. Errors stay inline. On success it calls `invalidateQueries` on that platform's key, then runs `postCreate`, whose result goes to the reveal dialog.
  - Edit and Remove go through the descriptor bindings and are gated on the list's `canManage`.
- `src/features/platform-connections/github-app-section.tsx` (new): `GitHubAppSection({ configuration, githubApp, user })`, moved from `settings.github.tsx`.
  - It holds the checks, the endpoints, the Create section (admin, App not configured) and the "Adding another GitHub account" callout.
  - Refresh access, Install and Manage App (admin only) are its header actions.
- `src/features/runner-pools/targets.ts` (new): pure rules. `targetKinds`, `unavailableReason` (exclusion, requires and per-kind limit), `targetPlanCheck`, `anyStandaloneKindAvailable`, `prerequisiteEmptyState`, `minMaxCount`, `requiredProviders`, `prioritySupported`, `priorityCandidates`, `labelRules`, `labelError`, `initialSelection`, `initialPriorityKeys`, `readOnlySummaries`, `buildCreatePayload` and `buildUpdatePayload`.
- `src/features/runner-pools/targets-row.tsx` (new): `TargetsRow`, the extended `SearchableMultiSelect` grouped by kind.
  - Each kind has its own load state (loading, error with Retry, or not connected with a link to `/settings/connections`).
  - Disabled reasons come from `targets.ts`. A "one" kind replaces its previous pick.
  - Locked chips cover kinds that are not offered. A manual-entry input appears for a picked manual option.
- `src/features/runner-pools/platform-options-section.tsx` (new): `PlatformOptionsSection`. It is collapsed by default and renders only for the targeted platforms that have `platformOptions` (help list plus `Component` slot).
- `src/components/secret-reveal-dialog.tsx` (new): `SecretRevealDialog`, extracted from `SecretDialog` with the brief's prop interface. Escape and outside clicks are ignored, and the default close text is `I’ve stored it, close`.
- `src/routes/settings.connections.tsx` (new): a thin route with no loader. It reads `getRouteApi("/settings")` and renders the single members-only Read-only callout, `ConnectionsSection` and `GitHubAppSection`.
- `src/components/ui/searchable-multi-select.tsx`: new optional `SearchableMultiSelectOption` fields (`group`, `disabled`, `disabledReason`, `icon`, `locked`) and a `groups` prop (heading order plus per-group status). Ungrouped options render as before.
- `src/routes/_app/runner-pools.new.tsx`: one Targets row and the Platform options section replace the Scope, Installation, Repositories, Runner group and Bitbucket rows.
  - The payload is `buildCreatePayload` plus the shared rows.
  - The empty state and the create gate come from `anyStandaloneKindAvailable` / `prerequisiteEmptyState`.
  - Picking a kind that needs a provider adds it (`requiredProviders`).
  - Labels help, placeholder and validation come from `labelRules`. The Maximum floor comes from `minMaxCount` and the preview from `targetPlanCheck`.
- `src/routes/_app/runner-pools.$poolId.settings.tsx`: the same Targets row and options section, seeded with `initialSelection`.
  - The payload is `{ poolId, ...buildUpdatePayload(...), shared rows }`.
  - Priority shows when `prioritySupported`, and its options are the selected repository targets.
  - `ReadOnlyPool` lists one Destination line per platform (`readOnlySummaries`).
- `src/features/runner-pools/tart-base-vm-row.tsx`: the `bitbucket` prop is removed, and the generic script text stays.
- `src/components/pool-plan-preview.tsx`: a `targetCheck` prop replaces `repositoryCount` and `scope`.
- `src/features/runners/runner-row.tsx`: a registry-driven CI-platform badge on the row and a "CI platform" line (with the runner link) in the details sheet. The delete confirm uses `getPlatform(runner.platform).name`.
- `src/features/runner-pools/schemas.ts`: exports `bitbucketLabelError(labels)`, which `validateBitbucketConnections` now calls. It accepts and rejects exactly what it did before.
- `src/features/platform-connections/platform-connections.functions.ts`: `getBitbucketConnections(signal?)`, same endpoint.
- `src/features/fleet/api-tokens-section.tsx`: uses `SecretRevealDialog` with today's strings. `SecretDialog` is removed.
- `src/routes/settings.github.tsx` and `src/routes/settings.integrations.tsx` are now redirect-only, keeping their exact one-line declarations. `src/routes/_app/platform-connections.tsx` now redirects to `/settings/connections`.
- `src/routes/settings.index.tsx`: the `?appError=` redirect goes to `/settings/connections`, and its comment is updated.
- `src/routes/settings.tsx`: one "Connections" nav item replaces GitHub and Bitbucket.
- `src/routes/_app/webhooks.tsx`: the empty-state link goes to `/settings/connections`.
- `docs/handoffs/145/survey.md` (new): the GitLab forward-compat table and the two required statements.

## Design decisions
- **Kind caps take form limits, not `context`.** `limit(limits: { maxCount })`: `TargetContext` holds only loaded data (round-3 decision), and the cap is the live Maximum field, as today's `maxSelected={maxCount}`. `TargetsRow` takes an optional `limits` prop. Without it, kinds are uncapped, as in the forward-compat harness.
- **Offered kinds per form: `offered(context)` instead of a static `offeredIn`** (A finding 2).
  - GitHub's kinds read `context.pool.scope` inside `github.tsx`.
  - Bitbucket is offered on create only, so on settings its targets are `locked` chips that are not listed in the picker.
- **Manual runner-group entry.** `manualEntry.parse(input, connectionId)` follows A finding 3. A manual option has a stable key (`github.runner-group:<inst>/manual`) and its `nativeId` follows the typed id. It starts from `defaults.runnerGroupId` on create and `pool.runnerGroupId` on update, as the old input did. Organizations whose group list fails or is empty also get the manual option, as today.
- **Two orders** (A finding 1). `ResolvedSelection` keeps click order, and `TargetOption.loadIndex` keeps list order. The GitHub create partial takes `installationId` from the selected repository with the lowest `loadIndex`.
- **Neutral update values.** `toUpdatePayload(selection, pool, { priorityKeys, priorityWaitMinutes })` (A finding 2). The GitHub partial maps keys back to repository ids and applies today's "only selected repositories" filter.
- **Add connection permission.** The parent observes each creatable platform's list query with `enabled: false`, so the platform's child component stays the only fetcher and Retry stays per-platform. The button shows when a creatable platform's loaded list says `canManage`.
- **Read-only callout:** the route owns it and shows it only to members (A finding 4a). Copy: "A GridOps administrator adds or changes platform connections and manages the GitHub App."
- **Installations description** (A finding 4b): it is kept, merged into the GitHub App section's description.
- **Actions placement** (A finding 4d): Refresh access, Install and Manage App are the actions of the GitHub App section's first `SettingsSection`.
- **Labels on settings:** unchanged (description "self-hosted, OS, architecture…", placeholder "gpu, large"). The settings form never branched on Bitbucket for labels, so it does not start now.
- **`requiredProviders` auto-add on the create form only.** The old settings form never auto-added `tart`, and settings Bitbucket targets are locked anyway.

## Reuse / extend-vs-new
- EXTEND `SearchableMultiSelect`: optional fields and a `groups` prop only. Fleet callers are untouched, and it does not import the registry.
- EXTEND (extract) `SecretDialog` → `src/components/secret-reveal-dialog.tsx`, used by API tokens and the Connections page.
- EXTEND `validateBitbucketConnections` by exporting `bitbucketLabelError`. The regex still appears only in `schemas.ts`.
- EXTEND (move) the `BitbucketSettings` / `ConnectionRow` logic into `ConnectionsSection`, driven by the Bitbucket bindings, with copy unchanged.
- EXTEND (move) the GitHub page sections into `github-app-section.tsx`, with calls, arguments and success text unchanged.
- EXTEND `tart-base-vm-row.tsx`, `pool-plan-preview.tsx` and the settings shell components.
- NEW, as the Reuse map specifies: `registry.tsx`, `github.tsx`, `bitbucket.tsx`, `types.ts`, `connections-section.tsx`, `targets.ts`, `targets-row.tsx`, `platform-options-section.tsx` and `settings.connections.tsx`.

## Architecture notes for A
- **New module boundary:** `src/features/platform-connections/` (types, registry, descriptors, connections section, GitHub App section) and `src/features/runner-pools/targets*.ts(x)`.
- **Import direction:**
  - `registry.tsx` imports the descriptors, and the descriptors import `types.ts`. Descriptors also import `runner-pools.functions` / `platform-connections.functions`, `schemas.ts` (the label predicate), `~/components/ui/*`, `~/lib/*` and `lucide-react`, never each other or route files.
  - `targets.ts` imports only types.
  - `targets-row.tsx` and `platform-options-section.tsx` import `registry` only for the default `descriptors`.
  - Shared UI (`SearchableMultiSelect`, `pool-plan-preview`, `secret-reveal-dialog`) never imports the registry.
- **Changed public interfaces:**
  - `PoolPlanPreview` props (`targetCheck`);
  - `TartBaseVmRow` (no `bitbucket`);
  - `getBitbucketConnections(signal?)`;
  - `SearchableMultiSelect` (optional additions).
- **No API, DB or OpenAPI change.** Nothing under `crates/`, `migrations/` or `docs/openapi/` was touched.

## Deviations from spec / wireframe
- The GitHub App section's first `SettingsSection` is titled "GitHub App" (it was "Connection" on the old page), so it doesn't read as a duplicate of the new "Connections" list heading. Its description folds in the old page's intro and the Installations description.
- `TargetKind.limit` takes `TargetLimits` rather than `context` (see Design decisions). `TargetKind.offered(context)` replaces `offeredIn`, as A recommended.
- `ReadOnlyPool` now has a separate "Destination" section with one line per platform. It used to be a single "Destination" value inside "Runner definition".
- The new-pool Targets row has no "N selected across M accounts" summary. The chips and the picker's count replace it.

## Tier 1 self-check (incl. tests now GREEN)
Run in this worktree on 2026-11-20, around 2:00 PM PST:

    $ npm run build      -> vite build ✓ built in 1.15s; then tsc --noEmit (exit 0)
    $ npm run lint       -> eslint src tests *.config.ts eslint.config.js --max-warnings=0 (exit 0, no output)
    $ npm run typecheck  -> tsc --noEmit (exit 0)
    $ npx vitest run     -> Test Files  46 passed (46)   Tests  445 passed (445)
    $ npx vitest run <T's 10 files + tests/api-tokens-section.test.ts>
                         -> Test Files  11 passed (11)   Tests  105 passed (105)
    $ grep -rn -e "settings/github" -e "settings/integrations" src --exclude=routeTree.gen.ts | grep -v -E '^src/routes/settings\.(github|integrations)\.tsx:[0-9]+:export const Route = createFileRoute\("/settings/(github|integrations)"\)\(\{$'
                         -> (no output)
    $ grep -rn appError src   -> only src/routes/settings.index.tsx:7-9
    $ grep -niE "github|bitbucket" src/features/runner-pools/targets-row.tsx src/features/runner-pools/platform-options-section.tsx src/features/platform-connections/connections-section.tsx
                         -> (no output)
    $ git diff b8cbf8d --stat -- tests   -> (empty: no test file touched)

## Evidence appendix
`docs/handoffs/145/evidence.md`

## Tests that look wrong (for T)
None.

## Known issues
- The brief's last criterion, a headless Playwright walkthrough of every control on the Connections page and both pool forms, is not part of F's work (F writes no tests) and does not exist yet. It belongs to T or U in the later phases.
- These were not run in a browser, only under Vitest/jsdom: the per-kind load states in the picker (loading, error with Retry, not connected), the runner details "CI platform" link, and the Add connection dialog's Back button.

## Files changed
- src/components/pool-plan-preview.tsx
- src/components/secret-reveal-dialog.tsx (new)
- src/components/ui/searchable-multi-select.tsx
- src/features/fleet/api-tokens-section.tsx
- src/features/platform-connections/bitbucket.tsx (new)
- src/features/platform-connections/connections-section.tsx (new)
- src/features/platform-connections/github-app-section.tsx (new)
- src/features/platform-connections/github.tsx (new)
- src/features/platform-connections/platform-connections.functions.ts
- src/features/platform-connections/registry.tsx (new)
- src/features/platform-connections/types.ts (new)
- src/features/runner-pools/platform-options-section.tsx (new)
- src/features/runner-pools/schemas.ts
- src/features/runner-pools/targets-row.tsx (new)
- src/features/runner-pools/targets.ts (new)
- src/features/runner-pools/tart-base-vm-row.tsx
- src/features/runners/runner-row.tsx
- src/routes/_app/platform-connections.tsx
- src/routes/_app/runner-pools.$poolId.settings.tsx
- src/routes/_app/runner-pools.new.tsx
- src/routes/_app/webhooks.tsx
- src/routes/settings.connections.tsx (new)
- src/routes/settings.github.tsx
- src/routes/settings.index.tsx
- src/routes/settings.integrations.tsx
- src/routes/settings.tsx
- docs/handoffs/145/survey.md (new, brief deliverable), docs/handoffs/145/evidence.md (new), docs/handoffs/145/handoff-F.md (this file), docs/handoffs/145/decisions.md (appended)


---

# Handoff-F, Round 4: Add connection offers every platform; one generic empty state (#145)

**Date:** 2026-11-20 (3:05 PM PST)
**Branch:** claude/ui-145-platform-neutral-cf5832 (on top of `2fb904c`, T's round-4 RED commit)
**Issue:** #145
**Brief section:** `docs/handoffs/145-brief.md`, "Round-4 operator decisions after the spec audit" (binding)

## What was done
- `src/features/platform-connections/types.ts`:
  - `ConnectionBindings.emptyText` is now `(page) => string | null`.
  - New `ConnectionBindings.chooserDescription: string` (required).
  - New `ConnectionBindings.connectLink?: (page) => { label; description: ReactNode; href: string | null; unavailableText? }`.
  - `PlatformDescriptor.externalLinks` is deleted.
- `src/features/platform-connections/github.tsx`:
  - `chooserDescription` and `connectLink` use the brief's copy. The link's label is "Install GitHub App" and its `href` is `githubApp.installUrl`. While there is no App, `href` is null and the unavailable text is "Create the GitHub App first (below on this page)."
  - `emptyText` returns null while `page.data?.githubApp` is absent.
  - `externalLinks` is removed.
- `src/features/platform-connections/bitbucket.tsx`: `chooserDescription` uses the brief's copy.
- `src/features/platform-connections/registry.tsx`: the unknown-platform fallback gets a `chooserDescription`. It is never listed in the chooser, because it has no `create` and no `connectLink`.
- `src/features/platform-connections/connections-section.tsx` (shared code; it still names no platform):
  - **List states:** the section observes every fetching list (`enabled: false`) and gives each platform a state of loading, error or ready. Page-derived lists are always ready.
  - **Generic empty state (W2):** it shows only when every list is ready and empty. It reads "No connections yet", then one of two lines:
    - for someone who can add a connection: "Connect a CI platform to start assigning it to runner pools.", with an "Add connection" call to action;
    - otherwise: "A GridOps administrator can add platform connections."
  - **Per-platform empty rows:** these render only outside the generic state, and only when `emptyText` is non-null.
  - **Add connection (W1):** this is now one controlled `AddConnectionDialog` owned by the section. The header button and the empty-state call to action both open it. Step 1 lists, in registry order, every descriptor that has `create` or `connectLink` and whose ready list says `canManage`. Each entry shows the icon, the name and the `chooserDescription`.
    - Picking a `create` platform shows today's form.
    - Picking a `connectLink` platform shows its description, then Back and a primary `<a target="_blank" rel="noreferrer">` with the link's label. When `href` is null, the primary is a disabled button and a warning callout shows the `unavailableText`.
    - Back returns to step 1 from either step.
  - **`postCreate` failures:** a throw or rejection after the dialog closed becomes `toast.error(message)`. `submit` now catches only `create` errors.
- `src/features/runners/runner-row.tsx`: the details sheet's platform link carries `title={link.label}`.
- `docs/handoffs/145/survey.md`: a new row covers "Offering GitLab in Add connection" (`chooserDescription`, `create` / `connectLink`). The `emptyText` row now gives the nullable signature, and statement 2 names the Add connection bindings. The survey never mentioned `externalLinks`.

## Design decisions
- **Fetching rows stay mounted under the generic empty state.** I rejected unmounting them when everything is empty. With no active observer, `invalidateQueries` after a create would mark the query stale without refetching it: the parent's observer is `enabled: false`, so it does not count as active. The generic state would then stick after the first connection was added. Instead, mounted rows render nothing while the generic state shows.
- **One dialog, two triggers.** The dialog was lifted into the section and remounts per opening (`key={addSession}`), so each opening starts at step 1 with empty values, as `openDialog` did before. The header "Add connection" is hidden while the generic state shows, so the page has only one such button.
- **`create` wins over `connectLink`** when a descriptor has both. The brief defines the two steps separately and no current descriptor has both bindings.
- **Admin copy rule:** "can manage at least one platform" is read as "at least one platform is addable" (`create` or `connectLink`, plus `canManage`). The admin copy therefore always comes with its button. With the production registry the two readings match.
- **Runner link label:** I used `title` rather than `aria-label`, so the link's accessible name stays the visible platform name and the label shows as its tooltip. The brief allows either.
- **Link step description:** `connectLink().description` (a `ReactNode`) renders in the dialog body as a `div`, and `DialogDescription` shows the `chooserDescription`. This avoids putting arbitrary nodes inside the description's `<p>`.

## Reuse / extend-vs-new
- EXTEND `ConnectionsSection` / its Add connection dialog. There is no new component file and no parallel path. The empty state reuses `EmptyState` (`~/components/page`) and the unavailable text reuses `Callout`.
- EXTEND the descriptor contract in `types.ts` with only the fields the brief names. `externalLinks` is deleted.

## Architecture notes for A
- Public contract change in `types.ts`, as the brief specifies: `chooserDescription` (required), `connectLink` (optional), nullable `emptyText`, and no `externalLinks`. There is no new module, dependency or import direction. `connections-section.tsx` still imports only `registry` and `types` from the platform side.
- No API, DB or OpenAPI change.

## Deviations from spec / wireframe
- The wireframe's C2 splits the GitHub step into a short description plus an info callout ("GridOps then picks the installation up…"). The contract carries one `description`, so the full sentence pair renders as one paragraph.
- The wireframe's admin empty-state copy (line 239) adds a second sentence that names GitHub and Bitbucket. The brief's round-4 copy is neutral and lives in shared code, so only "Connect a CI platform to start assigning it to runner pools." is rendered.
- The wireframe's note "A single-platform registry skips straight to step 2" is not implemented. It is not in the round-4 criteria, and this was already the case in the first cycle.

## Tier 1 self-check (incl. tests now GREEN)
Run in this worktree on 2026-11-20, around 3:00 PM PST:

    $ npm run typecheck  -> tsc --noEmit (exit 0)
    $ npm run lint       -> eslint src tests *.config.ts eslint.config.js --max-warnings=0 (exit 0, no output)
    $ npm run build      -> ✓ built in 1.12s; then tsc --noEmit (exit 0)
    $ npx vitest run     -> Test Files  46 passed (46)   Tests  461 passed (461)
    $ grep -niE "github|bitbucket" src/features/runner-pools/targets-row.tsx src/features/runner-pools/platform-options-section.tsx src/features/platform-connections/connections-section.tsx
                         -> (no output)
    $ grep -rn externalLinks src docs/handoffs/145/survey.md   -> (no output)
    $ git diff --stat -- tests   -> (empty: no test file touched)

The full 461 includes T's 16 round-4 tests, which were 15 failing plus 1 typecheck-gated at RED. Test #12 (`expectTypeOf` on the contract) is gated by `tsc`, and `tsc` passes.

## Evidence appendix
`docs/handoffs/145/evidence.md`, section "Round 4 (F, 2026-11-20)".

## Tests that look wrong (for T)
None.

## Known issues
- The round-4 surfaces were run only under Vitest/jsdom, not in a browser: the chooser, the link step (enabled and disabled), Back, the generic empty state for an admin and for a member, and the `postCreate` toast. Under the brief, U's headless walkthrough and its saved screenshots cover them.

## Files changed
- src/features/platform-connections/types.ts
- src/features/platform-connections/github.tsx
- src/features/platform-connections/bitbucket.tsx
- src/features/platform-connections/registry.tsx
- src/features/platform-connections/connections-section.tsx
- src/features/runners/runner-row.tsx
- docs/handoffs/145/survey.md (brief deliverable), docs/handoffs/145/evidence.md (appended), docs/handoffs/145/decisions.md (appended), docs/handoffs/145/handoff-F.md (this section)


---

# Round 5 (S rework: D1, D2 at 360 px)

**Date:** 2026-11-20 (3:45 PM PST)
**Branch:** claude/ui-145-platform-neutral-cf5832 (on top of `dd934c3`, S's REWORK commit)
**Issue:** #145
**Input:** `handoff-S.md` (REWORK, production only: D1, D1b and D2 at 360 px; the Targets dropdown clip is advisory)

## What was done
- `src/components/settings-ui.tsx` (`SettingsSection` header, D2):
  - The header row is now `flex-wrap`.
  - The title block is `min-w-0 flex-[1_1_12rem]`: it fills the row as before, and when the actions would squeeze it under 12rem the actions wrap onto their own line below the heading.
  - The actions block adds `max-w-full flex-wrap`, so a wrapped group wider than the screen wraps its own buttons instead of running off the edge. This keeps "Manage App" on screen.
- `src/features/platform-connections/connections-section.tsx` (`ConnectionRow`, D1 and D1b):
  - The row is `flex-wrap` below `sm` (`sm:flex-nowrap`). The text block's basis is the row width minus the leading avatar or dot (`flex-[1_1_calc(100%-3rem)]`, `sm:flex-1`).
  - `view.aside` and the Edit/Remove actions now sit in one trailing wrapper, `w-full justify-end` below `sm` and `sm:w-auto sm:shrink-0` from `sm` up. On a phone they go to their own line under the text, right-aligned. On wider screens they are the same right-hand cluster as before, with the same `gap-3` between status and actions.
  - The name line is `flex-wrap`: the name (`min-w-0 max-w-full truncate`) keeps its full width, and the platform badge and `view.badges` wrap under a long name instead of squeezing it.
  - The wrapper renders only when there is an aside or actions (`showActions`, the same condition as before).

## Design decisions
- **D2 is fixed in the shared `SettingsSection`, wrapping only when the actions don't fit.** Alternatives:
  - A local header in `github-app-section.tsx`. It would have duplicated `SettingsSection`'s header markup, which is a parallel path.
  - `SettingsLayout`'s always-stack-below-`sm` pattern (`flex-col sm:flex-row`). It would also have moved the fleet-profile Save button under its heading on phones, a page this story does not own.
  
  With `flex-wrap` and a 12rem title basis, a header wraps only when the actions plus 12rem don't fit. Its three users with `actions` are below. I grepped them first, as S asked, and confirmed them with a multiline search for `<SettingsSection ... actions=`.
  - The fleet-profile Save (pool settings) still fits beside its heading at 360. It is pixel-identical to the pre-fix build at 360, 768 and 1280.
  - Connections "Add connection" now wraps below the description at 360. It previously sat beside a 178 px text column. This is a #145 surface; 768 and 1280 are unchanged.
  - The GitHub App header wraps at 360 for administrators (three actions) and for members (two actions).
- **D1 stacks the trailing controls rather than hiding anything.** No row content is dropped on phones. The descriptor's own `hidden sm:inline` on Bitbucket's "Verified …" aside is left as the descriptor chose it.

## Reuse / extend-vs-new
Both changes modify existing objects (`SettingsSection`, `ConnectionRow`). No new component, file or prop. This is a spec'd shared-code edit: S's REWORK names `SettingsSection` as the place to fix and asks for the grep of its users, which is done above.

## Architecture notes for A
Layout classes only. No prop, type, export or dependency change. `SettingsSection`'s interface is unchanged. Its users with `actions` are `fleet-profile-section.tsx:231`, `connections-section.tsx:79` and `github-app-section.tsx:46`; users without `actions` render the same header DOM with no actions block, and the pixel check below covers `settings.general`.

## Deviations from spec / wireframe
None. The wireframe did not draw narrow widths; this follows S's acceptance for 360 px.

## Tier 1 self-check (incl. tests now GREEN)
```
npm run lint        -> eslint src tests *.config.ts eslint.config.js --max-warnings=0   (clean)
npx tsc --noEmit    -> exit 0
npx vitest run      -> Test Files 46 passed (46); Tests 461 passed (461)
npm run build       -> built; typecheck step clean
built CSS: .flex-\[1_1_12rem\]{flex:12rem}   .flex-\[1_1_calc\(100\%-3rem\)\]{flex:calc(100% - 3rem)}   .sm\:flex-nowrap present
```
I also ran a render check beyond Tier 1, because S's acceptance is a render check and layout does not exist in jsdom. This is not the U/S gate; U should re-walk.
- **Setup:** S's repro setup (`screenshots/repro/start-api.sh`, `fake-github.mjs`, S's seeded DBs). The fixed build ran on 18097/18098 and a pre-fix build of `HEAD` (`git archive`) ran on copies of the same DBs on 18101/18102. Script: `screenshots/repro/f-measure.mjs`. Viewports were 360, 768 and 1280 wide, 3400 tall, so the whole page is in the viewport (the app scrolls inside its own container).
- **360, populated administrator:** names measured `acme-labs` 66 px, `ada-admin` 67, `Acme Bitbucket` 98 and `Platform Team` 90, none clipped (`scrollWidth <= clientWidth`). Before the fix these were 0, 20, 45 and 45. "Manage App" is on screen: the off-screen-control list is only the visually hidden "Skip to content", where before it also had "Manage App". The GitHub App title block is 320 px wide and the actions sit on two lines below it.
- **360, populated member:** "Owner approval required" no longer overlaps "Organization" (`ownerOverlapsOrg: false`; it was `true` before the fix).
- **768 and 1280:** pixel-compared (`compare -metric AE`) against the pre-fix build for every Connections state (populated or empty, administrator or member), pool settings (repository and organization pools, including the fleet-profile Save header), new pool and General.
  - The only differing pixels are the footer build string and, on Connections, the Endpoints URLs, whose port differs between the two servers (`:18097` vs `:18101`; cropped and checked).
  - 360 is identical as well, except Connections.
- **Console errors and horizontal overflow:** none on any surface at any width.
- **Saved to `docs/handoffs/145/screenshots/`:**
  - `f-connections-{populated,empty}-{admin,member}-360.png`: after the fix.
  - `f-prefix-*-360.png`: the same views before the fix.
  - `f-geometry-{after,before}.json`: the measurements.
- **Servers:** all were stopped by port (18097, 18098, 18099, 18101, 18102; all five were confirmed free before I started).

## Evidence appendix
`docs/handoffs/145/evidence.md`: three entries appended under "F round 5":
- the other `SettingsSection` user with `actions`;
- the 20 px page padding at 360 that sets the 320 px header width;
- Bitbucket's phone-hidden aside.

## Tests that look wrong (for T)
None. No test changed. No jsdom test can see these layouts, and as S says a class-name assertion would be mock-shaped.

## Known issues
- **Member Bitbucket rows at 360 are about 8 px taller than before.** Bitbucket's aside is `hidden sm:inline`, so the trailing wrapper renders but shows nothing on a phone, and the row's `gap-y-2` adds a little space under the text. Cosmetic. I left it rather than override the descriptor's own visibility choice.
- **The Targets dropdown clip (S advisory 1) is not folded in.** It is pre-existing and not cheap. Both `SearchableMultiSelect` and `SearchableSelect` draw their list `absolute` (`searchable-multi-select.tsx:223`, `searchable-select.tsx:186`) inside `SettingsSection`'s `overflow-hidden` card. Fixing it means either portalling and positioning the list in both shared selects, or removing the card's `overflow-hidden` for every settings page. Either is a separate change with its own visual surface. It needs a follow-up issue, which is the orchestrator's or operator's to file.
- A-dup's two warns stand, as S notes. No F action this round.

## Files changed
- `src/components/settings-ui.tsx`
- `src/features/platform-connections/connections-section.tsx`

Non-production evidence written this round: `docs/handoffs/145/screenshots/f-*.png`, `f-geometry-*.json`, `screenshots/repro/f-measure.mjs`, plus appended `evidence.md` and `decisions.md` entries.
