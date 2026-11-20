# Handoff-U: Phase 8 - UI Walkthrough for one Connections page and one Targets picker for every CI platform (#145)

**Date:** 2026-11-20
**Branch:** claude/ui-145-platform-neutral-cf5832 (on top of `c2e91ac`, A-dup's commit)
**Issue:** #145
**Handoff-T-green reviewed:** `docs/handoffs/145/handoff-T-green.md` (PASS, 445/445 tests, no blockers)
**Handoff-F reviewed:** `docs/handoffs/145/handoff-F.md`
**Approved wireframe:** `docs/handoffs/145/wireframe.html` + `docs/handoffs/145/handoff-D.md` (revision 4)

## T precondition
Confirmed: T-green returned no blockers. T's "Known issues" explicitly named the surfaces that only a live browser can check (per-kind picker load states, the Add connection dialog's Back button, the runner details "CI platform" link, `/settings/connections` reached by SPA navigation, the three old-URL redirects) — all walked below.

## Backend note — this story is frontend-only, U still needed a real server
This repo is a Rust API (`gridops-api`) + TanStack Router SPA, authenticated via GitHub OAuth with no built-in bypass. Per my role ("you own serving the app"), I built `gridops-api` from source (`cargo build -p gridops-api`, debug profile) and ran it against a throwaway SQLite DB, with the frontend built via `npm run build` and served statically by the API (`GRIDOPS_WEB_ROOT`). To authenticate without real GitHub OAuth, I minted a session the same way the repo's own Rust integration-test fixture does (`crates/gridops-api/tests/control_plane_contracts.rs::ApiFixture::session_cookie`): inserted a `users`/`sessions` row directly and HMAC-signed the session token with the same `GRIDOPS_SESSION_SECRET` the server was started with, then set the resulting `gridops_session=<token>.<signature>` cookie via `page.evaluate` before navigating (the cookie is `HttpOnly` from the server, but a script-set cookie of the same name is still sent on subsequent requests, which is all a session check needs). This is not new test infrastructure — it mirrors the project's own existing fixture pattern, not an invention of mine.

## Wireframe conformance
Walked against `docs/handoffs/145/wireframe.html` revision 4 and `handoff-D.md`'s screen list (A-G). Matches on every frame I could reach live:
- **A (redirects).** `/settings/integrations`, `/settings/github`, `/platform-connections` and `?appError=` all land on `/settings/connections` (replace), as specified.
- **B (Connections page).** B1 (many: GitHub org row + Bitbucket row, badges, status, actions), B3→B4 equivalent (member read-only: one "Read-only access" callout, no Add/Edit/Remove, GitHub App section with checks/endpoints only, no Create/Manage App), B5 (per-kind load states), B6 (inline edit, blank-token-keeps-current), B7 (App-not-configured Create form) all matched.
- **C (Add connection + reveal).** C1 chooser, C3 Bitbucket fields + Back + Escape-to-close, idle/submitting/error states (invalid-token 400 surfaced inline as "Could not connect — Bitbucket API token is invalid.") all matched. C4 (post-create reveal dialog) was not exercised live — T's suite already drives it with a test-only descriptor (jsdom); doing so live would require a successful Bitbucket create, which needs a real Bitbucket token this environment cannot provide. Not a gap in F's work, a gap in what this environment can originate.
- **D (new-pool Targets row).** D2/D2b (Bitbucket disabled-with-reason "Select a GitHub repository or runner group first" until a GitHub target is picked), D3 (GitHub repository group: error+Retry; GitHub runner-group: manual-entry fallback row "Runner groups could not be loaded"), D4 (manual "Enter runner group ID" input, seeded from the default), D5b ("Install the GitHub App" when the signed-in user has zero visible installations), D7 (remove the last GitHub target while a Bitbucket workspace stays selected → workspace stays selected, Create pool stays enabled, submit sends no `POST /api/v1/runner-pools` and shows the thrown message in "Could not create the pool") all matched exactly as specified, including the auto-add of the Tart provider and the label-rule/placeholder swap to Bitbucket's rules.
- **E (pool settings Targets row).** E1 (Bitbucket shown as a locked "Kept as it is" chip, no remove control, in the editable settings form) and E3 (read-only pool: "Destination" with one line per platform — "GitHub · u-walk-org" and "Bitbucket · u-walk-bitbucket-renamed") both matched.
- **F (Platform options).** Collapsed by default, expands to the Bitbucket capacity note / label rule / Tart image script, badge in the header names the targeted platform(s).
- **G (Runner row).** G1 badges ("GitHub"/"Bitbucket" text badges next to each runner) and G2 (details sheet "CI platform" row, with a working external link when the descriptor's link builder returns one, and no link when it returns `null`) both matched.

No wireframe divergence found in anything reachable from this environment (the Add-connection chooser offering a disabled "GitHub" link-out entry — D's open question 6's default — is not implemented; see Advisory notes, not a REWORK item).

## Run environment
- `cargo build -p gridops-api` (debug profile), `npm run build` (dist already fresh from F/T).
- Served `./target/debug/gridops-api` on `http://127.0.0.1:18080`, `GRIDOPS_WEB_ROOT=<worktree>/dist`, `GRIDOPS_DATABASE_PATH=/tmp/u-walk-gridops/gridops.sqlite` (throwaway, deleted after the run), `GRIDOPS_SESSION_SECRET`/`GRIDOPS_ENCRYPTION_KEY`/`GITHUB_CLIENT_ID`/`GITHUB_CLIENT_SECRET`/`GITHUB_WEBHOOK_SECRET` set to placeholder values (no real GitHub App — see Known environment limits below), `GRIDOPS_MANAGER_URL=http://127.0.0.1:9` (unused).
- Seed (all direct SQLite inserts into the throwaway DB, removed with it): one admin user (`u-walk-admin`), one GitHub installation (`u-walk-org`, organization) with an admin `user_installations` grant and an AES-GCM-encrypted (fake) `access_token` so the vault's `open()` call succeeds, one Bitbucket connection (`u-walk-bitbucket`), one organization-scope runner pool (`u-walk-org-pool`) with that Bitbucket connection attached, two runners (one GitHub with a repository + `github_runner_id` for the CI-platform-link case, one Bitbucket), one repository row.
- Login: a session cookie minted the same way the repo's own `ApiFixture::session_cookie` test helper does (HMAC-SHA256 of a token with the server's session secret), set via `page.evaluate(() => document.cookie = …)` before navigating — see "Backend note" above.
- Playwright MCP: the browser binary (`chrome-for-testing`) was not pre-installed in this session; I ran `npx @playwright/mcp install-browser chrome-for-testing` once (one-time browser download, not a repo/package change) before driving.
- Viewports: desktop (default) and 360×800 (`browser_resize`).
- Cleanup: killed the `gridops-api` process and deleted `/tmp/u-walk-gridops/` entirely after the run; `target/` and `dist/` are build artifacts, left in place (not test state).

