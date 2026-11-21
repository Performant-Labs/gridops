# Handoff-D: Phase 2 - Design (wireframe) for #169 runner rows and dashboard take the pool platform from the registry

**Date:** 2026-11-21
**Branch:** claude/gitlab-169-runner-rows
**Mode:** (a) generated low-fi
**Wireframe:** docs/handoffs/169/wireframe.html (single static HTML file; exact copy highlighted yellow; orange left border = output changes, grey = must stay exactly as today)

## Screens & states covered
No new screen or control. Three existing places change how their text is worked out. The wireframe draws, per case, the right-hand column at `xl` (text and hover title), the line under the name below `md`, and the details-sheet row.
- **A Runner rows, baseline (unchanged):** GitHub repository pool (repository), GitHub organization pool with no repository (account login), Bitbucket runner on a GitHub pool (as GitHub; only the CI badge says Bitbucket), older server with no `poolScope` (resolves as GitHub's repository scope, as today).
- **B Runner rows, changed:** GitLab pool with a connection name (the name); with only an address (`baseUrl/scopePath`); with one address part (that part, no stray "/"); with neither ("GitLab"); unknown scope ("—", title "Unknown destination"). Never the installation login, never the raw scope.
- **C Dashboard "Runner pools" rows:** scope text "Repository" / "Organization" unchanged (raw scope plus the existing CSS `capitalize`), "GitLab" for a GitLab pool, "Unknown" for an unknown scope.
- **D Pool list `totalRunners`:** API only; the web UI does not display the field, so nothing is drawn (covered by the Rust tests).
- **E Contract table:** every case with resolved platform, destination text, title and dashboard scope text for the Tester and Feature implementor.
The narrow line keeps showing the running job's name when there is one, and has no title, as today.

## Existing components/patterns reused
`RunnerRow`, `RunnerDetails` rows, the dashboard `PoolsSection` row, `platformForPool` and the unknown-scope fallback from #159, the `PoolBindings` descriptor key. No new visual language.

## Open questions for approval
None blocking. Defaults chosen, each a copy call the operator may overrule:
1. **Details-sheet label stays "Repository" for every platform.** The brief's hook returns only `{ text, title }`, and GitHub output must stay exactly as today, so a GitLab runner reads "Repository: Acme / web". Renaming it to "Destination" (or adding a `label` to the hook) would change GitHub output or the hook's shape; left to a follow-up decision.
2. **Unknown-scope copy** "—" / "Unknown destination" / "Unknown", as the brief specifies and as #159 chose for the pool list.
3. **GitLab address with one part** shows that part alone, as #159 did for the pool detail.

## Verification
Rendered headlessly with chrome-headless-shell (no window) and looked at the full-page render and two zoomed crops (GitLab fallbacks and the dashboard section). Fixed one defect found (unreadable case tag in the dashboard frame) and re-rendered the crops. Every control is a Unicode glyph or text stand-in; no SVG geometry was authored. The "⋯" actions menu and status dot are grey stand-ins for the real components.

## Approval
[To be filled by O: "Approved by operator <ISO timestamp>" — D does not self-approve.]
