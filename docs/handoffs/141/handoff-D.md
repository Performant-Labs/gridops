# Handoff-D: Phase 2 - Design (wireframe) for #141 GitLab in the shared UI

**Date:** 2026-11-20
**Branch:** claude/gitlab-141-ui-3ae7f5
**Mode:** (a) generated low-fi
**Wireframe:** docs/handoffs/141/wireframe.html (single static HTML file, frames A-I, numbered notes per frame; revision 3)

## Screens & states covered
No new screen: every frame is an existing #145 surface with GitLab content. Exact copy is highlighted yellow in the file.
- **A Connections list:** many (GitHub, Bitbucket, a GitLab project, a GitLab group); GitLab empty row ("No GitLab projects or groups are connected yet."); read-only member (no Add, no actions); load error with Retry. Row cluster order: Edit, Verify, Rotate webhook secret, Remove, only when `canManage`.
- **B Add connection:** chooser entry; form (name, GitLab address default `https://gitlab.com`, Project/Group, path, token with the `create_runner` + `read_api` + Owner help); inline URL and path errors (the field `validate` hook); server errors in a callout (bad token, missing scope, missing role, 409 duplicate, upstream), message verbatim; submitting.
- **C Reveal and rotation:** create reveal (secret in the main copy box; webhook URL with its own Copy button and the "tick Job events" steps in `details`), rotate confirm, rotate reveal, rotate failure toast.
- **D Row actions:** Verify running, success toast, failure toast (server message); inline Edit (name + optional token only, PATCH; scope immutable) with error; Remove confirm, success, and 409 blocked toast.
- **E Targets (revision 3: E5, E5b, E6):** two kinds ("GitLab · Project", "GitLab · Group"), standalone, one option per connection with `<baseUrl>/<scopePath>`; mixed project+group; GitHub and Bitbucket options disabled with "Can't combine with GitLab targets" (and the reverse via their own reasons); plan sentence; not-connected status rows; pool settings for a GitLab pool with the listbox open (E5: options fetched with `getGitlabConnections`, so an unused "Docs site" connection can be added; no locked GitHub chip; GitHub and Bitbucket options do not appear at all), settings load states (E5b: loading, per-kind error with Retry), and a GitHub or Bitbucket pool's settings with no GitLab group at all (E6).
- **F Platform options and submit:** collapsed and expanded section, the `Run untagged jobs` switch (off by default, only while GitLab is targeted); Additional labels copy; tag errors (shared 64-character cap, shown at submit); submit errors including the no-installation case.
- **G Runner:** GitLab badge on the row; details panel "CI platform" shows the name as a link to `<baseUrl>/<scopePath>/-/runners/<id>` when the three optional runner fields exist (now sent by the runners list through the one narrow backend edit), plain name otherwise.
- **H Deliveries:** platform badge (the exported `PlatformBadge`, derived from the `gitlab:` id prefix) before the destination, GitLab rows with no destination text; the detail sheet gets the badge beside its heading and **no new row** (H2 revised), Destination "—" for non-GitHub rows without a repository.
- **I Gaps:** the generic descriptor-contract fixes (table) and the out-of-scope polling errors.

## Existing components/patterns reused
Connections list/row cluster, Add connection dialog and `FieldInput`, `SecretRevealDialog` (unchanged, `details` + `CopyButton`), `confirmAction` dialog, `useAction` toasts, the Targets listbox/chips/`unavailableReason`/`notConnected`/`planNote`, `PlatformOptionsSection`, `labelRules`, runner details "CI platform" row and `runnerLink`, the webhooks list row and detail sheet, `Callout`, `Badge`. `SearchableSelect` (the Project/Group field), `PlatformBadge`, the existing create success toast, the Targets per-kind load states (loading, error with Retry, `No {kind} options`). Closest analogue: the Bitbucket descriptor (workspace = one connection = one option).

