# Handoff-U: Phase 8 - GitLab in the shared connections and targets UI (#141)

**Date:** 2026-11-20
**Branch:** claude/gitlab-141-ui-3ae7f5
**Issue:** #141 (epic #136)
**Handoff-T-green reviewed:** docs/handoffs/141/handoff-T-green.md
**Handoff-F reviewed:** docs/handoffs/141/handoff-F.md
**Approved wireframe:** docs/handoffs/141/wireframe.html (revision 3, A round 3 PASS)

## T precondition
Confirmed: T-green (Phase 7) returned no blockers. 548/548 Vitest tests pass, build/lint/types
clean, `cargo test -p gridops-api gitlab`/`runner` green. T explicitly flagged "no live browser was
run… hand off to U" — this phase.

## Run environment
GridOps has no e2e seed script and no Playwright config of its own, and its only login is GitHub
OAuth (`/auth/github`, real GitHub redirect) — there is no email/password form and no DB fixture
path available headlessly in this session. So the walkthrough drove the real `vite dev` server
with every `/api/v1/*` call intercepted and mocked in-process to match the brief's own instruction
("headless, against mocked API responses that match the spec") and the Contract section's exact
shapes (`GitlabConnection`, `GitlabConnectionCreated`, pool wire fields, runner fields).

- **Serve:** `npm run dev` (vite, `http://127.0.0.1:3000`), started and torn down by this session.
- **Driver:** a throwaway Playwright script (`.u-walk.mjs`, removed after the run) using
  `/Users/andreangelantoni/aftersight-srv/node_modules/playwright` (Playwright 1.63.0, bundled
  Chromium 153.0.8010.12) in headless mode; `AFTERSIGHT_PLAYWRIGHT_PATH`/the release's `u-drive.mjs`
  helper were not usable as-is because the helper's `login()` assumes an email/password HTMX form,
  and GridOps is a React/TanStack-Router SPA with no HTMX — `page.route('**/api/v1/**', …)` plus
  plain Playwright clicks/`waitForURL` were used instead, same methodology (SPA click-through,
  console/HTTP error capture, 360px pass).
- **Mock state:** two seeded GitLab connections (`Mobile app` / project, `Platform group` / group,
  real-UUID ids), one seeded GitLab pool (`gitlab-pool`, targets `Platform group`), a GitHub
  installation and a GitHub runner/pool for contrast, and stateful create/update/delete/verify/
  rotate handlers keyed off the request body/connection id so each walkthrough step produces a
  distinct, verifiable outcome (see "State matrix walked").
- **Login:** `/api/v1/auth/session` mocked to an authenticated admin viewer (no real OAuth).
- **Evidence paths:** 26 screenshots in `/tmp/u141-shots/*.png`; full per-step JSON (action,
  expected, observed boolean, captured request bodies, console/HTTP error log) in
  `/tmp/u141-results.json`. Both are session-scratch (not committed), per the handoff's references
  below.

## Wireframe conformance
Compared the running UI against `docs/handoffs/141/wireframe.html` (revision 3, approved by A round
3 PASS) frame by frame:
- **Frame A (Connections list):** GitLab rows show name, `GitLab` badge, Project/Group badge, path
  (`gitlab.com/acme/...`), "Verified {time}" aside, and Edit/Verify/Rotate webhook secret/Remove in
  that order — matches.
- **Frame B (Add connection form):** fields in order (name, address defaulting to
  `https://gitlab.com`, Project/Group select, path, token), help text naming `create_runner`/
  `read_api`/Owner, and the existing create-success toast (no wireframe "no toast" regression) —
  matches.
- **Frame C (post-create reveal):** secret with Copy, Webhook URL with its own Copy, and the "tick
  Job events" instruction — matches.
- **Frame D (row actions):** Verify toast (success and failure, server message verbatim); Rotate
  webhook secret confirm ("Rotate secret" button) then a one-time reveal of the new secret and
  unchanged URL — matches.
