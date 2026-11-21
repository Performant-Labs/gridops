# Handoff-U: Phase 8 - UI walkthrough for #159 (pool list / pool detail destination from the registry)

**Date:** 2026-11-21
**Branch:** claude/gitlab-159-pool-destination (head 8c18212)
**Issue:** #159
**Handoff-T-green reviewed:** docs/handoffs/159/handoff-T-green.md (GREEN)
**Handoff-F reviewed:** docs/handoffs/159/handoff-F.md
**Handoff-A-dup reviewed:** docs/handoffs/159/handoff-A-dup.md (PASS)
**Approved wireframe:** docs/handoffs/159/wireframe.html + docs/handoffs/159/handoff-D.md

## T precondition
Confirmed. `handoff-T-green.md` and `handoff-A-dup.md` are both PASS at this head, with no `src`/`crates` diff since F's `4db2892`.

## Wireframe conformance
No divergence. Every frame the wireframe specified (A/B list, C/D detail, the fallback cases) was reproduced live and matched: GitHub/Bitbucket pools render byte-identical destination cells and rows to the pre-#159 baseline; GitLab pools show the registry-resolved connection name(s)/GitLab icon in the list and "GitLab" + connection chips in the detail, with no Account/avatar/Runner-group row; the unknown-scope fallback shows "—" (title "Unknown destination") in the list and "Scope: Unknown" alone in the detail, with no raw scope string echoed anywhere. See the per-state table below for exact text captured live.

