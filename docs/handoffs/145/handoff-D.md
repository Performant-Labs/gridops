# Handoff-D: Phase 3 - Design (wireframe) for #145 platform-neutral UI (revision 3)

**Date:** 2026-11-20
**Branch:** claude/ui-145-platform-neutral-cf5832
**Mode:** (a) generated low-fi
**Wireframe:** docs/handoffs/145/wireframe.html (single static HTML file, frames A-G, numbered annotations per frame)
**Revision:** 3 (this pass; the REV3 list is below). Revision 2 follows.
**Revision 2:** Revision 1 (commit 8526086) was BLOCKed in Phase 4 (handoff-A.md) and the brief was amended afterwards. This pass amends the wireframe where the amended brief and the review require it; where the wireframe and the brief disagree, the brief wins. Revised or added items carry a `REV2` tag in the file.

## Revision 3 changes (after the Phase-4 round-2 review and the brief amendments)
Edited in place; each change carries a `REV3` tag in the file. The brief wins on any difference.
- **GitHub App section (B, B note 5).** The route composes the GitHub App section beside the connections list. The descriptor has no `settingsSection` slot, and the wireframe no longer says so (brief "Connections page loading", last bullet).
- **Loading (B note 7, C note 3).** No route loader. Each platform with a fetching list binding loads on its own, shows loading rows only while its own list loads, and has its own Retry. GitHub rows come from the settings page data and have no loading row or Retry. After add, edit or remove only that platform's list reloads.
- **C4 reveal dialog.** The close button reads `I’ve stored it, close` (the extracted dialog's default; a hook may override it with `closeButtonText`), not "Done". Title, description, secret label, details and button text are drawn as the `PostCreateReveal` fields. The API tokens page renders the same extracted dialog with its own strings, so its dialog is unchanged.
- **A (redirects).** The `?appError=` redirect carries no search; the message is a toast (skipped on preload), as today. The old wireframe said the error stayed visible as a Callout on the target page, which the brief rules out.

## Revision 2 changes
- **C4 reveal dialog.** Only the explicit close button closes it (its text is now `I’ve stored it, close`, see revision 3); Escape and outside clicks are ignored (the "or Esc" wording is removed). It is the existing one-time-secret dialog extracted to a shared component, so API tokens and Connections behave identically. New note C7.
- **Bitbucket is not standalone (D2, new D2b, new D7, D notes 2 and 5).** With no GitHub target selected, every Bitbucket workspace option is shown disabled with the reason `Select a GitHub repository or runner group first`; it enables once a repository or runner group is picked. Removing the last GitHub target keeps the workspace selected, leaves Create pool enabled, and submit fails before any request with the schema's thrown message in the existing "Could not create the pool" callout (D7). Pool settings cannot reach that state (E note 5).
- **D5 empty state.** Replaced the invented "Add a connection first" with today's two empty states, "Connect GitHub first" (D5a) and "Install the GitHub App" (D5b), copy supplied by the GitHub descriptor. The gate is "no GitHub installation", not "no targets of any kind". The false "if only one platform has targets" sentence is removed.
- **D notes.** Namespaced internal option keys; per-kind caps counted separately; installation pinned from the first selected repository (mixed installations still allowed, as today); plan-preview sentence contract including the empty-form case.
- **E3 (new).** Read-only pool shows its targets through the same registry-driven display (platform badge + existing summary text), Priority shown by the descriptor flag.
- **B5b (new).** Partial load failure: one platform's error row with its own Retry, other platform's rows still rendered, Retry re-runs only that load.
- **A.** Webhooks empty-state link and the nav item added to the redirect/link table; no link targets a redirecting route.
- **F note 4.** The Bitbucket label rule is the one already exported from `schemas.ts`; no second copy.

## Screens & states covered

- **A. Settings nav + redirects.** "Connections" replaces the GitHub and Bitbucket items; AI agent kept. Table of the four redirects (`/settings/integrations`, `/settings/github`, `/platform-connections`, `?appError=`) and the pool-form link, each pointing straight at `/settings/connections`.
- **B. Connections page (`/settings/connections`).**
  - B1 many (GitHub installations + Bitbucket workspaces in one list: badge, name, detail, status, actions; suspended installation; GitHub App section kept for admins).
  - B2 one. B3 empty: guides to Add connection for admins; read-only variant says "A GridOps administrator can add platform connections."
  - B4 read-only: no Add, no Edit/Remove, no App section.
  - B5 loading and per-platform load error with Retry; B5b partial failure (one platform fails, the other renders; Retry re-runs only the failed load).
  - B6 inline edit (editable-on-update fields only, blank secret keeps the current one, error and saving states).
  - B7 GitHub App not configured (existing create form).
  - GitHub rows are list-only (external Configure link or "Owner approval required"); Bitbucket rows have Edit/Remove. Every action is hidden when the descriptor has no binding or cannot manage.
- **C. Add connection dialog + one-time reveal.**
  - C1 platform chooser first.
  - C2 GitHub (no create binding, so a link out to install the App; disabled when the App is not configured).
  - C3 Bitbucket fields from the descriptor list: idle, submitting, error.
  - C4 post-create reveal. Closes only through its close button, `I’ve stored it, close` (Escape and outside clicks ignored); the value is dropped after close and never offered again.
- **D. New-pool Targets row.**
  - D1 closed with chips. D2 open listbox grouped "Platform · Target kind" with disabled-with-reason incompatible options. D2b nothing selected: Bitbucket options disabled with reason (not standalone).
  - D3 per-group loading / error+Retry / not-connected / no-match rows.
  - D4 organization runner group (single-select, manual "Runner group ID" fallback row).
  - D5 no GitHub installation: "Connect GitHub first" (not signed in) and "Install the GitHub App" (zero installations).
  - D6 maximum reached (per-kind caps). D7 last GitHub target removed while a workspace stays selected: submit enabled, schema error callout, no request.
  - Also the Tart auto-add notice and the plan-preview sentence coming from the target kind.
- **E. Pool settings Targets row.** E1 repository pool (Bitbucket chips shown locked, display-only). E2 organization runner-group pool (listbox restricted to its own installation). E3 read-only pool targets (no controls).
- **F. Platform options.** F1 collapsed by default with neutral shared rows. F2 expanded for two platforms, with the optional component slot. F3 one platform. F4 not rendered when nothing is targeted or no content. F5 label-rule error shown in the Additional labels row while the section is collapsed.
- **G. Runner row.** G1 GitHub, Bitbucket and unknown-platform (dashed fallback) badges. G2 actions menu with "Open on {platform}" shown only when the link builder returns a URL, and the registry-driven delete-confirm text.

Per-state copy is marked in yellow in the file; it keeps today's wording where an equivalent exists.

## Existing components/patterns reused

`SettingsLayout` / `SettingsSection` / `SettingsRow` (page shell, one row shape), `Callout` (read-only, errors, info), `StatusDot`, `Badge`, `Button`, `Input`, `CopyButton` (reveal dialog), `EmptyState`, `InlineError` (retry rows), `useAction` confirm (Remove), `SearchableMultiSelect` extended with optional option fields only (group, disabled + reason, icon, locked, per-group status; no parallel widget, no registry import), the extracted `SecretDialog` for C4, BitbucketSettings's ConnectionRow edit/remove logic, the GitHub settings sections moved verbatim into the GitHub App section, `TartBaseVmRow` and the Additional labels row (made neutral). The only net-new surfaces are the Connections list, the Add connection dialog, the reveal dialog and the Platform options disclosure.

## Open questions for approval

The pipeline has no interactive operator, so each question has a default that preserves current behaviour; the wireframe is drawn with the default.

1. **Bitbucket targets in pool settings.** Default drawn: locked, display-only chips, because the current settings form has no workspace picker and does not send `bitbucketConnectionIds`. The alternative (editable) changes the save payload, which the payload-equality criterion rules out.
2. **Runner link.** Default drawn: "Open on {platform}" in the actions menu, only when the descriptor's link builder returns a URL. The brief names the builder but not where it surfaces.
3. **Runner badge on every row.** Today no badge is shown (only the delete-confirm text uses the platform name). Default drawn: a compact badge after the runner name, hidden below `sm`.
4. **Pool settings Priority section** follows the descriptor flag `supportsPriority` (true for GitHub repository), replacing the `pool.scope === "repository"` check.
5. **User-account installations** in the runner-group kind get an "Enter runner group ID" entry, preserving today's manual fallback.
6. **GitHub in the Add connection chooser** is a link-out (install the GitHub App), because GitHub has no create endpoint; disabled with pointing copy when the App is not configured.
7. **D7 error wording.** The message in the "Could not create the pool" callout is whatever `parseCreateRunnerPoolInput` throws for `installationId: 0`; it is not friendly and is not designed here, because improving it needs a `schemas.ts` rule change that the brief rules out for this story. Default: show the thrown message, as today.
8. **Read-only pool targets (E3).** Default drawn: the existing summary text per kind (`2 repositories`, or the account name for an organization pool) behind a platform badge, plus a Bitbucket line when the pool has workspaces (a small truthful addition: today's read-only view omits them). Alternative: keep the single old "Destination" line unchanged.

Icons are Unicode glyphs only (no hand-drawn SVG geometry). Real icon components (lucide, as used elsewhere) are an implementation choice; the wireframe's glyphs are placeholders.

## Verification of the render

Revision 3 re-rendered headlessly (Chrome `--headless=new` screenshot, no visible window) and inspected the legend, A, B2-B4, B7, C1 and C4 frames, which hold every REV3 change: the new text fits, the C4 button and placeholders are complete and on-canvas, div balance 442/442. Frames B1/B5 and D-G were not touched in revision 3 and were not re-inspected. Revision 2 was rendered headlessly (Chrome `--headless=new` screenshot, no visible window) and inspected the changed and added frames (legend, A, B5/B5b, C2-C4, D1-D7, E1-E3, F1) in the screenshot: glyphs are complete and on-canvas, no clipped controls, div balance 441/441. Frames G and F2-F4 were not changed and were not re-inspected in this pass (revision 1 inspected them). Not checked: dark theme and narrow/mobile widths (low-fi gate; layouts reuse `SettingsRow`, which already stacks on narrow screens).

## Approval

Approval is recorded by the pipeline's unattended gate (actor and time), not by D.
