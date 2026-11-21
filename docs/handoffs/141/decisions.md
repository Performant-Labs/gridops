# Decision journal - #141 GitLab in the shared connections and targets UI

## D (Design) - 2026-11-20
- **Decided:** Generated a low-fi HTML wireframe (`docs/handoffs/141/wireframe.html`, frames A-I) that adds no page, nav item or pool-form row: GitLab content drawn into the existing Connections list, Add connection dialog, one-time reveal, Targets listbox, Platform options, runner details panel and deliveries list. Descriptor-contract gaps recorded (frame I): (1) generic per-row action list with optional `confirmLabel` for Verify and Rotate webhook secret; (2) generic per-platform option values channel for `run_untagged`; (3) reveal needs NO change (`SecretRevealDialog.details` is a ReactNode and `CopyButton` can sit in it: secret in the main copy box, webhook URL + Job events steps in `details`); (4) NEW, found by design: a `select` field type for the create form's Project/Group choice, because the API needs `scopeKind` and `ConnectionField` only has `text` and `secret`; (5) deliveries platform derived from the id prefix against the registry in `webhooks.tsx`; (6) polling errors out of scope (#148 logs only). Verify, Rotate and Remove failures are toasts; Add and Edit keep inline callouts; server messages verbatim.
- **Assumed:** (1) The GitLab list API carries no verification status, so Verify's outcome lives only in a toast and the "Verified {time}" caption is `updatedAt`, as for Bitbucket. (2) A GitLab row with no repository shows the platform badge and no destination text; GitHub rows gain a GitHub badge so the column is consistent; the "GitHub App" fallback text stays for GitHub only. (3) The no-installation submit failure shows the existing "Could not create the pool" callout with the validator's or server's message verbatim; no new copy. (4) GitLab adds no Provider requirement and no priority or manual entry. (5) The lucide icon `Gitlab` exists in the installed version (not checked: `node_modules` is absent in this worktree).
- **Hedged:** All new copy (help text, reveal wording, confirm, plan sentence, tag errors) is a proposal. The comma tag rule cannot be triggered from the comma-split Additional labels field; the wireframe draws the 255-character and empty-tag errors and keeps the comma rule as a unit-tested validator defence, so the brief's walkthrough item "tag validation error on a comma" cannot be performed as worded. The wireframe's pool-wire fields (`runnerGroupId: 1`, `installationId`) are the brief's mocked spec, not verified against #138. Dark theme and narrow widths were not drawn.
- **Evidence:** Read `docs/handoffs/141-brief.md` and its round-1 review; `src/features/platform-connections/types.ts`, `bitbucket.tsx`, `connections-section.tsx` (row cluster, edit form, `FieldInput`), `src/components/secret-reveal-dialog.tsx`, `src/components/confirm-dialog.tsx`, `src/features/runner-pools/platform-options-section.tsx`, `targets.ts` (`unavailableReason`), `schemas.ts` (`installationId`, `bitbucketLabelError`), `src/features/runners/runner-row.tsx`, `src/routes/_app/webhooks.tsx`, and the #145 wireframe and handoff-D for the visual language. No survey.md exists for this run. Rendered the file with headless Chromium (chrome-headless-shell, no window) and looked at the screenshots of frames A-F; frames G, H and I were not looked at, and the GitLab badge glyph here is Unicode, not the real lucide icon.