## Run environment
GridOps has no e2e seed script and its only login is GitHub OAuth (same situation prior U rounds for #141/#145 recorded), so this pass built and served the real Rust API against a throwaway SQLite DB, seeded directly, with a hand-minted session cookie (same technique #145's U round used):

- **Build:** reused the existing, current build artifacts — `target/debug/gridops-api` (mtime after F's last commit, `git diff --stat HEAD -- src crates` empty) and `dist/` (vite build, same check).
- **Serve:** `target/debug/gridops-api` on `127.0.0.1:19301`, with:
  `GRIDOPS_DATABASE_PATH=<scratch>/u159.sqlite`, `GRIDOPS_BASE_URL=http://127.0.0.1:19301`, `GRIDOPS_API_BIND=127.0.0.1:19301`, `GRIDOPS_WEB_ROOT=<worktree>/dist`, `GRIDOPS_SESSION_SECRET=u159-walkthrough-session-secret`, `GRIDOPS_ENCRYPTION_KEY=<32-byte base64>`, `GRIDOPS_FLEET_ALLOW_LOOPBACK_HTTP=true`, `GITHUB_CLIENT_ID`/`GITHUB_CLIENT_SECRET=<dummy>` (only required so `validate_api()` passes at startup; no GitHub API call was made — GitHub App features were not exercised, only the already-fetched installation/repository/account rows).
- **DB:** migrated automatically on first connection (`sqlx::migrate!`), then seeded via a throwaway Node+sqlite3 script (`<scratch>/seed.mjs`, modeled on `docs/handoffs/145/screenshots/repro/seed.mjs`'s session-signing approach) with:
  - one admin user + session;
  - one GitHub installation (`acme-labs`, organization) with 3 repositories;
  - one Bitbucket connection (`Acme Bitbucket`);
  - two GitLab connections (`Acme / web`, `Acme / mobile`);
  - 8 runner pools covering every state the brief's Done-when names: GitHub repository pool with one repo (`pool-gh-one`), GitHub repository pool with several repos (`pool-gh-many`, 3 repos), GitHub organization pool (`pool-gh-org`), a Bitbucket (GitHub-scoped) pool with a Bitbucket connection attached (`pool-bb`), a GitLab pool with one connection (`pool-gl-one`), a GitLab pool with two connections (`pool-gl-two`), a GitLab pool with no connections (`pool-gl-zero`), and an unknown-scope pool (`pool-unknown`, `scope='azure-devops'`).
- **Login:** session cookie minted with the same HMAC-SHA256(`GRIDOPS_SESSION_SECRET`)-over-token scheme `auth.rs::create_session`/`Vault::sign` use, set via `browser_evaluate` document.cookie, then a hard load of `/` confirmed the authenticated shell renders (sidebar, "Control plane online").
- **Driver:** Playwright MCP, headless Chromium, direct `mcp__playwright__*` tool calls (not `u-drive.mjs` — that helper assumes an HTMX email/password form and HTMX swap timing; GridOps is a React/TanStack-Router SPA with no HTMX, the same reasoning the #141 U rounds recorded). Viewports: 1280×900 (desktop — the destination column is `hidden … xl:flex`, so 1280 is required to see it) and 360×800 (mobile).
- **Cleanup:** `pkill -f target/debug/gridops-api` (confirmed down via `curl` returning no response), throwaway SQLite DB removed. `target/` and `dist/` are pre-existing build artifacts from F's own self-check, left in place (not session-owned).
- **State safety:** no shared/account-wide setting was touched — this was a dedicated throwaway DB and admin user created for this pass only, not a shared operator account.

## State matrix walked

| # | Page | Pool / state | Reached via | Result |
|---|---|---|---|---|
| 1 | `/runner-pools` list | all 8 pools, one page | **Hard load** (direct `page.goto`) | PASS — "Runner pools 8" heading; every row present |
| 2 | `/runner-pools` list | same | **SPA nav** (sidebar "Runner pools" link, `getByRole('link').click()`, URL updates without reload) | PASS — identical destination cell text/title captured before and after SPA nav (see Evidence); no discrepancy |
| 3 | List destination cell | GitHub repository pool, **one** repo (`pool-gh-one`) | SPA (row visible on list) | PASS — text/title `acme-labs/web-app`, `<img>` GitHub avatar `https://github.com/acme-labs.png?size=40` |
| 4 | List destination cell | GitHub repository pool, **many** repos (`pool-gh-many`, 3) | SPA | PASS — text/title `3 repositories`, GitHub avatar |
| 5 | List destination cell | GitHub organization pool (`pool-gh-org`) | SPA | PASS — text/title `acme-labs`, GitHub avatar |
| 6 | List destination cell | Bitbucket (GitHub-scoped) pool (`pool-bb`) | SPA | PASS — text/title `acme-labs/ios-client`, GitHub avatar — **identical to the GitHub-repository-pool baseline**, no Bitbucket workspace shown, matching the brief's "Bitbucket pool keeps its GitHub account destination" |
| 7 | List destination cell | GitLab pool, **one** connection (`pool-gl-one`) | SPA | PASS — text/title `Acme / web`, lucide `Gitlab` icon (`svg.lucide-gitlab`), **no** `<img>`/GitHub avatar |
| 8 | List destination cell | GitLab pool, **two** connections (`pool-gl-two`) | SPA | PASS — text `2 connections`, title `Acme / web, Acme / mobile` (both names), GitLab icon, no avatar |
| 9 | List destination cell | GitLab pool, **zero** connections (`pool-gl-zero`) | SPA | PASS — text/title `GitLab` (neutral fallback, not the installation's GitHub account), GitLab icon, no avatar |
| 10 | List destination cell | **unknown** scope pool (`pool-unknown`, `azure-devops`) | SPA | PASS — text `—`, title `Unknown destination`, a dashed placeholder `<span>` (no `<img>`, no `<svg>` icon) — never a GitHub avatar |
| 11 | `/runner-pools/pool-gh-many` detail, Destination group | GitHub repository pool, many repos | SPA (click "Open multi-repo-builds") | PASS — Scope "repository", Account "acme-labs" + avatar, repository chips `acme-labs/web-app`/`acme-labs/api-service`/`acme-labs/ios-client` — unchanged baseline shape |
| 12 | `/runner-pools/pool-gh-org` detail, Destination group | GitHub organization pool | Hard load | PASS — Scope "organization", Account "acme-labs" + avatar, "Runner group #1" row present |
| 13 | `/runner-pools/pool-bb` detail, Destination group | Bitbucket (GitHub-scoped) pool | Hard load | PASS — Scope "repository", Account "acme-labs" + avatar, repository "acme-labs/ios-client" — **identical to a GitHub repository pool**, no Bitbucket-specific row |
| 14 | `/runner-pools/pool-gl-two` detail, Destination group | GitLab pool, two connections | SPA (click "Open gitlab-two") | PASS — Scope **"GitLab"** (not "Gitlab"), two chips each showing name then `baseUrl/scopePath` (`Acme / web` / `https://gitlab.com/acme/web`, `Acme / mobile` / `https://gitlab.com/acme/mobile`) in server (`membership.created_at`) order, **no** Account row, **no** avatar, **no** Runner-group row |
| 15 | `/runner-pools/pool-gl-zero` detail, Destination group | GitLab pool, zero connections | Hard load | PASS — "Scope: GitLab" then "No connections listed" (muted), no Account/avatar/Runner-group row |
| 16 | `/runner-pools/pool-unknown` detail, Destination group | unknown scope | Hard load | PASS — "Scope: Unknown" only — the raw scope string (`azure-devops`) is never echoed, no Account/avatar/repository/Runner-group row |
| 17 | 360px mobile | `/runner-pools` list | resize + reload | PASS — no horizontal overflow (`scrollWidth === clientWidth`); the destination column is intentionally `hidden … xl:flex` (desktop-only), a **pre-existing** layout rule untouched by #159 (not in F's diff), not a new defect |
| 18 | 360px mobile | `/runner-pools/pool-gl-two` detail | resize + reload | PASS — Destination group renders in the sidebar stack, both connection chips visible, no overflow. Screenshot saved. |

**Zero/empty state:** walked and mandatory-covered (rows 9, 15 — GitLab pool with no connections) and the unknown-scope fallback (rows 10, 16), per the brief's explicit "none or absent" and "unknown scope" cases.

**Console / state assertions:** `browser_console_messages` (warning level, cumulative across the whole session) returned **0 messages** at every checkpoint — list (hard load and after SPA nav), every pool detail (hard load and SPA nav), and both viewports. No Alpine/HTMX concept applies to this stack (React/TanStack Router SPA); the pin on "no dead component" was addressed by inspecting the live DOM directly (icons/avatars/text) rather than a framework introspection API, since React has none analogous to Alpine's data stack.

## Findings
None blocking.

1. **All 8 contract-table cases from the brief's "Done when" render correctly, live, in both the list and the detail**, matching the approved wireframe's per-state copy and controls exactly (text, title/tooltip, avatar-vs-icon-vs-placeholder, row set).
2. **GitHub and Bitbucket pools are pixel-for-pixel/byte-for-byte identical to the pre-#159 baseline** in both views — confirmed live, not just by reading the diff.
3. **No regression on SPA nav vs hard load** — the list's destination cells captured identical text/title before and after a real sidebar-link SPA navigation.
4. **No console error or warning** anywhere in the session.
5. One pre-existing, out-of-scope layout note: the list's destination column is hidden below the `xl` breakpoint (1280px) by a Tailwind class (`hidden w-40 … xl:flex`) that #159 did not touch — at 360/768/1024px a user cannot see the destination cell on the list at all (only on the detail page). This predates #159 (not in F's diff) and is not a #159 defect; noting it for completeness only.

## Evidence
- Screenshots (session-scratch, saved under the worktree so this tool could write them):
  - `docs/handoffs/159/screenshots/u-list-1280.png` — full pool list at 1280px, all 8 pools visible with correct destination cells.
  - `docs/handoffs/159/screenshots/u-detail-gitlab-two-360.png` — GitLab pool detail (two connections) at 360px, no overflow.
- Live DOM captures (via `browser_evaluate`) of every list row's destination cell (`title`, `textContent`, presence of `<img>`/`<svg>`/dashed placeholder, icon class, avatar `src`) and every detail page's Destination `PropertyGroup` text — see the state-matrix table above for the exact values captured.
- `browser_console_messages` (warning level, `all: true`): `Total messages: 0 (Errors: 0, Warnings: 0)` at the final checkpoint, covering the entire session.
- API sanity cross-check (`curl` against the live server, same seeded DB): `GET /api/v1/runner-pools?page=1` returns `gitlabConnections: []` for every GitHub/Bitbucket/unknown pool and the correct ordered connection array for each GitLab pool — confirming the UI's source data matches the brief's API contract.

## Verdict
**PASS** — every state the brief's Done-when and the approved wireframe specify (GitHub one/many repos, GitHub organization, Bitbucket, GitLab one/two/zero connections, unknown scope) was independently reproduced in a real headless browser, on both the SPA-navigation and hard-load paths, at desktop (1280px) and mobile (360px), with a clean console throughout. No behavioral defect, no wireframe divergence. Ready for S.