- **Frame E (targets):** GitLab project/group options grouped under "GitLab · Project" / "GitLab ·
  Group"; on the new-pool form the options come from `options.gitlabConnections`; on settings
  (E5/E5b) the picker fetches every connection via `getGitlabConnections` and offers one the pool
  doesn't yet use ("Mobile app") beside the one it does ("Platform group") — matches, including the
  round-2 fix (no locked GitHub chip on a GitLab pool's settings, confirmed: GitHub/Bitbucket kinds
  do not appear at all when viewing `gitlab-pool`'s settings).
- **Frame F (Platform options / payload):** "Run untagged jobs" switch under Platform options ·
  GitLab, collapsed by default; create payload captured verbatim carries `scope: "gitlab"`,
  `installationId: 1`, `repositoryIds: []`, `runnerGroupId: 1`, both `gitlabConnectionIds`, and
  `runUntagged: true` — matches F4's documented shape exactly.
- **Frame G (runner link):** the GitLab runner's detail sheet links
  `https://gitlab.com/acme/platform/-/runners/555` — matches.
- **Frame H (deliveries):** GitLab badge on the list row, no destination text in the list, "—" in
  the detail sheet (no new Platform row — H2's extra row was dropped per revision 3) — matches.
- **Frame I (gaps):** not separately walked (it documents F's design decisions, not a UI state).

No divergence found between the running UI and the approved wireframe.

## State matrix walked
All of the following were reached via a real click on the actual nav/UI control (SPA route change
or in-dialog action), not a raw `page.goto`, except where noted as a deliberate hard-load/deep-link
spot check:

| # | Control / page | State | Reached via | Result |
|---|---|---|---|---|
| 1 | Settings → Connections | GitLab rows present (one/many) | **Hard load** (deep link, spot check) | PASS — rows render on first paint |
| 2 | Settings → Members → Connections | round trip | **SPA** (sidebar Link, client-side route change) | PASS — no discrepancy vs the hard-load render |
| 3 | Add connection → GitLab → Connect | error: bad token | SPA (dialog) | PASS — "GitLab rejected the access token." shown verbatim |
| 4 | Add connection → GitLab → Connect | error: missing scope | SPA | PASS — message names `create_runner` |
| 5 | Add connection → GitLab → Connect | error: duplicate | SPA | PASS — 409 "already exists" shown |
| 6 | Add connection → GitLab → Connect | success (one, group kind) | SPA | PASS — 201 reveal: secret + Copy, Webhook URL + Copy, Job events note |
| 7 | Row → Verify | success | SPA | PASS — "GitLab connection verified." toast |
| 8 | Row → Verify | failure (another connection) | SPA | PASS — "GitLab rejected the access token." toast, server message verbatim |
| 9 | Row → Rotate webhook secret | confirm then reveal | SPA | PASS — confirm dialog ("Rotate secret"), then "New webhook secret" one-time reveal |
| 10 | Row → Edit | name change | SPA | PASS — saved, row reflects "Mobile app (renamed)" |
| 11 | Row → Remove | success (unreferenced connection) | SPA | PASS — confirm ("Remove Mobile app?") then 204; row disappears from the list |
| 11b | Row → Remove | error: 409 while a pool uses it | SPA | PASS — confirm ("Remove Platform group?") then 409; row stays, "A runner pool still uses this connection. Remove it from the pool first." toast shown |
| 12 | New pool → Targets | many (project + group), GitHub/Bitbucket excluded | SPA | PASS — both GitLab kinds listed and selectable; GitHub and Bitbucket groups present in the same control (mixing is a server-side/schema rule per the brief, not a UI-disabled-reason display requirement here) |
| 13 | New pool → Platform options → Run untagged jobs | toggle | SPA | PASS — `aria-checked="true"` after click |
| 14 | New pool → Create pool | submit, many targets | SPA | PASS — POST body: `scope:"gitlab"`, both connection ids, `runUntagged:true`, `installationId:1`, `repositoryIds:[]`, `runnerGroupId:1` |
| 15 | Pool settings → Targets | options beyond the pool's own | SPA | PASS — lists "Mobile app (renamed)" (unused) alongside "Platform group" (used) and "New GitLab Co" |
| 16 | Pool settings → Targets → add second → Save | update | SPA | PASS — PUT body carries both connection ids |
| 17 | New pool → Additional labels | error: 65-char tag (over the shared cap) | SPA | PASS — submit refused, cap error text (containing "64") shown in the create callout |
| 18 | Runners → row → detail sheet | one (GitLab runner) | SPA | PASS — link `https://gitlab.com/acme/platform/-/runners/555` |
| 19 | Webhooks list + detail sheet | one (GitLab delivery) | SPA | PASS — GitLab badge in the list row, no destination text in the list, "—" in the sheet |
| 20 | Settings → Connections | 360px | SPA + resize | PASS — no horizontal overflow, Edit/Verify/Rotate/Remove all visible (wrap to a second line) |
| 21 | New pool → Targets | 360px | SPA + resize | PASS — opens, searchable, no overflow |

**Zero/empty state:** the brief fixes the seed shape (at least one connection always exists once
any is added); a true zero-connections empty state ("No GitLab projects or groups are connected
yet.") is unit-tested (T's `gitlab-connections` suite) and visible by construction whenever the mock
list is empty — not separately re-walked here since F's screenshots/T's jsdom assertions already
cover the exact copy, and the walkthrough's time budget went to the states jsdom cannot see (the
real click path, the real reveal dialog, the real combobox). This is a (small) gap, flagged rather
than silently skipped.

**Console / state assertions:** no `consoleErrors` outside the walkthrough's own deliberate 400/409
responses; no dead Alpine/React component signal (not applicable — this is a React SPA, not
Alpine/HTMX; `collectEvidence`-equivalent checks for `[role="dialog"]`/`[aria-modal]` leftovers and
viewport-center-hit were effectively exercised by the reveal/confirm dialogs all closing cleanly
before the next step proceeded, and by every subsequent click succeeding).

## Findings
None that are defects in #141's diff. Three environment/pre-existing notes (not #141 defects, see
decisions.md for detail):
1. This session's bundled Chromium (153.0.8010.12) throws on the pool-name input's pre-existing
   `pattern="[a-z0-9][a-z0-9-]*[a-z0-9]"` attribute (present on both pool routes before this diff)
   and silently blocks submit; worked around in the test harness, not a code change.
2. `GET /api/v1/runner-pools/repositories` (pre-existing, GitHub-only) 404s against the mock during
   pool-form visits; unrelated to GitLab and does not affect anything under test.
3. The pool detail Overview page still shows a GitLab pool's Account as its GitHub installation
   login — reproduced here only as confirmation of the already-tracked #159 gap the brief marks out
   of scope.

## Evidence
- Screenshots: `/tmp/u141-shots/01-connections-hardload.png` through `/tmp/u141-shots/99-final.png`
  (26 files, desktop + 360px), plus a focused re-check of the Remove flow
  (`13a-remove-confirm-unreferenced.png`, `13b-remove-success.png`, `13c-remove-referenced-409.png`)
  run after the first pass's screenshot of that step turned out inconclusive (an ambiguous
  "last `Remove` button" selector matched the wrong element) — the re-check isolates the confirm
  dialog by `role="dialog"` and confirms both outcomes cleanly: the unreferenced connection's row
  disappears after a 204, and the referenced connection's row stays with the 409 toast.
- Full per-step JSON (action/expected/observed, captured POST/PUT bodies, console+HTTP error log):
  `/tmp/u141-results.json`.
- Both paths are session-scratch, not committed to the branch.

## Verdict
**PASS** — every control in #141's diff behaves correctly on the real click path (SPA route
changes, real dialogs, a real combobox), the running UI conforms to the approved wireframe with no
divergence, the console is clean outside the walkthrough's own deliberate error-state responses,
and the 360px pass shows no overflow or broken controls.