## State matrix walked
All reached via SPA navigation (TanStack Router `Link`/nav clicks), with hard-reload (`page.goto`) used at least once per page as the spot-check (the three redirect routes and `?appError=` are necessarily hard-navigations by nature; the Connections page, new-pool form, pool settings, runner list and runner-pool overview were each reached both by a nav-link click and by a direct `goto` — no discrepancy observed between the two paths, console state and rendered content were identical).

**Connections page (`/settings/connections`)**
| Control | States | Result |
|---|---|---|
| GitHub row | one (1 installation) | PASS — badge, scope text "Organization", "Selected repositories · 0 pools" → "· 1 pool" after a pool was seeded against it (copy stayed truthful), external Configure link correct |
| Bitbucket row | one | PASS — badge, workspace text, "Verified Nm ago", Edit/Remove |
| Add connection | chooser → Bitbucket form → Back → chooser again → Escape closes | PASS, each step confirmed by snapshot |
| Add connection, submit empty | zero (required fields unfilled) | PASS — no request fired (native required validation), no console error |
| Add connection, submit invalid token | error | PASS — 400 surfaced inline as "Could not connect — Bitbucket API token is invalid.", dialog stays open, fields retained |
| Edit Bitbucket connection | one, rename | PASS — inline edit shows only editable-on-update fields (name, "New API token" with "leave blank to keep"), Save → list updates, toast "Connection updated." |
| Remove Bitbucket connection (in use by a pool) | error (FK conflict) | PASS — confirm dialog warns "Pools that use it must be changed first.", on confirm the server returns 409, toast shows the specific message ("…is used by pool u-walk-org-pool. Remove it from that pool first."), connection is NOT removed from the list |
| Member (read-only) view | — | PASS — "Read-only access" callout, no Add/Edit/Remove, GitHub App section limited to checks+endpoints, no Create/Manage App |
| Mobile (360px) | — | PASS — no overflow, rows stack, truncation is graceful |

