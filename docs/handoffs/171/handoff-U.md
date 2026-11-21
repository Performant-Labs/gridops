# Handoff-U: Phase 8 - UI walkthrough for #171 (+ #174) GitLab token scopes and deletion warnings

**Date:** 2026-11-21
**Branch:** claude/gitlab-171-scopes-cbe8b5 (head 665c126)
**Issue:** #171 (+ #174)
**Handoff-T-green reviewed:** docs/handoffs/171/handoff-T-green.md (GREEN)
**Handoff-F reviewed:** docs/handoffs/171/handoff-F.md
**Approved wireframe:** docs/handoffs/171/wireframe.html

## T precondition
Confirmed. `handoff-T-green.md` is GREEN with no blockers. `git diff --stat HEAD -- src crates` is empty, so the working tree matches the commit T and A-dup verified; I rebuilt both artifacts from this exact tree (`cargo build -p gridops-api`, `npm run build`) rather than trust stale binaries, and both builds were clean.

## Wireframe conformance
No divergence. Both changed screens were reproduced live and matched the approved wireframe exactly.

- **Screen 1a (Add connection, GitLab)** — opened the "Add connection" dialog, selected GitLab: the Access token field's help paragraph reads verbatim "The token needs the create_runner, manage_runner and read_api scopes (or api), and its user needs the Maintainer role or higher on a project, or the Owner role on a group. GridOps checks both before saving." — the exact wireframe text, one short paragraph, plain text (no inline code), matching F's NIT-2 decision to keep it unsplit.
- **Screen 1b (Edit connection, token replacement)** — opened Edit on the seeded `gridops-e2e` connection: the same help paragraph renders under "New access token", byte-identical to 1a.
- **Screen 1c (refusal states)** — **not independently reproduced live.** `verify_access` calls the real GitLab HTTP API (`crates/gridops-core/src/gitlab/http.rs`), so a live 400 "missing the manage_runner scope" toast can only be produced against a real GitLab token/project or a stubbed GitLab endpoint; neither was available in this offline walkthrough. This is a live-reproduction limitation of the case, not a UI defect — the message format is fixed server-side and is independently covered by T's `http_tests.rs` scope table and the API tests in `gitlab_tests.rs` (create / token replacement / Verify, per the brief's Done-when). Flagged as a gap, consistent with #169's U pass treatment of its own live-reproduction limitation.
- **Screen 2a (Events list, many — includes deletion warnings)** — seeded one per-runner warning (runner_id set, "a1b2c3d4" runner link), one sweep warning (runner_id NULL, "—" in the Runner column), and one unrelated "runner registered" info row. The pool's "Recent activity" table and its "All activity" page both render exactly as drawn: amber dot for warnings (implicit via row styling), event name "GitLab runner not deleted", the full message text, the Runner cell as a link to live logs or "—", and relative time. Byte-identical between SPA nav and hard reload (see State matrix row 7/8).
- **Screen 2b (one warning, expanded)** — clicking a warning row expands it and shows the metadata exactly as drawn: `{"connectionId":"gl-e2e","connection":"gridops-e2e","gitlabRunnerId":59802498,"reason":"missing_scope:manage_runner"}` — ids and a reason code only, no token, matching the wireframe's note that exact keys are F's call (the tests assert the message, not the metadata shape).
- **Screen 2c (narrow viewport)** — resized to 360px: Details and Runner columns hide, the message text appears inline when the row is expanded — matching the wireframe's "existing pattern" note exactly.
- **Screen 2d (empty / loading / error, unchanged)** — the runner-less, event-less pool (`pool-gl-empty`) shows "No lifecycle events recorded yet." (the pool's existing empty-events copy, unchanged) and "No runners right now" — confirming a healthy pool's events list is unaffected by this change, as the wireframe states.

No control the wireframe specified was missing, and no observed copy or behavior diverged from the approved target, with the one noted live-reproduction gap (1c).

## Run environment
GridOps has no e2e seed script and its only login is GitHub OAuth (same situation as #141/#145/#159/#169's U rounds), so this pass built and served the real Rust API against a throwaway SQLite DB, seeded directly, with a hand-minted session cookie (same technique as #159/#169's U rounds):

- **Build:** rebuilt both artifacts from the current tree, even though `git diff --stat HEAD -- src crates` was already empty: `CARGO_BUILD_JOBS=2 cargo build -p gridops-api` (clean) and `npm run build` (vite build + `tsc --noEmit`, clean).
- **Binary location:** this workspace's cargo builds share a target dir at `/Users/andreangelantoni/.cache/cargo-target/gridops/debug/` (not `./target`), so the server was launched from `.../cargo-target/gridops/debug/gridops-api`.
- **Serve:** on `127.0.0.1:19371`, with `GRIDOPS_DATABASE_PATH=<scratch>/u171.sqlite`, `GRIDOPS_BASE_URL=http://127.0.0.1:19371`, `GRIDOPS_API_BIND=127.0.0.1:19371`, `GRIDOPS_WEB_ROOT=<worktree>/dist`, `GRIDOPS_SESSION_SECRET`/`GRIDOPS_ENCRYPTION_KEY` (random, local-only, generated with `openssl rand -base64 32`), `GRIDOPS_FLEET_ALLOW_LOOPBACK_HTTP=true`, `GITHUB_CLIENT_ID`/`GITHUB_CLIENT_SECRET` (dummy values, only so startup validation passes — no GitHub API call was made).
- **DB:** migrated automatically on first connection (`sqlx::migrate!`), then seeded via a throwaway Node+sqlite3 script (`<scratch>/seed.mjs`, modeled on #159/#169's seed scripts) with:
  - one admin user + session;
  - one GitHub installation (`acme-labs`, organization, needed because `runner_pools.installation_id` is `NOT NULL`);
  - one GitLab connection `gl-e2e` ("gridops-e2e", `https://gitlab.com/acme/mobile-app`), matching F's seed SQL in handoff-F's Rework section;
  - a GitLab pool with events (`pool-gl-e2e`) and a GitLab pool with none (`pool-gl-empty`), both linked to `gl-e2e` via `runner_pool_gitlab_connections`;
  - one runner row (`gl-runner-1`, offline, `gitlab_runner_id=59802498`) on `pool-gl-e2e`, so the per-runner event has a real runner to link to (first 8 characters of the id, "a1b2c3d4");
  - three `runner_events` rows on `pool-gl-e2e`: a per-runner "GitLab runner not deleted" warning (`runner_id` set, metadata `{"connectionId":"gl-e2e","connection":"gridops-e2e","gitlabRunnerId":59802498,"reason":"missing_scope:manage_runner"}`, using the exact message text from F's own seed SQL in handoff-F), a sweep "GitLab runner not deleted" warning (`runner_id` NULL, a different `gitlabRunnerId` so it's visibly a second row), and an unrelated "runner registered" info event (green/default level) to confirm non-warning rows are unaffected.
- **Login:** session cookie minted with the vault's own `HMAC-SHA256(GRIDOPS_SESSION_SECRET)` scheme (`crates/gridops-core/src/crypto.rs::Vault::sign`/`verify`, `crates/gridops-api/src/auth.rs::SESSION_COOKIE = "gridops_session"`, cookie value `{token}.{sign(token)}`), set via `browser_evaluate` `document.cookie` on `127.0.0.1` (host match), then confirmed with a load of `/` (authenticated shell: sidebar, "Control plane online", the seeded pools visible on the dashboard).
- **Driver:** Playwright MCP, headless Chromium, direct `mcp__playwright__*` tool calls — not `u-drive.mjs` (that helper assumes an HTMX form and HTMX-swap timing; GridOps is a React/TanStack-Router SPA with no HTMX, same reasoning as #141/#159/#169's U rounds). Viewports: 1280×900 (desktop) and 360×800 (mobile).
- **Cleanup:** `pkill -f gridops-api` for this session's PID (confirmed down — `curl` to `127.0.0.1:19371` returns `000`/no response); the scratch SQLite DB, the session secret and the encryption key were deleted. `target/`/`dist/` are reusable build artifacts (not session state) and were left in place.
- **State safety:** no shared/account-wide setting was touched. A dedicated throwaway DB and admin user were created for this pass only, not the shared operator account. The one global-settings page visited (`/settings/general`) was viewed only, no field was changed or saved. The "Add connection" dialog was opened and the GitLab form inspected, then closed with Escape with nothing submitted — no connection was added, no existing connection's token was replaced, and the existing `gridops-e2e` connection's Edit dialog was opened then Cancelled, leaving it unchanged.

## State matrix walked

| # | Page | Case | Reached via | Result |
|---|---|---|---|---|
| 1 | `/settings/connections`, Edit `gridops-e2e` | Screen 1b: token-replacement help text | SPA nav (Settings → Connections → Edit) | PASS — help paragraph matches wireframe verbatim |
| 2 | `/settings/connections`, Add connection → GitLab | Screen 1a: add-connection help text | SPA nav (dialog) | PASS — same help paragraph, byte-identical to 1b |
| 3 | `/settings/connections` | refusal toast (1c) | N/A | **Gap** — requires a live or stubbed GitLab API call; not reproduced live. Covered by T's `http_tests.rs`/`gitlab_tests.rs` per the brief. |
| 4 | `/` → `/runner-pools/pool-gl-e2e` (Overview) | Screen 2a: many events incl. 2 warnings + 1 info | SPA nav (dashboard pool link) | PASS — "GitLab runner not deleted" rows (one with runner link, one "—"), "runner registered" info row, correct messages and relative times |
| 5 | `/runner-pools/pool-gl-e2e`, expand row | Screen 2b: one warning, expanded | SPA nav (click row) | PASS — metadata shown exactly: `{"connectionId":"gl-e2e","connection":"gridops-e2e","gitlabRunnerId":59802498,"reason":"missing_scope:manage_runner"}` — no token |
| 6 | `/runner-pools/pool-gl-e2e`, 360px | Screen 2c: narrow viewport | resize (same loaded page) | PASS — Details/Runner columns hidden; message text shown inline on an expanded row |
| 7 | `/runner-pools/pool-gl-e2e` | Events table | **Hard load** (`page.goto`) | PASS — byte-identical to row 4's SPA-nav capture (same event rows, same cell text) — **no SPA-nav-vs-hard-load discrepancy** |
| 8 | `/runner-pools/pool-gl-e2e/activity` | full Activity page | SPA nav (click "All activity") | PASS — same 3 rows, same message text, as the pool overview's "Recent activity" panel |
| 9 | `/runner-pools/pool-gl-empty` | Screen 2d: empty events (unchanged) | Hard load | PASS — "No lifecycle events recorded yet." (existing empty-events copy) and "No runners right now"; confirms a healthy pool's events list is unaffected by this diff |

**Zero/empty state:** mandatory-covered — row 9 is the explicit empty-events case the wireframe names ("Empty: the pool's existing empty-events copy ... unchanged").

**Error state:** not independently reproduced (same reasoning as row 3 — the error copy "Pool activity could not be loaded." is an existing, unchanged surface the wireframe explicitly marks as "existing states, unchanged"; this diff writes new warning rows into the existing table, it does not touch the loading/error rendering path).

**Gap:** row 3 (refusal toast, Screen 1c) could not be reproduced on a live server without a real GitLab token or a stubbed endpoint. The message format is fixed server-side (unchanged by this diff's text, only the triggering scope list changed) and is independently pinned by T's Rust tests per the brief's Done-when (`http_tests.rs` scope table; `gitlab_tests.rs` 400 "missing the manage_runner scope" on create/replace/Verify). Flagged as a gap, not treated as a live PASS — consistent with how #169's U pass flagged its own un-reproducible case (R4).

**Console / state assertions:** `browser_console_messages` (warning level, `all: true`, cumulative across the whole session) returned **0 messages at every checkpoint** — Overview (hard load), Settings → Connections (dialog open/close ×2), pool detail (SPA nav, hard reload, resize to 360px, row expand), pool Activity page (SPA nav), empty pool (hard load). No Alpine/HTMX concept applies to this stack (React/TanStack Router SPA, no HTMX); dead-component risk was addressed by reading the live rendered DOM text directly (ground truth) via `browser_snapshot`, not framework introspection (React exposes none analogous to Alpine's data stack).

## Findings
None blocking.

1. **Help text matches the approved wireframe exactly**, in both the Add-connection dialog (1a) and the Edit-connection dialog (1b): all three scopes named (`create_runner`, `manage_runner`, `read_api` or `api`), Maintainer-or-higher on a project / Owner on a group, one short plain-text paragraph, "GridOps checks both before saving."
2. **The reconciler's new per-runner and sweep deletion warnings render correctly** in the pool's existing Events/Activity surface: amber-level "GitLab runner not deleted" rows with the exact message text (connection name, GitLab runner id, client-safe reason, remedy sentence — never a token), a Runner-column link when a runner row exists and "—" when it does not (sweep case), both confirmed live against the running server and seeded data, in both the "Recent activity" panel and the full "All activity" page.
3. **No new component, column or control was introduced** — the existing `PoolEventTable`/activity list renders the new warning rows with no code change needed on the frontend, exactly as the wireframe's "no new component" note states.
4. **Metadata expansion never shows a token** — confirmed by reading the expanded row's rendered JSON directly: only `connectionId`, `connection`, `gitlabRunnerId`, `reason`.
5. **Narrow viewport (360px) behaves per the existing pattern** — Details/Runner columns hide, message text appears on row expansion, matching Screen 2c exactly.
6. **A healthy pool's events list is unaffected** — the empty-events pool shows the pre-existing "No lifecycle events recorded yet." copy, confirming the new warning-writing code paths do not alter the empty state.
7. **No SPA-nav-vs-hard-load discrepancy** anywhere tested (row 4 vs row 7).
8. **No console error or warning** anywhere in the entire session.
9. **No account-wide or instance-wide state was mutated** — Settings → General was viewed only; the Add-connection dialog and the existing connection's Edit dialog were both opened to inspect the help text, then closed/cancelled with nothing submitted, leaving the seeded `gridops-e2e` connection and all global settings exactly as seeded.

**Gaps (not defects):**
- Screen 1c (the refusal toast on a token missing `manage_runner`) could not be reproduced live — it requires a real GitLab API round-trip. Covered only by T's Rust/API tests, not independently re-verified in the browser. Noted per the brief's own acknowledgment that `verify_access` calls the live GitLab API.
- The existing error state ("Pool activity could not be loaded.") is explicitly unchanged by this diff and was not independently reproduced; it is pre-existing surface, not a new behavior this issue introduces.

## Evidence
Screenshots (saved under the worktree so this tool could write them):
- `docs/handoffs/171/screenshots-u/pool-events-360.png` — `/runner-pools/pool-gl-e2e` at 360px, with an expanded warning row showing the message and metadata, Details/Runner columns hidden per the existing narrow-viewport pattern.

Live DOM captures (via `browser_snapshot`) of:
- the Add-connection and Edit-connection GitLab forms' Access-token help paragraph (verbatim text captured above);
- the pool Events table (desktop, both via SPA nav and hard reload) and the full Activity page, all three seeded rows with exact message text and Runner-column behavior;
- the expanded warning row's metadata JSON;
- the empty-events pool's "No lifecycle events recorded yet." copy.

`browser_console_messages` (warning level, `all: true`): `Total messages: 0 (Errors: 0, Warnings: 0)` at the final checkpoint, covering the entire session.

## Verdict
**PASS** — every UI surface this diff changes (the connection form's token help text in both Add and Edit, and the pool Events/Activity list's rendering of the new deletion-warning rows, including the per-runner vs. sweep Runner-column distinction, metadata expansion with no token, the narrow-viewport pattern, and the unaffected empty state) was independently reproduced in a real headless browser, on both the SPA-navigation and hard-load paths where applicable, at desktop (1280px) and mobile (360px), with a clean console throughout and no mutation of shared account-wide state. The only gap is the live refusal-toast case (Screen 1c), which requires a real GitLab API round-trip this offline walkthrough cannot make and is independently covered by T's Rust/API test suite per the brief's Done-when — not a defect, a live-reproduction limitation inherent to the case, exactly as #169's U pass treated its own analogous gap. Ready for S.