## Open questions for approval
Revision 2, after A round 1 (BLOCK) and the amended brief. Resolved by the brief and no longer open: the Project/Group `select` (gap 5a), the path label (now static), the 255-character tag rule (shared 64 wins), the delivery platform lookup (registry, gap 7), the confirm label (`confirmOptions.confirmLabel`), the GitHub chip on GitLab pool settings (gap 4). Still open:
1. **Comma and empty tag rules are unreachable from the field.** "Additional labels" splits on commas and drops empty entries (`runner-pools.new.tsx:169`), so the brief's walkthrough item "tag validation error on a comma" cannot be performed through the UI. Wireframe default: the walkthrough shows the 64-character error (shared schema message, at submit); the comma and empty rules stay in `gitlabLabelError` and are covered by Vitest.
2. **Verify outcome.** No status field exists, so success/failure is toast only and the caption stays "Verified {updatedAt}". Accept, or ask #138 for a `verifiedAt`/status field.
3. **Deliveries.** GitHub rows also get a platform badge; GitLab rows show no destination text (no project field in the list), the detail sheet shows "—". Accept, or ask #140 for a displayable destination.
4. **No-installation submit.** Shows the validator's or server's message verbatim with no new copy (#138 still requires `installationId`). Accept, or specify friendlier copy (it would belong in a shared schema, not a GitLab string).
5. **Reveal order.** The secret is the dialog's primary copy box and the URL sits in `details` below it, because `SecretRevealDialog` has one value slot; no component change.

## Revision 3 changes (after A round 2 BLOCK and the brief amended in 9b9f6ff)
- E5 redrawn and E5b/E6 added: on settings the GitLab options are fetched (the pool detail lists only the pool's own connections); GitHub/Bitbucket options do not appear on a GitLab pool; no GitLab group on a GitHub pool. E note 2 rewritten (create: pool options; settings: fetch; `offered`/`available`).
- H2: the extra "Platform" row is dropped; the badge sits beside the sheet heading. H1 note: the exported `PlatformBadge`, no third badge.
- B note 4: the create toast stays and the reveal opens after it. B note 1: the select is `SearchableSelect` (`searchable={false}`).
- G note: the runner fields come from the narrow backend edit to the runners list.
- I: gap 5 and gap 7 reworded (SearchableSelect; PlatformBadge and the runner-row-style neutral-source assertion); two new rows (runner JSON backend edit; settings option source).
- Open question 1 (comma tag rule) is unchanged: the brief's walkthrough still says "tag validation error on a comma", which the comma-split Additional labels field cannot produce; the wireframe keeps the 64-character error at submit.

## Revision 2 changes
- B2/B3/notes: path label "Project or group path" (static); defaults and inline validation mapped to gap 5a-c.
- F1-F3 and notes: tag cap 64 (not 255); F3 redrawn to show only what can appear (the schema's message at submit).
- E5: note that no locked GitHub chip appears (gap 4).
- C notes: `SecretReveal` rename and the single `SecretRevealDialog` via `onReveal`.
- H notes: `platformForDeliveryId` in `registry.tsx`; fallback destination text is a descriptor value.
- F4: body now includes `repositoryIds: []` and the registry-order note (gap 6).
- I: table renumbered to the brief's gaps 1-7, plus the two out-of-reach rows (polling errors; #159).

## Verification
Revision 3: rendered headlessly with chrome-headless-shell (no window) and looked at the screenshots of frames E (E4 to E6) and H after the edits; both are complete and on-canvas. Frames B (two note edits), G (one note edit) and I (table text) were edited in text only and not re-inspected visually. Revision 2: inspected frames B, C (partly), E5, F, G, H and I; layout, badges, switches, dialogs, callouts and tables are complete and on-canvas. Frames A and D were inspected in the previous revision and not changed. The B5 "Connecting…" label is low-contrast on the dark disabled button in this low-fi render only. Badge glyphs are Unicode stand-ins; the real lucide `Gitlab` icon was not rendered and may not exist in the installed lucide (`node_modules` is absent in this worktree).

## Approval
[To be filled by O: "Approved by operator <ISO timestamp>" — D does not self-approve.]
