# Handoff-D: Phase 3 - Design (wireframe) for #145 platform-neutral UI

**Date:** 2026-11-20
**Branch:** claude/ui-145-platform-neutral-cf5832
**Mode:** (a) generated low-fi
**Wireframe:** docs/handoffs/145/wireframe.html (single static HTML file, frames A-G, numbered annotations per frame)

## Screens & states covered

- **A. Settings nav + redirects.** "Connections" replaces the GitHub and Bitbucket items; AI agent kept. Table of the four redirects (`/settings/integrations`, `/settings/github`, `/platform-connections`, `?appError=`) and the pool-form link, each pointing straight at `/settings/connections`.
- **B. Connections page (`/settings/connections`).**
  - B1 many (GitHub installations + Bitbucket workspaces in one list: badge, name, detail, status, actions; suspended installation; GitHub App section kept for admins).
  - B2 one. B3 empty: guides to Add connection for admins; read-only variant says "A GridOps administrator can add platform connections."
  - B4 read-only: no Add, no Edit/Remove, no App section.
  - B5 loading and per-platform load error with Retry.
  - B6 inline edit (editable-on-update fields only, blank secret keeps the current one, error and saving states).
  - B7 GitHub App not configured (existing create form).
  - GitHub rows are list-only (external Configure link or "Owner approval required"); Bitbucket rows have Edit/Remove. Every action is hidden when the descriptor has no binding or cannot manage.
- **C. Add connection dialog + one-time reveal.**
  - C1 platform chooser first.
  - C2 GitHub (no create binding, so a link out to install the App; disabled when the App is not configured).
  - C3 Bitbucket fields from the descriptor list: idle, submitting, error.
  - C4 post-create reveal. Closes only through Done or Esc; the value is dropped after close and never offered again.
- **D. New-pool Targets row.**
  - D1 closed with chips. D2 open listbox grouped "Platform · Target kind" with disabled-with-reason incompatible options.
  - D3 per-group loading / error+Retry / not-connected / no-match rows.
  - D4 organization runner group (single-select, manual "Runner group ID" fallback row).
  - D5 no targets / not signed in empty state ("Add a connection first").
  - D6 maximum reached.
  - Also the Tart auto-add notice and the plan-preview sentence coming from the target kind.
- **E. Pool settings Targets row.** E1 repository pool (Bitbucket chips shown locked, display-only). E2 organization runner-group pool (listbox restricted to its own installation).
- **F. Platform options.** F1 collapsed by default with neutral shared rows. F2 expanded for two platforms, with the optional component slot. F3 one platform. F4 not rendered when nothing is targeted or no content. F5 label-rule error shown in the Additional labels row while the section is collapsed.
- **G. Runner row.** G1 GitHub, Bitbucket and unknown-platform (dashed fallback) badges. G2 actions menu with "Open on {platform}" shown only when the link builder returns a URL, and the registry-driven delete-confirm text.

Per-state copy is marked in yellow in the file; it keeps today's wording where an equivalent exists.

## Existing components/patterns reused

`SettingsLayout` / `SettingsSection` / `SettingsRow` (page shell, one row shape), `Callout` (read-only, errors, info), `StatusDot`, `Badge`, `Button`, `Input`, `CopyButton` (reveal dialog), `EmptyState`, `InlineError` (retry rows), `useAction` confirm (Remove), `SearchableMultiSelect` extended with an optional option `group` (no parallel widget), BitbucketSettings's ConnectionRow edit/remove logic, the GitHub settings sections moved verbatim into the GitHub App section, `TartBaseVmRow` and the Additional labels row (made neutral). The only net-new surfaces are the Connections list, the Add connection dialog, the reveal dialog and the Platform options disclosure.

## Open questions for approval

The pipeline has no interactive operator, so each question below has a default that preserves current behaviour; the wireframe is drawn with the default.

1. **Bitbucket targets in pool settings.** Default drawn: shown as locked chips, not editable, because the current settings form has no workspace picker and does not send `bitbucketConnectionIds`. The alternative (editable, which changes the save payload) is out of scope per the payload-equality criterion.
2. **Runner link.** Default drawn: "Open on {platform}" item in the actions menu, only when the descriptor's link builder returns a URL. The brief names the builder but not where it surfaces.
3. **Runner badge on every row.** Today no badge is shown (only the delete-confirm text uses the platform name). Default drawn: a compact badge after the runner name, hidden below `sm`.
4. **Pool settings Priority section** stays after Destination and is shown when a selected target kind supports prioritisation (descriptor flag `supportsPriority`, true for GitHub repository), replacing the `pool.scope === "repository"` check.
5. **User-account installations** in the runner-group kind get an "Enter runner group ID" entry, preserving today's manual fallback; organization installations list real groups.
6. **GitHub on the Add connection chooser** is a link-out (install the GitHub App), not a form, because GitHub has no create endpoint. If the App is not configured the action is disabled with copy pointing at the App setup section.

Icons are Unicode glyphs only (no hand-drawn SVG geometry). Real icon components (lucide, as used elsewhere) are an implementation choice; the wireframe's glyphs are placeholders.

## Verification of the render

Rendered headlessly (Chrome `--headless=new` screenshot, no visible window) and inspected frames A-G section by section: glyphs complete and on-canvas, no clipped controls. Fixed three issues found on the first look (nav tag wrapping, block-level copy highlight layout in empty states, wrong frame reference). Not checked: dark theme and narrow/mobile widths (low-fi gate; layouts reuse `SettingsRow`, which already stacks on narrow screens).

## Approval

Approval is recorded by the pipeline's unattended gate (actor and time), not by D.