## A (Architecture review, up front) - 2026-11-20T17:03:34-08:00
- **Decided:** BLOCK, with 2 block findings and 7 warns (docs/handoffs/141/handoff-A.md).
  - Block 1 [completeness]: `github.tsx` treats every pool that is not a repository pool as an organization pool (`initialSelection` :244-255, `readOnlySummary` :257, `toUpdatePayload` :225-243, `initialPriorityKeys` :256). A GitLab pool's settings form would get a locked GitHub runner-group chip that should not be there. Through the GitLab kinds' `excludes`, that chip disables every GitLab option. `github.tsx` is not on the brief's allowlist. Fix: a named generic gap (explicit two-scope branches, empty values for any other scope) and the file added to the allowlist.
  - Block 2 [correctness]: the GitLab pool wire fields must go into `schemas.ts` and `runner-pools.functions.ts`. The zod parse strips unknown keys and the scope enum rejects "gitlab". Bitbucket's precedent and #145 survey statement 1 both put the fields there. The brief bans GitLab strings in `schemas.ts` beyond the tag rule and does not list `runner-pools.functions.ts`. Fix: sanction the Bitbucket pattern explicitly and reword Done-when.
  - Warns:
    - `ConnectionField` needs select, default and validate (wireframe B2/B3), with shapes taken from existing patterns; `labelFor`/`helpFor` is over-abstraction.
    - The `buildCreatePayload` registry order matters.
    - Platform option values should go inside the `TargetUpdateValues` object, not a new argument.
    - Row-action reveals must go through the one `SecretRevealDialog`.
    - The delivery platform lookup belongs in `registry.tsx`, with no platform checks in `webhooks.tsx`.
    - The 255-character tag rule is dead code under the shared 64-character cap.
    - The pool list and detail views have the same two-scope assumption (out of scope, follow-up).
- **Assumed:** The #145 survey's forward-compat table plus the brief stand in for a #141 survey Reuse map, which does not exist. The workflow doc numbers this gate Phase 4. #138's branch `claude/gitlab-138-api-44d3a8` is the pool wire F will build against. The wireframe is treated as the design under approval (its approval line is not filled in).
- **Hedged:** Block 2 could be read as a wording problem in the brief. It is kept as a block because F would otherwise have to break a hard rule or invent a validation bypass or an import cycle, and the coexistence rule tells F to stop when a required file is out of scope. Finding 8 (tag length) is a warn because the shared cap already makes it harmless at submit; it is a spec decision for O and S.
- **Evidence:** Read the brief, 141-brief-result-r1.md, handoff-D.md, decisions.md and the wireframe text (frames A-I).
  - Read in full: types.ts, registry.tsx, bitbucket.tsx, github.tsx, connections-section.tsx, platform-connections.functions.ts, secret-reveal-dialog.tsx, use-action.ts, platform-options-section.tsx, targets.ts, schemas.ts, runner-pools.functions.ts and tests/platform-neutral-source.test.ts.
  - Read in part: runner-pools.new.tsx (1-260), runner-pools.$poolId.settings.tsx (1-140), confirm-dialog.tsx, copy-button.tsx, targets-row.tsx (grep), webhooks.tsx (grep), runner-row.tsx (grep), operations.functions.ts (Runner type), docs/handoffs/145/survey.md and 145/handoff-A.md.
  - Checked the server label cap in crates/gridops-core/src/models.rs on master (:401-409) and on #138's branch (:430-435).
  - Confirmed `git diff c575130 HEAD -- src tests` is empty, and that no other branch changes src/ or tests/ against origin/master.

## D (Design, revision 2) - 2026-11-20
- **Decided:** Revised `docs/handoffs/141/wireframe.html` to the amended brief (A round 1, BLOCK). Path label is static ("Project or group path"); select, default and validate map to gap 5a-c; tag cap is the shared 64 (F3 redrawn to what can appear: the schema message at submit); frame I renumbered to the brief's gaps 1-7 plus two out-of-reach rows (polling errors, #159); E5 notes no locked GitHub chip (gap 4); C/H notes use `SecretReveal`/`onReveal` and `platformForDeliveryId`; F4 body includes `repositoryIds: []` and the registry-order note. Still no new page, nav item or pool-form row.
- **Assumed:** (1) A 65+ character tag error appears at submit in the existing "Could not create the pool" callout with the shared schema's message, not live under the field, because `gitlabLabelError` checks only non-empty and no comma. (2) Empty and comma tag errors cannot be triggered from the comma-split field (`runner-pools.new.tsx:169` trims and drops empties), so they are Vitest-only. (3) All other revision-1 assumptions stand (Verify outcome is toast-only; GitLab delivery rows show no destination text).
- **Hedged:** Brief walkthrough item "tag validation error on a comma" is unreachable as worded; the walkthrough should use the 64-character error instead (open question 1 in handoff-D). The real lucide `Gitlab` icon was not checked (no `node_modules`). New copy remains a proposal.
- **Evidence:** Read the amended brief, handoff-A.md, decisions.md and the full revision-1 wireframe; checked `schemas.ts:13` (64 cap), `runner-pools.new.tsx:139,169-171,250` (label split and error line). Rendered the file headlessly (chrome-headless-shell) and looked at the screenshots of frames B, C, E5, F, G, H, I.

