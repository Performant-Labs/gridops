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
