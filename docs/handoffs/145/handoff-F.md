# Handoff-F: Phase 6 - One Connections page and one Targets picker for every CI platform

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