## A (Architecture review, up front, round 2) - 2026-11-20T17:17:23-08:00
- **Decided:** BLOCK, with 1 block finding and 5 warns (docs/handoffs/141/handoff-A.md, rewritten for round 2).
  - **Round-1 blocks resolved:**
    - Block 1: gap 4, with `github.tsx` on the allowlist.
    - Block 2: the Bitbucket-pattern wire fields in `schemas.ts`, `runner-pools.functions.ts` and `operations.functions.ts`.
    - All seven round-1 warns are taken up as gaps 1-3 and 5-7, the 64-character tag cap, and #159.
  - **New block [correctness]:** on the pool settings form, the GitLab target options come from data that holds only the pool's own connections.
    - The brief and wireframe E5 take the options from "the already-loaded `gitlabConnections`".
    - On settings, the context is `{ form: "update", pool }`. The pool detail's `gitlabConnections` is a membership join (`pool_gitlab_connections`, origin/master `resources/gitlab.rs:505`).
    - So no second connection can ever be added on settings, which contradicts the brief's own walkthrough step.
    - Fix, inside `gitlab.tsx` only:
      - `offered`: create, or `pool.scope === "gitlab"`.
      - `available` on update: `pool.scope === "gitlab"`.
      - `load` on update: fetches every connection with `getGitlabConnections(signal)`, through the existing lazy-load seam (as `repositoryKind.load` does). Option keys match `initialSelection`.
      - No change to `TargetContext` or the settings route.
  - **Warns:**
    - Split the GitLab refinement into a create-only scope rule and a shared bounds rule, as #138 does. The update schema has no `scope`.
    - Render the select field with `SearchableSelect`.
    - Model the `webhooks.tsx` neutral-source check on the runner-row one, because of the existing "GitHub App" empty-state copy. Reuse `PlatformBadge`, not a third badge.
    - Wireframe B note 4 (no create toast) and H2 (a new Platform row in the detail sheet) are outside the gap list.
    - #138 has merged to origin/master (0086aa9), so the wire is authoritative there.
- **Assumed:**
  - The #145 survey and the brief stand in for a #141 Reuse map, which does not exist.
  - The GitLab list API (GET `/api/v1/platform-connections/gitlab`) is readable by any signed-in user. The settings editor only renders for `pool.canManage`.
  - The workflow doc numbers this gate Phase 4.
- **Hedged:**
  - Finding 1 is a block rather than a warn. The brief states the wrong option source twice (the Contract section and wireframe E5), and T would pin it in tests before F could notice. The fix touches no shared file.
  - The H2 detail-sheet row is O's spec call, so it is a warn.