**New-pool form (`/runner-pools/new`)**
| Control | States | Result |
|---|---|---|
| Targets picker — GitHub · Repository | error (list load 500 once the vault/token plumbing was fixed — see note) | PASS — "Couldn't load repository options." + Retry; Retry re-fires the request (confirmed via console/network) |
| Targets picker — GitHub · Organization runner group | error/fallback (list fails) | PASS — manual "Enter runner group ID" option with "Runner groups could not be loaded" reason, selectable |
| Targets picker — manual runner-group entry | one | PASS — chip appears, "Runner group ID" spinbutton (default 1) appears, label "u-walk-org · Enter runner group ID" |
| Targets picker — Bitbucket · Workspace | zero-GitHub-selected (disabled) → one-GitHub-selected (enabled) | PASS — disabled with reason "Select a GitHub repository or runner group first" until a GitHub target is picked, then selectable; picking it auto-adds the Tart provider |
| Platform options section | collapsed → expanded | PASS — collapsed badge "Bitbucket", expands to the capacity note, label-rule note and Tart image script path |
| Additional labels help/placeholder | GitHub-only vs Bitbucket-present | PASS — text and placeholder swap to Bitbucket's rules only once a Bitbucket target is selected |
| D7: remove last GitHub target, Bitbucket stays selected | — | PASS — workspace chip stays, Create pool stays enabled; submit sends no `POST /api/v1/runner-pools` (confirmed via network log), shows "Could not create the pool — Invalid input" |
| Create-pool gate, zero installations | zero/empty | PASS — whole form is replaced by "Install the GitHub App" empty state (D5b copy), confirmed by temporarily removing the seeded `user_installations` grant and reloading |
| Mobile (360px) | — | PASS — Targets picker opens with its search box, no overflow |

**Pool settings (`/runner-pools/pool-org/settings`)**
| Control | States | Result |
|---|---|---|
| Targets row (admin/editable) | organization pool, one | PASS — GitHub runner-group chip is removable ("Remove u-walk-org · Runner group 1"), Bitbucket chip is locked ("Kept as it is", no remove control) |
| Read-only view (member/no-manage) | one | PASS — "Destination" shows one line per platform: "GitHub · u-walk-org" and "Bitbucket · u-walk-bitbucket-renamed" |

**Runners (`/runners`, pool overview)**
| Control | States | Result |
|---|---|---|
| CI platform badge on row | GitHub, Bitbucket | PASS — correct text badge per platform |
| Details sheet "CI platform" row | GitHub with repository+`github_runner_id` (link), Bitbucket (no link) | PASS — GitHub renders a working external link (`https://github.com/<owner>/<repo>/settings/actions/runners/<id>`), Bitbucket renders plain text "Bitbucket" (its `runnerLink` is `() => null` by design) |

## Findings
None blocking. Two advisory, non-blocking notes:

1. **Pre-existing console error, not introduced by #145.** Submitting either pool form logs `Pattern attribute value [a-z0-9][a-z0-9-]*[a-z0-9] is not a valid regular expression … Invalid character class` from the Pool-name input's HTML5 `pattern` attribute (Chromium's newer regex engine rejects an unescaped `-` inside `[a-z0-9-]` under its `v`-flag validation path). Verified by `git show ef65745:src/routes/_app/runner-pools.new.tsx | grep pattern=` — the exact same `pattern="[a-z0-9][a-z0-9-]*[a-z0-9]"` string exists at the commit immediately before F started, on a line F did not touch (confirmed no hunk on that line in `git diff ef65745 HEAD -- src/routes/_app/runner-pools.new.tsx`). It has no functional effect observed: form submission still proceeded to the app's own (zod) validation and produced the correct truthful error/success state in every case I drove. Flagging for the backlog, not for this story's F.
2. **Add-connection chooser does not list GitHub as a disabled link-out option.** `handoff-D.md`'s open question 6 named a default (GitHub appears in the chooser as a disabled, pointing-copy entry when the App isn't configured) but F's `connections-section.tsx` filters the chooser to `descriptors.filter(d => d.connections.create)`, and the GitHub descriptor has no `create` binding (by design — see the brief's verified facts: "GitHub has no connection create/update/remove function"), so it is simply absent from the chooser rather than shown disabled. T's own `connections-page.test.ts` only exercises the Bitbucket chooser path, so this was never asserted either way. Installing another GitHub account is still reachable (via the GitHub App section's own Install action, once the App is configured), so nothing is unreachable — this is a wireframe-vs-build gap on a documented "default", not a behavioral defect. Not blocking; noting for S/the record.

## Evidence
Screenshots and snapshots were captured through the session's Playwright MCP tool calls (desktop + 360px, Connections page, Add-connection flow, Edit/Remove, new-pool Targets picker states, D7 state, pool settings editable + read-only, runner details sheets). Representative console/network excerpts are inlined above (400 on invalid Bitbucket token, 409 on in-use Bitbucket removal, 500→Retry on the GitHub lists once a real-shaped encrypted access token made the server reach a real (rejecting) GitHub API instead of erroring on `invalid encrypted value`, no `POST /api/v1/runner-pools` on the D7 case). No screenshot files were saved to the project tree; they live in this session's tool-output directory only.

## Verdict
**PASS** — every control I could reach in a real headless browser behaved correctly on the SPA-navigation path (reconfirmed on hard reload with no discrepancy), matched the approved wireframe/handoff-D, and the console showed only expected HTTP-error logs for conditions I deliberately created (invalid token, in-use removal, unreachable fake GitHub credentials) plus one pre-existing, unrelated, no-functional-impact console warning (see Advisory note 1). Ready for S.
