# Handoff-D: Phase 2 - Design (wireframe) for #141 GitLab in the shared UI

**Date:** 2026-11-20
**Branch:** claude/gitlab-141-ui-3ae7f5
**Mode:** (a) generated low-fi
**Wireframe:** docs/handoffs/141/wireframe.html (single static HTML file, frames A-I, numbered notes per frame)

## Screens & states covered
No new screen: every frame is an existing #145 surface with GitLab content. Exact copy is highlighted yellow in the file.
- **A Connections list:** many (GitHub, Bitbucket, a GitLab project, a GitLab group); GitLab empty row ("No GitLab projects or groups are connected yet."); read-only member (no Add, no actions); load error with Retry. Row cluster order: Edit, Verify, Rotate webhook secret, Remove, only when `canManage`.
- **B Add connection:** chooser entry; form (name, GitLab address default `https://gitlab.com`, Project/Group, path, token with the `create_runner` + `read_api` + Owner help); inline URL and path errors; server errors in a callout (bad token, missing scope, missing role, 409 duplicate, upstream), message verbatim; submitting.
- **C Reveal and rotation:** create reveal (secret in the main copy box; webhook URL with its own Copy button and the "tick Job events" steps in `details`), rotate confirm, rotate reveal, rotate failure toast.
- **D Row actions:** Verify running, success toast, failure toast (server message); inline Edit (name + optional token only, PATCH; scope immutable) with error; Remove confirm, success, and 409 blocked toast.
- **E Targets:** two kinds ("GitLab · Project", "GitLab · Group"), standalone, one option per connection with `<baseUrl>/<scopePath>`; mixed project+group; GitHub and Bitbucket options disabled with "Can't combine with GitLab targets" (and the reverse via their own reasons); plan sentence; not-connected status rows; pool settings for a GitLab pool.
- **F Platform options and submit:** collapsed and expanded section, the `Run untagged jobs` switch (off by default, only while GitLab is targeted); Additional labels copy; tag errors (255 characters, empty); submit errors including the no-installation case.
- **G Runner:** GitLab badge on the row; details panel "CI platform" shows the name as a link to `<baseUrl>/<scopePath>/-/runners/<id>` when the three optional fields exist, plain name otherwise.
- **H Deliveries:** platform badge (registry, derived from the `gitlab:` id prefix) before the destination, GitLab rows with no destination text; new "Platform" row in the detail sheet, Destination "—" for non-GitHub rows without a repository.
- **I Gaps:** the generic descriptor-contract fixes (table) and the out-of-scope polling errors.

## Existing components/patterns reused
Connections list/row cluster, Add connection dialog and `FieldInput`, `SecretRevealDialog` (unchanged, `details` + `CopyButton`), `confirmAction` dialog, `useAction` toasts, the Targets listbox/chips/`unavailableReason`/`notConnected`/`planNote`, `PlatformOptionsSection`, `labelRules`, runner details "CI platform" row and `runnerLink`, the webhooks list row and detail sheet, `Callout`, `Badge`. Closest analogue: the Bitbucket descriptor (workspace = one connection = one option).

## Open questions for approval
1. **Project/Group choice.** The create form needs it (API `scopeKind`), and `ConnectionField` has no select. Wireframe default: add a generic `select` field type (frame I gap 4). Alternative: two chooser entries sharing one descriptor (larger contract change).
2. **Comma tag rule.** "Additional labels" is comma-split, so a tag cannot contain a comma and the brief's walkthrough item "tag validation error on a comma" is unreachable. Wireframe default: the walkthrough checks the 255-character and empty-tag errors; the comma rule stays in `gitlabLabelError` and is covered by Vitest.
3. **Verify outcome.** No status field exists, so success/failure is toast only and the caption stays "Verified {updatedAt}". Accept, or ask #138 for a `verifiedAt`/status field.
4. **Deliveries.** GitHub rows also get a platform badge; GitLab rows show no destination text (no project field in the list). Accept, or ask #140 for a displayable destination.
5. **No-installation submit.** Shows the validator's or server's message verbatim with no new copy. Accept, or specify friendlier copy (it would belong in a shared schema, not a GitLab string).
6. **Confirm label.** "Rotate secret" needs an optional `confirmLabel` on the generic row action (otherwise the dialog says "Continue").
7. **Reveal order.** The secret is the dialog's primary copy box and the URL sits in `details` below it, because `SecretRevealDialog` has one value slot; no component change.

## Verification
Rendered headlessly with chrome-headless-shell and inspected the screenshots of frames A-F (layout, badges, buttons, dialogs, toasts, switch all complete and on-canvas). Frames G, H and I were not visually inspected. Badge/icon glyphs are Unicode stand-ins; the real lucide `Gitlab` icon was not rendered and may not exist in the installed lucide (node_modules absent here).

## Approval
[To be filled by O: "Approved by operator <ISO timestamp>" — D does not self-approve.]