- **Evidence:**
  - Read the amended brief, 141-brief-result-r1.md, handoff-D.md (revision 2), decisions.md, the round-1 handoff-A.md, and the wireframe text (frames B, E5, F, H, I).
  - Read in full: `types.ts`, `registry.tsx`, `bitbucket.tsx`, `github.tsx`, `platform-connections.functions.ts`, `connections-section.tsx`, `schemas.ts`, `runner-pools.functions.ts`, `targets.ts`, `platform-options-section.tsx`, `use-action.ts`, `tests/platform-neutral-source.test.ts`.
  - Read in part: `runner-pools.$poolId.settings.tsx` (1-200), `runner-pools.new.tsx` (36-140), `targets-row.tsx` (1-140), `webhooks.tsx`, `runner-row.tsx` (78-112), `secret-reveal-dialog.tsx`, `confirm-dialog.tsx`, and the `Runner` and `WebhookDelivery` types.
  - Checked on origin/master (#138 merged, 0086aa9):
    - `models.rs:528-570` (`validate_gitlab_targets`, `validate_gitlab_connections`).
    - `resources.rs:1468, :1523, :2495-2499`.
    - `resources/gitlab.rs:505-533`.
    - The OpenAPI GitLab paths.
  - Coexistence: no live branch changes the #141 shared files. The two that touch the pool forms date from 2026-11-03, before #145.

## D (Design, revision 3) - 2026-11-20
- **Decided:** Revised `docs/handoffs/141/wireframe.html` to the brief amended in 9b9f6ff (A round 2, BLOCK).
  - E5 now shows a GitLab pool's settings with the listbox open and an unused connection ("Docs site") offered, because on settings the options are fetched with `getGitlabConnections` (the pool detail holds only the pool's own connections). E5b draws the loading and per-kind error/Retry states; E6 shows a GitHub or Bitbucket pool's settings with no GitLab group. GitHub and Bitbucket options do not appear at all on a GitLab pool (corrects revision 2's "disabled with a reason").
  - H2's extra Platform row is dropped; the `PlatformBadge` sits beside the sheet heading. B note 4: the create toast stays. B/I: the select is `SearchableSelect`. G/I: the runner link's three fields come from the narrow runners-list backend edit. I gains two rows (runner JSON edit; settings option source) and gap 7 names the runner-row-style neutral-source assertion.
