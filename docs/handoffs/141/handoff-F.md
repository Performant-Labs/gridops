# Handoff-F: Phase 5 - GitLab in the shared connections and targets UI (#141)

**Date:** 2026-11-20
**Branch:** claude/gitlab-141-ui-3ae7f5
**Issue:** #141 (epic #136)

## What was done
- `src/features/platform-connections/gitlab.tsx` (new). This is the GitLab descriptor. It holds:
  - **Connection form:** name, address (defaults to `https://gitlab.com`; must be http(s) with no query or fragment), a Project/Group select, path (`segment(/segment)*`) and token. The token help names `create_runner`, `read_api` and Owner.
  - **Row actions:** Verify, and Rotate webhook secret (which asks first and then reveals the new secret).
  - **Post-create reveal:** the secret, then the webhook URL with its own Copy button, then the Job events steps.
  - **Target kinds:** `gitlab.project` and `gitlab.group`.
  - **Pool payload partials:** create and update, plus the settings selection and summary.
  - **Tag rule** and the **Run untagged jobs** Platform options switch.
  - **Runner link:** `<baseUrl>/<scopePath>/-/runners/<id>`.
- `src/features/platform-connections/platform-connections.functions.ts`: the GitLab API functions (list, create, PATCH update, delete, verify, rotate) and their types.
- `src/features/platform-connections/registry.tsx`: GitLab is registered last, with a comment that order matters. Adds `platformForDeliveryId(id)`, which falls back to `githubPlatform` by reference.
- `src/features/platform-connections/types.ts`: the generic contract additions.
  - `PostCreateReveal` is renamed to `SecretReveal`.
  - `ConnectionField` gains `select`, `options`, `defaultValue` and `validate`.
  - New `ConnectionRowAction` type, and `ConnectionBindings.actions`.
  - `platformOptions` gains `initialValues` and a `Component` that takes `{ values, onChange }`.
  - New `PlatformOptionValues` and `TargetCreateValues`; `TargetUpdateValues` gains `platformOptions`.
  - `toCreatePayload` takes an optional third `values` argument.
  - New descriptor value `deliveryDestinationFallback`.
- `src/features/platform-connections/connections-section.tsx`: generic changes only.
  - Row actions sit between Edit and Remove. They run through `useAction`, and the row is disabled while one runs.
  - `onReveal` is passed from `ConnectionsSection` → `FetchedConnections` / `ConnectionRows` → `ConnectionRow`, so the section keeps one reveal dialog.
  - `FieldInput` renders a select with `SearchableSelect` (`searchable={false}`). It shows `aria-invalid` and a `text-danger` line for a `validate` problem.
  - Defaults seed the create values. A validate problem blocks create and edit.
- `src/features/platform-connections/github.tsx`: gap 4. Explicit `repository` / `organization` branches in `toUpdatePayload`, `initialSelection` and `readOnlySummary`, with empty values for any other scope. Adds `deliveryDestinationFallback: "GitHub App"`.
- `src/features/runner-pools/targets.ts`: adds `initialPlatformOptionValues(descriptors, pool?)`. `buildCreatePayload` takes an optional fourth `values` argument.
- `src/features/runner-pools/platform-options-section.tsx`: optional `values` and `onChange` props. Each descriptor's `Component` gets its own entry.
- `src/routes/_app/runner-pools.new.tsx` and `src/routes/_app/runner-pools.$poolId.settings.tsx`: each holds the platform option values, seeded by `initialPlatformOptionValues`. They pass the values to the section and to the payload builders. Neither names a platform.
- `src/routes/_app/webhooks.tsx`: `PlatformBadge` in each row and beside the sheet heading. The destination fallback comes from the delivery's platform: "GitHub App" for GitHub, nothing in the list for other platforms, and "—" in the sheet.
- `src/features/runner-pools/schemas.ts`:
  - `scope` accepts `"gitlab"`.
  - `gitlabConnectionIds`: on create, a UUID array, max 100, default `[]`; optional on update.
  - `runUntagged`: default false on create, optional on update.
  - `validateGitlabTargets` (create only) and `validateGitlabConnections` (create and update).
  - Exports `gitlabLabelError`.
