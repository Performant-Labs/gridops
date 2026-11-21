# Handoff-U: Phase 8 - UI walkthrough for #169 (runner rows and dashboard take the pool platform from the registry)

**Date:** 2026-11-21
**Branch:** claude/gitlab-169-runner-rows (head 97d74c2)
**Issue:** #169
**Handoff-T-green reviewed:** docs/handoffs/169/handoff-T-green.md (GREEN)
**Handoff-F reviewed:** docs/handoffs/169/handoff-F.md
**Approved wireframe:** docs/handoffs/169/wireframe.html

## T precondition
Confirmed. `handoff-T-green.md` is GREEN, with no blockers. `git diff --stat HEAD -- src crates` is empty, so the working tree matches the commit T and F verified; I rebuilt both artifacts from this exact tree (`cargo build -p gridops-api`, `npm run build`) rather than trust stale binaries.

## Wireframe conformance
No divergence. Every frame the wireframe specifies (A. GitHub/Bitbucket/older-server baseline, B. GitLab + unknown scope runner rows, C. dashboard scope text) was reproduced live and matched exactly:

- **R1/R2/R3 (GitHub repository, GitHub organization, Bitbucket-on-GitHub-pool)** — unchanged: `acme-labs/web` and `acme-labs` exactly as today, in all three places (right-hand column + title, narrow line, details-sheet "Repository" row).
- **L1 (GitLab, connection with a name)** — "Acme / web" (text = title) everywhere.
- **L2 (GitLab, connection with no name, full address)** — "https://gitlab.example.com/acme/web" everywhere (scheme kept, per T's pinned assumption).
- **L2b (GitLab, connection with no name, only a base URL)** — "https://gitlab.example.com", no stray trailing slash.
- **L3 (GitLab, no connection at all)** — "GitLab" (never the installation login — this runner's own `accountLogin` is `acme-labs`, and it is never shown).
- **U1 (unknown pool scope)** — text "—", title "Unknown destination", in the row, the narrow line, and the details sheet; the raw scope (`quantum`) is never echoed anywhere.
- **Details-sheet label stays "Repository"** for every platform, exactly as D's open question 1 resolved.
- **Dashboard scope text (C.)** — "repository · ephemeral" / "organization · ephemeral" / "GitLab · ephemeral" / "Unknown · ephemeral", matching D1–D4 exactly (GitHub/Bitbucket keep the raw scope under the existing `capitalize` CSS; GitLab shows "GitLab" not "Gitlab"; unknown shows "Unknown").
- **D. `totalRunners`** — API-only, no drawn UI; verified live (Run environment / Evidence below) rather than drawn, per the wireframe's own note.

No control the wireframe specified was missing, and no observed copy or behavior diverged from the approved target.

## Run environment
GridOps has no e2e seed script and its only login is GitHub OAuth (same situation as #141/#145/#159's U rounds), so this pass built and served the real Rust API against a throwaway SQLite DB, seeded directly, with a hand-minted session cookie (same technique as #159's U round):

- **Build:** rebuilt both artifacts from the current tree to avoid relying on stale binaries, even though `git diff --stat HEAD -- src crates` was already empty: `CARGO_BUILD_JOBS=2 cargo build -p gridops-api` (29.7s, clean) and `npm run build` (vite build + `tsc --noEmit`, clean).
- **Serve:** `target/debug/gridops-api` on `127.0.0.1:19302`, with `GRIDOPS_DATABASE_PATH=<scratch>/u169.sqlite`, `GRIDOPS_BASE_URL=http://127.0.0.1:19302`, `GRIDOPS_API_BIND=127.0.0.1:19302`, `GRIDOPS_WEB_ROOT=<worktree>/dist`, `GRIDOPS_SESSION_SECRET` and `GRIDOPS_ENCRYPTION_KEY` (random, local-only), `GRIDOPS_FLEET_ALLOW_LOOPBACK_HTTP=true`, `GITHUB_CLIENT_ID`/`GITHUB_CLIENT_SECRET` (dummy values, only so `validate_api()` passes at startup — no GitHub API call was made; GitHub App features were not exercised).
- **DB:** migrated automatically on first connection (`sqlx::migrate!`), then seeded via a throwaway Node+sqlite3 script (`<scratch>/seed.mjs`, modeled on #159's `docs/handoffs/159/screenshots/repro/seed.mjs`) with:
  - one admin user + session;
  - one GitHub installation (`acme-labs`, organization) with one repository (`acme-labs/web`);
  - one Bitbucket connection (`Acme Bitbucket`);
  - three GitLab connections: `gl-named` (name "Acme / web", `https://gitlab.example.com/acme/web`), `gl-addr` (empty name, same address — connection names are the only schema-nullable-by-choice field; `name` is `NOT NULL` so I used an empty string to drive the `name || address` fallback), `gl-addr1` (empty name, base URL only, empty `scope_path`);
  - five runner pools covering every state the brief/wireframe name: GitHub repository (`pool-gh-repo`), GitHub organization (`pool-gh-org`), Bitbucket-on-GitHub-scoped (`pool-bb`), GitLab (`pool-gl`, carrying all four GitLab runner states), and an unknown scope (`pool-unknown`, `scope='quantum'`);
  - eight runners, one per wireframe case: R1 (`pool-gh-repo`, GitHub, repository present), R2 (`pool-gh-org`, GitHub, no repository — falls back to `accountLogin`), R3 (`pool-bb`, Bitbucket platform on the GitHub-scoped pool), L1 (`pool-gl`, `gitlab_connection_id='gl-named'`), L2 (`gitlab_connection_id='gl-addr'`), L2b (`gitlab_connection_id='gl-addr1'`), L3 (`pool-gl`, `gitlab_connection_id=NULL` — no connection at all), U1 (`pool-unknown`).
  - `runner_pool_installations` needed no manual seeding: migration 0014's `AFTER INSERT ON runner_pools` trigger populates it automatically, confirmed live (the dashboard and pool list showed every pool with no extra insert).
  - a ninth, runner-less pool (`pool-empty`) added directly via `sqlite3` after the walkthrough's browser pass, to verify `totalRunners: 0` live against the running server (API-only per the wireframe; "no drawn UI" for this case).
  - **The "older server, no `poolScope`" case (R4) could not be produced live** — the real server always emits `poolScope` now (`p.scope AS pool_scope`, `NOT NULL`), so there is no way to make a live server omit it. This case is covered only by the jsdom test per T's handoff (`older server with no poolScope` in `tests/runner-row-platform.test.ts`); I did not re-derive it live, since no server build can produce it. Noting this as a gap in live reproduction (not a wireframe/behavioral defect — the code path can only be exercised via a synthetic object, which is what the unit test does).
- **Login:** session cookie minted with the vault's own `HMAC-SHA256(GRIDOPS_SESSION_SECRET)` scheme (`crates/gridops-core/src/crypto.rs::Vault::sign`/`verify`, `crates/gridops-api/src/auth.rs::SESSION_COOKIE = "gridops_session"`), set via `browser_evaluate` document.cookie on `127.0.0.1` (host match), then confirmed with a hard load of `/` (authenticated shell: sidebar, "Control plane online").
- **Driver:** Playwright MCP, headless Chromium, direct `mcp__playwright__*` tool calls — not `u-drive.mjs` (that helper assumes an HTMX email/password form and HTMX-swap timing; GridOps is a React/TanStack-Router SPA with no HTMX, same reasoning #141/#159's U rounds recorded). Viewports: 1280×900 (desktop — the runner row's destination column is `hidden … xl:block`, so ≥1280 is required to see it) and 360×800/900 (mobile).
- **Cleanup:** `pkill -f target/debug/gridops-api` (confirmed down — `curl` returns no HTTP response/`000`), throwaway SQLite DB and its `-shm`/`-wal` files removed. `target/` and `dist/` are reusable build artifacts (not session state) and were left in place.
- **State safety:** no shared/account-wide setting was touched — a dedicated throwaway DB and admin user, created for this pass only, not the shared operator account.

## State matrix walked

| # | Page | Case | Reached via | Result |
|---|---|---|---|---|
| 1 | `/` (Overview) | dashboard pool rows: repository, organization, GitLab, unknown | **Hard load** (`page.goto`) | PASS — "repository · ephemeral" (web-ci), "organization · ephemeral" (org-pool), "repository · ephemeral" (bb-pool, unchanged), "GitLab · ephemeral" (gl-pool), "Unknown · ephemeral" (odd-pool) |
| 2 | `/` → `/runners` | — | **SPA nav** (sidebar "Runners" link; `page.getByRole('link',{name:'Runners',exact:true})`; URL changed with no reload) | PASS — "Runners 8" heading, same destination text/title as the hard-load capture below |
| 3 | `/runners` list, right-hand column (xl, 1280px) | R1 GitHub repository | SPA | PASS — text/title `acme-labs/web` |
| 4 | same | R2 GitHub organization (no repository) | SPA | PASS — text/title `acme-labs` (installation login fallback) |
| 5 | same | R3 Bitbucket on GitHub-scoped pool | SPA | PASS — text/title `acme-labs/web`, **identical to R1** — resolves to GitHub, CI-platform badge still says "Bitbucket" |
| 6 | same | L1 GitLab, connection with a name | SPA | PASS — text/title `Acme / web` |
| 7 | same | L2 GitLab, connection with no name, full address | SPA | PASS — text/title `https://gitlab.example.com/acme/web` (scheme kept) |
| 8 | same | L2b GitLab, connection with no name, base URL only | SPA | PASS — text/title `https://gitlab.example.com` — **no stray trailing slash** |
| 9 | same | L3 GitLab, no connection at all | SPA | PASS — text/title `GitLab` — **not** the installation login (`acme-labs`), confirming "never the installation login" |
| 10 | same | U1 unknown scope (`quantum`) | SPA | PASS — text `—`, title `Unknown destination` — raw scope `quantum` never rendered anywhere |
| 11 | `/runners`, narrow line (<md, resized to 360px) | all 8 cases | resize (same loaded page) | PASS — identical strings to the right-hand column for every case (captured via `querySelectorAll('.text-2xs...md\\:hidden')`) |
| 12 | `/runners`, details sheet "Repository" row | L3 (neither) | click "Show details for runner-7f3g" | PASS — "Repository: GitLab", label unchanged ("Repository") |
| 13 | `/runners`, details sheet "Repository" row | U1 (unknown) | click "Show details for runner-7f3h" | PASS — "Repository: —", label unchanged |
| 14 | `/runners` → `/runner-pools/pool-gl` | GitLab pool detail, runner rows (right-hand column) | **SPA nav** (click "gl-pool" link in a runner row; URL changed with no reload) | PASS — same 4 GitLab destination strings as the `/runners` list (L1/L2/L2b/L3), confirming the row renders identically inside the pool-detail page |
| 15 | `/runner-pools/pool-gl` | same | **Hard load** (`page.goto`) | PASS — byte-identical destination text/title to the SPA-nav capture at row 14 — **no SPA-nav-vs-hard-load discrepancy** |
| 16 | `/runner-pools/pool-gl`, Destination sidebar group | GitLab pool, two connections (`gl-named`, `gl-addr`) | hard load | PASS — "Scope: GitLab", both connection chips shown — **unaffected #159 behavior**, confirming no regression from #169's changes |
| 17 | `/` (hard reload) | dashboard scope text | **Hard load** | PASS — identical to row 1's SPA-rendered values, confirming SPA-nav-vs-hard-load parity for the dashboard too |
| 18 | `/runner-pools` list | regression check | hard load | PASS — list loads cleanly, no console errors (confirms #159's destination-column surface is unaffected by #169's diff) |
| 19 | 360px mobile | `/runners` list | resize | PASS — screenshot `u-runners-360.png`; no horizontal overflow (`scrollWidth === clientWidth`) |
| 20 | 360px mobile | `/` dashboard | resize + hard reload | PASS — screenshot `u-dashboard-360.png`; no overflow; scope text correctly hidden below `sm` (pre-existing, untouched rule) |
| 21 | 360px mobile | `/runner-pools/pool-gl` detail | resize + hard reload | PASS — screenshot `u-pool-gl-detail-360.png`; no overflow; narrow-line destinations for all 4 GitLab runner rows match the desktop right-hand column text exactly |
| 22 | API-only (no drawn UI, per wireframe section D) | `totalRunners: 0` for an empty pool | `curl` against the live seeded server | PASS — `pool-empty` (no runners) returned `totalRunners: 0`; the other 5 seeded pools returned their live runner counts (1, 1, 1, 4, 1) |

**Zero/empty state:** mandatory-covered — L3/U1 are the "neither name nor address" and "unknown scope" fallbacks (rows 9–10, 12–13), and `totalRunners: 0` for a runner-less pool (row 22) is the explicit zero-count case the brief names.

**Gap:** R4 ("older server, no `poolScope`") could not be reproduced on a live server — the current server always emits the field. Covered only by the jsdom unit test (`tests/runner-row-platform.test.ts`), not independently re-verified live; flagged above, not treated as a live PASS.

**Console / state assertions:** `browser_console_messages` (warning level, `all: true`, cumulative across the whole session) returned **0 messages at every checkpoint** — dashboard (hard load and SPA-consistent read), `/runners` (SPA nav, every destination read, dialog open/close ×2, resize to 360px), GitLab pool detail (SPA nav, hard reload, resize), and `/runner-pools` list. No Alpine/HTMX concept applies to this stack (React/TanStack Router SPA, no HTMX); the "no dead component" pin was addressed by reading live DOM text/title directly (ground truth), not by framework introspection (React exposes none analogous to Alpine's data stack).

## Findings
None blocking.

1. All 9 wireframe cases (R1–R3, L1–L3, U1, plus the dashboard's 4 scope-text cases) render correctly, live, matching the approved wireframe's exact copy, in the row's right-hand column, the narrow line, and the details sheet.
2. GitHub and Bitbucket runner rows, and the dashboard's "repository"/"organization" scope text, are **unchanged** from the pre-#169 baseline — confirmed live (R3 is byte-identical to R1 at every one of the three render sites).
3. **Never the installation login for GitLab** — L3 (no connection at all) showed "GitLab", not the installation's `acme-labs` login, confirmed by direct comparison against R2's login-fallback text.
4. **No stray trailing slash** when a GitLab connection has only a base URL (L2b: "https://gitlab.example.com", not "https://gitlab.example.com/").
5. **Raw scope never echoed** for the unknown-scope case, in any of the three runner-row locations or the dashboard.
6. **No SPA-nav-vs-hard-load discrepancy** anywhere tested (rows 2 vs hard-load reads, 14 vs 15, 1 vs 17).
7. **No regression** to #159's own surfaces (`/runner-pools` list, the GitLab pool detail's Destination sidebar group) from this diff.
8. **No console error or warning** anywhere in the entire session.
9. `totalRunners: 0` for an empty pool confirmed live against the running server (API-only; no UI regresses because none exists for this field, per the wireframe).

## Evidence
Screenshots (session-scratch, saved under the worktree so this tool could write them):
- `docs/handoffs/169/screenshots/u-runners-1280.png` — `/runners` list at 1280px, all 8 runners with correct destination cells.
- `docs/handoffs/169/screenshots/u-runners-360.png` — `/runners` list at 360px, narrow-line destinations, no overflow.
- `docs/handoffs/169/screenshots/u-pool-gl-detail-1280.png` — GitLab pool detail at 1280px (runner rows + Destination sidebar group).
- `docs/handoffs/169/screenshots/u-pool-gl-detail-360.png` — same page at 360px, no overflow.
- `docs/handoffs/169/screenshots/u-dashboard-360.png` — dashboard at 360px, no overflow.

Live DOM captures (via `browser_evaluate`) of every runner row's right-hand column (`title`, `textContent`) and narrow line, every details-sheet "Repository" row, and the dashboard's scope text for all 5 seeded pools — see the state-matrix table above for the exact values captured.

`browser_console_messages` (warning level, `all: true`): `Total messages: 0 (Errors: 0, Warnings: 0)` at the final checkpoint, covering the entire session.

API sanity cross-check (`curl` against the live server, same seeded DB): `GET /api/v1/runners` returns `poolScope` and `gitlabConnectionName`/`gitlabBaseUrl`/`gitlabScopePath` exactly matching each seeded runner's state (null for GitHub/Bitbucket, populated per-case for GitLab); `GET /api/v1/runner-pools` returns `totalRunners` of 1/1/1/4/1/0 for the six seeded pools (including the empty one added after the browser pass) — confirming the UI's source data matches the brief's API contract and T's Rust test assertions.

## Verdict
**PASS** — every state the brief's Done-when and the approved wireframe specify (GitHub repository/organization, Bitbucket-on-GitHub-pool, GitLab name/address/one-address-part/neither, unknown scope, and the dashboard's four scope-text cases) was independently reproduced in a real headless browser, on both the SPA-navigation and hard-load paths, at desktop (1280px) and mobile (360px), with a clean console throughout, and no regression to #159's adjacent surfaces. The only gap is the "older server, no `poolScope`" case, which cannot be produced on a live server build and is covered only by T's jsdom test — not a defect, a live-reproduction limitation inherent to the case. Ready for S.