- **Assumed:** (1) The GitLab connection list is readable by any signed-in user and the settings editor renders only for managers (as in A round 2). (2) The empty-fetch status on settings is the shared "No {kind} options" text; only its wording comes from the existing picker, so it is not drawn as final copy. (3) The create toast copy is the shared default `{name} connection added.` ("GitLab connection added."). (4) Revision 1-2 assumptions stand (toast-only Verify outcome; GitLab delivery rows with no destination text).
- **Hedged:** The brief's walkthrough item "tag validation error on a comma" is still unreachable through the comma-split labels field (open question 1). The real lucide `Gitlab` icon was not rendered (no `node_modules`); badges use Unicode stand-ins. New copy remains a proposal. Dark theme and narrow widths were not drawn.
- **Evidence:** Read the amended brief, handoff-A.md (round 2), decisions.md, handoff-D.md and the full wireframe frames B, E, G, H, I; checked `targets-row.tsx:100-140` (load, loading, error/Retry and `No {kind} options` states, and `offered` handling), `webhooks.tsx:132-158` (the detail sheet's rows and heading), `connections-section.tsx:325-345` (the create toast). Rendered headlessly (chrome-headless-shell, no window) and looked at frames E (E4-E6) and H.

## A (Architecture review, up front, round 3) - 2026-11-20T17:30:05-08:00
- **Decided:** PASS, with 0 block findings and 3 warns (docs/handoffs/141/handoff-A.md, rewritten for round 3).
  - **Round-2 block resolved.** The brief's Target kinds section now has:
    - `offered`: create, or a GitLab pool.
    - `available`: create, the kind's scope has a connection; update, a GitLab pool.
    - `load`: create, from the pool options; update, `getGitlabConnections(signal)`, through the lazy-load seam `github.tsx:93-105` uses.
    - No `TargetContext` or route change.
    - Wireframe E5, E5b and E6 match.
  - **All five round-2 warns are taken up:** the split refinement, `SearchableSelect`, `PlatformBadge` with the runner-row-style assertion, the create toast with no H2 row, and the master baseline.
  - **New backend edit (the runners list JSON) is consistent.**
    - The list already joins display-only fields (`repository`, `accountLogin`, `machine`).
    - `/api/v1/runners` is not in the OpenAPI.
    - The GitHub descriptor builds its link from raw fields too.
    - No parallel worktree touches that function.
  - **Warns:**
    1. Name `resources/gitlab_tests.rs` as the test home. No runners-list test exists, so "or the existing runners-list test" invites a second harness.
    2. `runner-row.tsx:46, :109, :127` falls back to `accountLogin`, so a GitLab runner shows its installation's GitHub login. This is a #159 sibling that #159 does not list. Add it to #159, and do not fix it here with a platform branch.
    3. The Hard-limits line puts `registry.tsx` under "no gitlab string", while gap 6 must register GitLab there. Allow the import and the array entry. Fall back to `githubPlatform` by reference.
- **Assumed:**
  - The #145 survey and the brief stand in for a #141 Reuse map, which does not exist.
  - The workflow doc numbers this gate Phase 4.
  - The wireframe is treated as the design under approval; its approval line is not filled in.
- **Hedged:**
  - Finding 2 is a warn, not a block. The brief's hard limits and the neutral-source test already stop the drifting fixes. The remaining question is where to track the gap, which is O's call.
  - D's open question 1 (the comma tag error cannot be reached from the comma-split field) is a spec/walkthrough matter for O and S, not architecture, so A does not rule on it.
- **Evidence:**
  - Read the amended brief and its diff from round 2 (0d01dfa..9b9f6ff), 141-brief-result-r1.md, handoff-D.md (revision 3), decisions.md, the round-2 handoff-A.md, and the wireframe text (frames B, E5-E6, G, H, I).
  - Read: `resources.rs:1916-1998` (runners list), `resources/gitlab.rs:80-101` (the list handler takes any `AuthUser`), `resources/gitlab_tests.rs:1-80, :241-306`, `migrations/0047_gitlab.sql:1-80`, `types.ts`, `registry.tsx`, `platform-connections.functions.ts`, `github.tsx:80-130, :205-207`, `runner-row.tsx` (grep), `webhooks.tsx` (grep), `operations.functions.ts:40-60`, `tests/platform-neutral-source.test.ts`.
  - Checked the OpenAPI paths for runners: only runner-pool paths exist.
  - Checked `token_access.rs:1072`, the only test that touches `/api/v1/runners`.
  - Checked `gh issue view 159`: it covers the pool list and the pool detail only.
  - Coexistence: ran `git diff origin/master...<branch>` for every GitLab branch and `git status` in the #138-followup, #139, #140, #142, #147 and #148 worktrees. #140's brief agrees on the `gitlab:<connection_id>:<key>` id.
  - Confirmed `git diff 0086aa9 HEAD -- src tests crates` is empty.

## T (Tester, Phase 4 RED) - 2026-11-20T17:52:48-08:00
- **Decided:** RED is valid; F may implement. Handoff: docs/handoffs/141/handoff-T-red.md.
  - **New tests:**
    - Vitest: 5 new files and 1 fixture, 85 tests.
      - `gitlab-descriptor` (52, unit);
      - `gitlab-pool-schema` (13, unit);
      - `gitlab-connections` (8, jsdom);
      - `gitlab-pool-forms` (4, jsdom);
      - `webhook-delivery-platform` (4, jsdom);
      - neutral-source: 3 new checks.
    - Rust: 1 API test in `resources/gitlab_tests.rs` (A warn 1), for the runners list's `gitlabRunnerId` / `gitlabBaseUrl` / `gitlabScopePath`. For a GitHub runner the three keys must be present and null.
  - **Updated existing tests** (they would break the moment GitLab is registered):
    - the registry order is now `github, bitbucket, gitlab`;
    - the unknown-platform fallback moved from `gitlab` to `azure-devops`;
    - the type test is renamed to `SecretReveal`;
    - the runner-row badge expects "GitLab";
    - the stubs in the connections-page test and in `poolFormFetch` serve an empty GitLab list.
  - **Full suite:** 87 failed (all of them new or updated tests) and 459 passed (every pre-existing test). The Rust test fails on the missing `gitlabRunnerId` key.
- **Assumed:**
  - The wireframe's exact copy (revision 3) is the approved UI contract for labels: the field labels, `Verify`, `Rotate webhook secret`, `Run untagged jobs` and "Job events". Its approval line is still empty.
  - Option `connectionId` or `nativeId` is the connection UUID, following the Bitbucket analogue.
  - The new-pool form's create values reach `buildCreatePayload` as a fourth `{ platformOptions }` argument (brief gap 2).
- **Hedged:**
  - For settings, only a *changed* `runUntagged` is pinned. The wireframe says "sent only when different" and the brief is silent.
  - The GitLab platform-option value shape, the row-action type and `platformForDeliveryId`'s name are not pinned.
  - The tag rule's comma and empty checks are tested at unit level only, because they cannot be reached from the field (D open question 1).
  - No satisfiability sketch was built. The DOM contracts were checked against the existing components instead.
  - The `targets-row-forward-compat` harness may need a test-only update if F makes the `PlatformOptionsSection` value props required.
- **Evidence:**
  - Baseline: 46 files and 461 tests green after `npm ci`.
  - Read the brief, handoff-A (round 3), handoff-D, the wireframe text, and these files in full or in the parts the tests touch: `types.ts`, `registry.tsx`, `github.tsx`, `bitbucket.tsx`, `platform-connections.functions.ts`, `connections-section.tsx`, `targets.ts`, `schemas.ts`, `runner-pools.functions.ts`, `platform-options-section.tsx`, `targets-row.tsx` (load on mount), `webhooks.tsx`, `runner-row.tsx`, `use-action.ts`, `confirm-dialog.tsx` (the `window.confirm` fallback), `secret-reveal-dialog.tsx`, `copy-button.tsx`, the `searchable-select` trigger ARIA, `settings-ui` `Switch` (Radix), the OpenAPI GitLab schemas, `resources.rs` `runners` (:1916-1998) and migration 0047.
  - Ran: `npx vitest run`, `cargo test -p gridops-api the_runners_list_carries`, `rustfmt --check`, `eslint tests`, `tsc --noEmit` (test files) and `cargo clippy -p gridops-api --all-targets`. Clippy found nothing in the new test; its pre-existing lints in other files are outside #141.

## F (Feature Implementor, Phase 5) - 2026-11-20T18:06:03-08:00
- **Decided:** Implemented against T's RED. Handoff: docs/handoffs/141/handoff-F.md. The GitLab descriptor is `gitlab.tsx`, registered last. Each descriptor-contract gap is closed generically, with no GitLab string in a shared file:
  - **Gap 1 (row actions):** `ConnectionBindings.actions?(item)` returns `ConnectionRowAction[]` (`label`, `run(item)`, `confirm?`, `confirmOptions?`, `success?`). The row runs each action through `useAction`, between Edit and Remove. The row's buttons are disabled while an action or Remove runs. A returned `SecretReveal` (renamed from `PostCreateReveal`) reaches the section's one `SecretRevealDialog` through `onReveal`.
  - **Gap 2 (option values):** `platformOptions.initialValues?(pool?)` and `Component: ComponentType<{ values, onChange }>`. `initialPlatformOptionValues` in `targets.ts` seeds the forms' `Record<platformId, unknown>`. The values travel as `platformOptions` inside `TargetUpdateValues` and inside a new optional fourth `TargetCreateValues` argument to `toCreatePayload` / `buildCreatePayload`. Both routes hold the record and pass it on. Neither route names a platform.
  - **Gap 3 (reveal):** no component change. The secret is the main copy box. The webhook URL, its own `CopyButton` and the Job events steps go in `details`.
  - **Gap 4 (`github.tsx`):** `toUpdatePayload`, `initialSelection` and `readOnlySummary` branch on `repository` and `organization` explicitly, and return `{}` / `{}` / `null` for any other scope. `initialPriorityKeys` already returned `[]`.
  - **Gap 5 (fields):** `ConnectionField` gains `type: "select"`, `options`, `defaultValue` and `validate`. The select is the shared `SearchableSelect` (`searchable={false}`, `ariaLabel` = the field label). `defaultValue` seeds the create values when a platform is chosen. A `validate` problem shows `aria-invalid` and a `text-danger` line once the value is non-empty (or after a refused submit), and blocks the request.
  - **Gap 6 (order):** GitLab is registered last, with a comment that order matters.
  - **Gap 7 (deliveries):** `platformForDeliveryId(id)` in `registry.tsx` falls back to `githubPlatform` by reference. The new descriptor value `deliveryDestinationFallback` is "GitHub App" for GitHub only. `webhooks.tsx` shows `PlatformBadge` in the row and beside the sheet heading. The sheet's Destination shows "—" when there is no destination. The list shows no destination text.
  - **Backend:** the runners list selects `r.gitlab_runner_id` and LEFT JOINs `gitlab_connections gc` for `gitlab_base_url` and `gitlab_scope_path`, emitted as `gitlabRunnerId`, `gitlabBaseUrl` and `gitlabScopePath` (null otherwise).
- **Assumed:**
  - **runUntagged on settings:** settings sends `runUntagged` only when it differs from the pool's stored value (wireframe F1). A form that holds no GitLab option value keeps the stored one, so a caller without `platformOptions` never flips it.
  - **Create schema rules:** the create schema rejects GitLab ids on a non-GitLab scope, a GitLab scope with no ids, and GitLab with repositories or Bitbucket workspaces. Both schemas check unique ids, at most `maxCount`, and the GitLab tag rule when ids are present (Bitbucket pattern).
  - **Target kinds:** the GitLab kinds `raisesMinMaxCount`, because each connection registers its own runner and the server caps ids at `maxCount`.
- **Hedged:**
  - The plan sentence and `notConnected` copy differ from the wireframe; see the handoff's Deviations.
  - Two pre-existing `tests/connections-page.test.ts` member tests fail. T's GitLab stub (`:83`) always reports `canManage: true`, so a member is offered Add connection. That is a test-only rework for T. Production follows the server's `canManage`.
  - The headless Playwright walkthrough is T's Tier 2 job and was not run here.
- **Evidence:**
  - `npx vitest run`: 546 passed, 2 failed (the two stub tests above).
  - `npm run build`: vite build plus `tsc --noEmit`, exit 0.
  - `npm run lint`: exit 0.
  - `cargo test -p gridops-api the_runners_list_carries`: passed.
  - `cargo test -p gridops-api gitlab`: 34 + 3 passed.
  - `cargo test -p gridops-api runner`: 11 + 2 + 19 passed.
  - `cargo fmt --check -p gridops-api`: exit 0.
  - `cargo clippy -p gridops-api -- -D warnings` (with `-A clippy::unused_async_trait_impl`): clean.
  - Source facts are in docs/handoffs/141/evidence.md.

## T (Tester, Phase 7 GREEN) - 2026-11-20
- **Decided:**
  - Repaired the wrong test stub myself (test-only): in `tests/connections-page.test.ts` the GitLab list stub now follows `bitbucketList.canManage`. Production was correct.
  - Verdict PASS: no production change is needed.
- **Assumed:** the `release_preflight` failure in `cargo test -p gridops-api` is environmental (the test needs a host with no macOS ARM64 machine, and this host is one). The change touches nothing it reads.
- **Hedged:** no live-browser check was run (headless-first, and U owns it). jsdom tests cannot see real client init, so U should walk the connection form, the reveal, both pool forms and the deliveries page.
- **Evidence:**
  - `npx vitest run`: 51 files, 548 tests, all passed.
  - `npm run build`, `npm run lint` and `npx tsc --noEmit`: exit 0.
  - `cargo test -p gridops-api gitlab` (34 + 3) and `runner` (11 + 2 + 19): passed.
  - `cargo fmt --check -p gridops-api`: exit 0.

## A phase 7 (anti-duplication gate), 2026-11-20
PASS. GitLab extends the existing descriptor, registry, connections section, targets and webhooks objects; no parallel path. One warn: hardcoded `OTHER_PLATFORM_KINDS` in `gitlab.tsx`, consistent with the Bitbucket pattern.

## U (UI Walkthrough, Playwright backend) - 2026-11-20
- **Decided:** PASS. Drove the real `vite dev` server (port 3000) in a headless Chromium (Playwright
  1.63.0 / Chromium 153.0.8010.12), with every `/api/v1/*` call intercepted and mocked to match the
  brief's Contract section (no DB/seed script exists for this project, and GridOps' only login is
  GitHub OAuth, so a mocked `/api/v1/auth/session` plus a mocked API surface was the only way to
  reach the UI headlessly without a real GitHub App + DB). All 24 scripted checks passed: connection
  add (success + three error states: bad token, missing scope, duplicate), Verify (success and
  failure), Rotate webhook secret (confirm + one-time reveal), Edit (name + token), Remove (success
  on an unreferenced connection, 409 on a referenced one — re-checked in an isolated follow-up pass
  after the first pass's screenshot of this step was inconclusive, see Hedged), the Targets picker
  (GitLab project + group, exclusion against GitHub/Bitbucket), Platform options (Run untagged
  jobs), pool create (POST body: `scope: "gitlab"`, both connection ids, `runUntagged: true`), pool
  settings (fetches connections beyond the pool's own via `getGitlabConnections`, PUT body carries
  the added id), the 64-char tag cap error, the GitLab runner's detail-sheet link
  (`https://gitlab.com/acme/platform/-/runners/555`), the delivery badge and sheet "—" destination,
  and 360px usability on Connections and the new-pool Targets control. Screenshots and the full
  per-step JSON are in `/tmp/u141-shots/` and `/tmp/u141-results.json` (session-scratch, not
  committed).
- **Assumed:** the mocked API responses are a faithful stand-in for #137/#138's real contract (the
  brief's own instruction: "headless, against mocked API responses that match the spec"). Real
  server-side behavior (actual GitLab API verification, real webhook delivery) is T/S's and the
  backend's concern, not U's.
- **Hedged:**
  - **Test-script flaw caught and fixed, not a #141 defect:** the first pass's Remove check used
    `page.getByRole('button', { name: 'Remove' }).last()` to click the confirm dialog's own button,
    which is ambiguous against the list's own per-row "Remove" buttons (same accessible name); the
    resulting screenshot showed the confirm dialog still open 500ms later, so that result was
    reported honestly as inconclusive rather than claimed as a pass. A follow-up pass scoped the
    click to `page.getByRole('dialog')` and confirmed both outcomes cleanly (`13a`/`13b`/`13c`
    screenshots): the unreferenced connection's row disappears after a 204, and the referenced
    connection's row stays with the 409 "A runner pool still uses this connection…" toast.
  - **Environment-only finding, not a #141 defect:** this session's bundled Chromium
    (153.0.8010.12, from the Playwright 1.63.0 install used to drive the walkthrough) throws
    `SyntaxError: Invalid regular expression … /v: Invalid character class` on the pool-name
    input's `pattern="[a-z0-9][a-z0-9-]*[a-z0-9]"` (present on both pool routes, unchanged by this
    diff, pre-dating #141) and silently blocks every pool-form submit. Worked around in the test by
    stripping the `pattern` attribute before submitting; not a code change, and not something F
    should fix under this issue. Flagging for the operator since a sufficiently new Chromium in a
    real user's browser could hit the same thing; out of #141's scope to fix.
  - `GET /api/v1/runner-pools/repositories` (an existing, unmocked endpoint the GitHub repository
    target kind's prerequisite check always calls) 404s three times in the console during the
    pool-form visits. Pre-existing, unrelated to GitLab, and does not block anything under test
    (the GitHub repository kind is not exercised here) — noted, not filed as a #141 finding.
  - The pool detail Overview page still shows the GitLab pool's Account as its GitHub installation
    login ("acme") and "Scope: Gitlab" — this is the already-tracked #159 gap the brief marks
    out of scope, reproduced here only as confirmation, not a new finding.
  - Did not re-verify the dashboard Overview page or any non-GitLab surface beyond what the
    walkthrough path touched incidentally.
- **Evidence:** `/tmp/u141-shots/*.png` (26 screenshots across every state above, both desktop and
  360px) and `/tmp/u141-results.json` (per-step JSON: action, expected, observed, PASS/FAIL,
  request bodies captured for the two POST/PUT assertions, console/HTTP error log). No console
  error, dead component, or blocking overlay was observed outside the deliberate 400/409 test
  responses.
