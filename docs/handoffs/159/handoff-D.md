# Handoff-D: Phase 2 - Design (wireframe) for #159 pool list and detail destination from the registry

**Date:** 2026-11-21
**Branch:** claude/gitlab-159-pool-destination
**Mode:** (a) generated low-fi
**Wireframe:** docs/handoffs/159/wireframe.html (single static HTML file, frames A-E, numbered notes per frame; exact copy highlighted yellow)

## Screens & states covered
No new screen or control. Two existing surfaces change how the destination is worked out.
- **A Pool list, GitHub and Bitbucket (unchanged baseline):** repository with one repo (full name), several repos ("N repositories"), organization (account login), Bitbucket pool (GitHub account). GitHub avatar, same `title`, same truncation.
- **B Pool list, GitLab and fallbacks (changed):** one connection (its name, e.g. "Acme / web"); two or more ("2 connections", all names joined with ", " in the `title`); none or field absent (neutral "GitLab" with the GitLab icon); unknown scope (a dash, neutral placeholder square, no avatar, no icon). Never a GitHub avatar for a non-GitHub-owned pool.
- **C Pool detail Destination, GitHub and Bitbucket (unchanged baseline):** repository (2 repos, and 9 repos with "Show 3 more" / "Show fewer"), organization (Scope, Account with avatar, Runner group #id), Bitbucket (same as GitHub).
- **D Pool detail Destination, GitLab and fallbacks (changed):** Scope "GitLab" then one two-line item per connection (name, then `baseUrl/scopePath`), for one and for two connections; none or absent shows Scope "GitLab" plus a muted "No connections listed"; unknown scope shows Scope "Unknown" and nothing else. No Account row, avatar, repository list or Runner group row for GitLab.
- **E Contract table:** the eight cases (list cell and detail rows, owning descriptor) for the Tester and Feature implementor.

## Existing components/patterns reused
The pool list row and its destination cell (`PoolRow`, `Avatar` slot), the detail `PropertyGroup` / `PropertyRow`, the repository-list chip style (reused for the connection items), `RepositoryList` and its Show N more toggle (moved to `github.tsx`), the registry resolution pattern of `platformForDeliveryId`, and the descriptor `icon` (lucide `Gitlab`, as in `gitlab.tsx`). No new visual language.

## Open questions for approval
None blocking. Defaults chosen where the brief left room (all recorded in the wireframe notes), each a product copy call the operator may overrule:
1. **Unknown scope copy:** list shows "—" (title "Unknown destination"); detail shows Scope "Unknown" only, without echoing the raw scope string.
2. **GitLab no-connections copy:** list "GitLab" with the GitLab icon; detail "No connections listed".
3. **Scope label for GitLab** is the proper "GitLab" (today's raw value capitalised would read "Gitlab").
4. **Partial connection data:** show only the parts present (no stray "/"); an empty name falls back to `baseUrl/scopePath`.
5. **All GitLab connections are listed on the detail** with no "Show more" toggle.
6. The brief's "scope comparison" test criterion is not mechanically greppable (reviewer W-3); the wireframe does not depend on it, the Tester should pin the forbidden tokens.

## Verification
Rendered headlessly with chrome-headless-shell (no window) and looked at the full-page and top-section screenshots. Frames A and B were re-inspected after fixing the hover tooltip's placement (it overlapped a row; now beside the cell). Frames C, D and E were inspected once in the full-page render: layout, chips, labels and tables are complete and on-canvas. The GitLab icon is a "GL" stand-in; the real lucide icon was not rendered here.

## Approval
[To be filled by O: "Approved by operator <ISO timestamp>" — D does not self-approve.]