- `src/features/runner-pools/runner-pools.functions.ts`: `GitlabPoolConnection` type. Optional `gitlabConnections` on the pool options and pool detail. Optional `gitlabConnectionIds` and `runUntagged` on the detail. `scope` gains `"gitlab"`.
- `src/features/operations/operations.functions.ts`: `Runner.platform` gains `"gitlab"`. Adds the optional `gitlabRunnerId`, `gitlabBaseUrl` and `gitlabScopePath`.
- `crates/gridops-api/src/resources.rs`: in the runners list, selects `r.gitlab_runner_id` and LEFT JOINs `gitlab_connections gc` for `gc.base_url` / `gc.scope_path`. Emits `gitlabRunnerId`, `gitlabBaseUrl` and `gitlabScopePath`, which are null otherwise.

## Design decisions
- **Scope cap not applied.** The change touches 15 production files, more than the cap's guideline of about 6, so I checked whether it should be split. I did not split it:
  - The brief fixes the exact file list, and A passed it in round 3.
  - The issue is one cohesive feature, not a polish batch.
  - T's single RED suite spans all of it, so splitting it would leave every sub-phase red.
  - The backend part is one query.
- **The row action's reveal value.** `useAction` returns a boolean, not the action's value. So the row wraps `action.run(item)` in a closure that keeps the returned reveal, and calls `onReveal` once `useAction` resolves. This keeps one `useAction` path for the toast, confirm and refresh. The alternative was to let `useAction` return the value, but that changes a shared hook used across the app.
- **The validation display rule.** A problem is shown when the field holds a value, or after a refused submit. That way an empty Path field is not red when the form opens, but an empty-path submit is still blocked and explained. The address may be blank (it sends the default), so its `validate("")` is null and the field is not `required`.
- **runUntagged on update** is sent only when it differs from the pool's stored value (wireframe F1). When the form holds no GitLab option value, the stored value is kept, so a caller that passes no `platformOptions` never flips it.
- **Settings option source.** `load` on update fetches `getGitlabConnections(signal)`, and on create reads `options.gitlabConnections`. Both filter by `scopeKind`. Option keys are `<kindId>:<connection id>`, the same as `initialSelection`'s.
- **`raisesMinMaxCount: true` on the GitLab kinds.** Each connection registers its own runner, and the server caps the ids at `maxCount`. So the Maximum input's floor follows the selection, as it does for repositories.
- **Exclusion.** The GitLab kinds `excludes` the three GitHub and Bitbucket kind ids, with "Can’t combine with GitLab targets". `unavailableReason` is symmetric, so the reverse direction uses GitHub's own reasons (evidence.md).
- **Schema tag rule.** `validateGitlabConnections` also applies `gitlabLabelError` when GitLab ids are present. This mirrors where `validateBitbucketConnections` applies the Bitbucket rule. The shared 64-character cap is unchanged.
- **Badge dot.** GitLab uses `bg-warning` (amber, the closest token to GitLab's orange). The icon is lucide's `Gitlab`, which exists in the installed 0.552.0.

## Reuse / extend-vs-new
EXTEND, per the brief and A. The Bitbucket descriptor is the model: one connection is one option, the wire fields go in `schemas.ts` and `runner-pools.functions.ts`, and the label rule is shared by the schema and the descriptor.
- `gitlab.tsx` is the one new module. It is the descriptor the #145 contract asks each platform to add ("one more descriptor module plus one line in `registry.tsx`").
- Every shared change extends an existing object: `ConnectionBindings`, `ConnectionField`, `platformOptions`, `TargetUpdateValues`, `buildCreatePayload`, `PlatformOptionsSection`, `FieldInput` and `ConnectionRow`. None adds a parallel path.
- The reveal reuses `SecretRevealDialog` and `CopyButton` unchanged. The select reuses `SearchableSelect`. The delivery badge reuses the exported `PlatformBadge`. The backend edit extends the one existing runners-list query.

## Architecture notes for A
- **Layers touched:** the descriptor contract (`types.ts`) and registry; the shared Connections section and Platform options section; the pool routes (values plumbing only); the deliveries page; the pool schema and wire types; one backend SQL query and its JSON.
- **New public contract members:** `SecretReveal` (a rename), `ConnectionRowAction`, `ConnectionBindings.actions`, `ConnectionField.{options, defaultValue, validate}` and `type: "select"`, `platformOptions.{initialValues, Component props}`, `PlatformOptionValues`, `TargetCreateValues`, `TargetUpdateValues.platformOptions`, the optional third `toCreatePayload` argument, `PlatformDescriptor.deliveryDestinationFallback`, `targets.initialPlatformOptionValues`, the optional fourth `buildCreatePayload` argument, and `registry.platformForDeliveryId`.
- **Dependency directions:**
  - `webhooks.tsx` now imports `PlatformBadge` from `connections-section.tsx` and `platformForDeliveryId` from the registry.
  - `types.ts` imports the `ConfirmOptions` type from `confirm-dialog.tsx`.
- **Backend contract:** `GET /api/v1/runners` items gain three always-present, nullable keys. This route is not in the OpenAPI document, so there is no OpenAPI change.
- **Neutral-source check:** the GitLab name appears only in `gitlab.tsx`, the API functions, the three wire-field files, and the registry's import and array entry (`tests/platform-neutral-source.test.ts` is green).

## Deviations from spec / wireframe
- **Plan sentence (E3).** The wireframe has "2 GitLab targets selected. Each registers a runner…". A kind's `planNote` receives only that kind's count, so with a project and a group both selected, a count would be wrong. The note is instead "Each selected GitLab project or group registers its own runner; the N-runner maximum must cover them all." It is still `ok: false` when the count is over the maximum.
- **Not-connected status (E4).** The wireframe has one string for both kinds. Each kind says its own: "No GitLab projects are connected yet" / "No GitLab groups are connected yet". With only projects connected, the Group kind would otherwise claim nothing is connected.
- **List destination.** A GitLab delivery row shows no destination text (wireframe H1); the sheet shows "—" (H2). No other deviation.

## Tier 1 self-check (incl. tests now GREEN)
```
$ npx vitest run
 FAIL  tests/connections-page.test.ts > Connections list > hides Add connection, Edit and Remove when the descriptor reports the user member can manage
 FAIL  tests/connections-page.test.ts > Connections empty states > a member with nothing connected sees the generic empty state with the member copy and no Add connection
 Test Files  1 failed | 50 passed (51)
      Tests  2 failed | 546 passed (548)
```
All 87 tests T wrote or updated for #141 now pass: gitlab-descriptor, gitlab-pool-schema, gitlab-connections, gitlab-pool-forms, webhook-delivery-platform, platform-neutral-source, platform-registry and runner-row-platform. Every other pre-existing test passes except the two above. They are a test-stub issue (see "Tests that look wrong").
```
$ npm run build            # vite build + tsc --noEmit
✓ built in 1.58s
> tsc --noEmit             # exit 0
$ npm run lint             # eslint src tests … --max-warnings=0, exit 0
$ CARGO_BUILD_JOBS=2 RUST_TEST_THREADS=2 cargo test -p gridops-api the_runners_list_carries
test result: ok. 1 passed; 0 failed
$ CARGO_BUILD_JOBS=2 RUST_TEST_THREADS=2 cargo test -p gridops-api gitlab
test result: ok. 34 passed; 0 failed   |   test result: ok. 3 passed; 0 failed
$ CARGO_BUILD_JOBS=2 RUST_TEST_THREADS=2 cargo test -p gridops-api runner
test result: ok. 11 passed   |   ok. 2 passed   |   ok. 19 passed (fleet_http, incl. the runners route)
$ cargo fmt --check -p gridops-api                                        # exit 0
$ cargo clippy -p gridops-api -- -D warnings -A clippy::unused_async_trait_impl   # no findings
```
I did not start a server. Tier 2 (the headless walkthrough) is T's Phase 6.

## Evidence appendix
docs/handoffs/141/evidence.md

## Tests that look wrong (for T)
- **The `tests/connections-page.test.ts` stub at `:83` always serves `{ canManage: true, items: [] }` for GitLab.** Two member tests fail because of it:
  - "hides Add connection, Edit and Remove when the descriptor reports the user member can manage" (`:189-199`);
  - "a member with nothing connected sees the generic empty state with the member copy and no Add connection" (`:455-462`).

  Add connection is offered for any platform whose list reports `canManage` (`connections-section.tsx:70-73`, unchanged #145 logic). So the stub tells the page that the member can manage GitLab, and the page shows Add connection and the admin empty-state copy. Production is correct: the server reports `canManage` per user.

  **Fix (test-only):** have the GitLab stub follow the Bitbucket list's permission, e.g. `Response.json({ canManage: bitbucketList.canManage, items: [] })`. Its role is the per-role `canManage` already set in each test.
- Separately, `tests/targets-row-forward-compat.test.ts` needed no change: the new `PlatformOptionsSection` `values` and `onChange` props are optional.

## Known issues
- **Polling errors (#148)** are still not listed. No API exposes them. This was out of reach per the brief; nothing was built.
- **#159 is still open.** The pool list destination, the pool detail Account row, and the runner row's Repository column (A round 3, warn 2) still show a GitLab pool or runner under its installation's GitHub login. That is tracked in #159.
- **No installation.** A GitLab-only create on an account with no GitHub App installation sends `installationId: 0`. The create schema's message for `installationId` is shown verbatim (brief and wireframe F4). There is no new copy.

## Files changed
- crates/gridops-api/src/resources.rs
- src/features/operations/operations.functions.ts
- src/features/platform-connections/connections-section.tsx
- src/features/platform-connections/github.tsx
- src/features/platform-connections/gitlab.tsx (new)
- src/features/platform-connections/platform-connections.functions.ts
- src/features/platform-connections/registry.tsx
- src/features/platform-connections/types.ts
- src/features/runner-pools/platform-options-section.tsx
- src/features/runner-pools/runner-pools.functions.ts
- src/features/runner-pools/schemas.ts
- src/features/runner-pools/targets.ts
- src/routes/_app/runner-pools.$poolId.settings.tsx
- src/routes/_app/runner-pools.new.tsx
- src/routes/_app/webhooks.tsx

Docs (not production): docs/handoffs/141/handoff-F.md, docs/handoffs/141/evidence.md, docs/handoffs/141/decisions.md (F entry appended).

---

# Handoff-F: Rework after S (Spec Audit REWORK, cycle 1) - clipped Edit on narrow GitLab rows

**Date:** 2026-11-20
**Branch:** claude/gitlab-141-ui-3ae7f5
**Issue:** #141

## What was done
- `src/features/platform-connections/connections-section.tsx` (`ConnectionRow`, action cluster at :267-269).
  - The cluster's classes change from `flex shrink-0 items-center gap-1` to `flex min-w-0 flex-wrap items-center justify-end gap-1`, and a two-line comment explains why.
  - Below `sm`, the cluster now wraps, right-aligned, inside the full-width actions line. Before, it overflowed off the left edge and the card's `overflow-hidden` clipped it.
  - No GitLab string is added; the fix is generic for any descriptor with several row actions.

## Design decisions
- **Wrap, not shrink or a menu.** Wrapping right-aligned is the fix S suggested. It keeps every action a visible ghost button with the same labels, order and hit target.
  - An overflow "More" menu was rejected: it would change the row's interaction model and the wireframe.
  - Shortening the labels was rejected: it would need GitLab-specific copy.
- **`min-w-0`** lets the cluster shrink below its one-line width inside the `w-full` actions line, so the wrap happens instead of overflowing.
- **`sm` and up is untouched by construction.** There the parent is `sm:w-auto sm:shrink-0`, so it takes the cluster's one-line width and the cluster never needs to wrap. The pixel diff below confirms this.

## Reuse / extend-vs-new
This extends the existing shared `ConnectionRow`. No new object.

## Architecture notes for A
None. This is a class change in one shared component: no new module, interface or dependency.

## Deviations from spec / wireframe
None. At widths where the actions don't fit on one line, they now take a second line, right-aligned, under the row. This is how the row already stacks its actions below `sm`.

## Verification (headless; no window opened)
I served the worktree with `vite dev` on port 3150, then stopped it. I measured with S's scratchpad Playwright 1.63 tooling (`measure.mjs`, `mock.mjs`, with the GitLab mock). For each of Edit, Verify, Rotate webhook secret and Remove I recorded the left and right edges and ran `elementFromPoint` at the button's centre.

The table shows the "Mobile app" row; the "Platform group" row gives identical numbers. The card starts at x=20.

| Width | Edit L-R | Verify L-R | Rotate R | Remove L-R | All hit-testable | Page overflow |
|---|---|---|---|---|---|---|
| 320 | 179-223 | 227-283 | 211 | 215-283 | yes | no |
| 360 | 61-105 | 109-165 | 323 | 255-323 | yes | no |
| 390 | 91-135 | 139-195 | 353 | 285-353 | yes | no |
| 414 | 43-87 | 91-147 | 305 | 309-377 | yes (one line) | no |
| 480 / 560 / 639 / 640 | in card | in card | in card | in card | yes | no |
| 768 | 361-405 | 409-465 | 623 | 627-695 | yes | no |
| 1280 | 735-779 | 783-839 | 997 | 1001-1069 | yes | no |

- **300 px (below the acceptance range).** The page overflows because the app shell has a 320 px minimum width. Shell elements end at x=320 with or without GitLab rows. This predates #141 and is not caused by this row.
- **Before/after pixel diff of `/settings/connections`.** I compared against S's pre-fix HEAD captures in `screenshots/cycle-1/`, with `compare -fuzz 10%` and the 24 px version bar excluded (it shows the commit hash):

  | Surface | 360 | 768 | 1280 |
  |---|---|---|---|
  | GitHub + Bitbucket rows | 1 px | 3 px | 3 px |
  | GitLab rows | changed (intended: actions wrap to a second line) | 5 px | 4 px |

  - The 1-5 px counts are anti-aliasing inside one 10x10 icon box, so all of those renders are unchanged.
  - The GitLab 360 crop shows Edit, Verify and Rotate webhook secret on the first line and Remove right-aligned on the second line, all inside the card. The crop is at `/private/tmp/claude-501/-Users-andreangelantoni-Projects-gridops--claude-worktrees-gitlab-141-ui-3ae7f5/026565be-3ca5-4937-951c-4703a5817ecc/scratchpad/shots/f-fix/gitlab-row-360-crop.png`.

## Tier 1 self-check (incl. tests now GREEN)
```
$ npx tsc --noEmit            # exit 0 (TSC_OK)
$ npm run lint                # eslint --max-warnings=0, clean
$ npx vitest run
 Test Files  51 passed (51)
      Tests  548 passed (548)   # includes tests/platform-neutral-source.test.ts
```
No backend file was touched, so cargo was not re-run.

## Evidence appendix
None: no behavior outside the diff is relied on.

## Tests that look wrong (for T)
None. No test change is needed: layout does not exist under jsdom.

## Known issues
- **Not fixed: S's cosmetic note.** At 768 and 1280 px the Project/Group badge wraps to its own line and the path is truncated. S marked this non-blocking and its acceptance requires 768/1280 to stay unchanged. Fixing it would mean narrowing the action cluster at `sm`+, which changes the row on every platform. It needs its own design call and is out of this rework's scope.
- **For U, per S:** re-walk 360 px using per-button geometry, not page-level overflow.

## Files changed
- src/features/platform-connections/connections-section.tsx

Docs (not production): docs/handoffs/141/handoff-F.md (this section), docs/handoffs/141/decisions.md (entry appended).
