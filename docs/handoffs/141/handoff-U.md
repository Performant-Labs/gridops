# Handoff-U: Phase 8 - GitLab in the shared connections and targets UI (#141)

**Date:** 2026-11-20 (re-walk, cycle 2, after S REWORK → F fix → T → A re-checks)
**Branch:** claude/gitlab-141-ui-3ae7f5 (HEAD `9441795`; F's row-action fix is `675a213`)
**Issue:** #141 (epic #136)
**Handoff-T-green reviewed:** docs/handoffs/141/handoff-T-green.md (cycle 2 re-verification, 2026-11-20)
**Handoff-F reviewed:** docs/handoffs/141/handoff-F.md ("Rework after S" section)
**Handoff-S reviewed:** docs/handoffs/141/handoff-S.md (REWORK, cycle 1)
**Handoff-A-dup reviewed:** docs/handoffs/141/handoff-A-dup.md (PASS)
**Approved wireframe:** docs/handoffs/141/wireframe.html (revision 3, A round 3 PASS)
**Prior U pass:** this file's cycle-1 content (superseded below) reported PASS before S's REWORK;
this re-walk targets exactly what S's REWORK item 2 asked of U ("re-walk 360 px using per-button
geometry and a hit test, not only page overflow") plus a spot check of the surrounding states.

## T precondition
Confirmed. T's cycle-2 re-verification (`decisions.md`, "T (Phase 7 re-verification after S
REWORK, cycle 1)"): vitest 548/548, tsc 0, lint clean, build 0, cargo `the_runners_list_carries`
and `gitlab` green, all at HEAD `675a213`. A's anti-duplication re-check (same HEAD) is PASS: the
rework is 4 lines in `connections-section.tsx` (`ConnectionRow`'s action cluster), no new GitLab
string, no parallel path.

## Wireframe conformance
No divergence found. This re-walk is scoped to the row-action layout S flagged (not a wireframe
requirement itself — the wireframe does not specify a pixel layout for the action cluster), plus a
spot check of the states the cycle-1 U pass already matched against `wireframe.html` revision 3
frame-by-frame (connections list, add-connection form/reveal, row actions, targets, platform
options/payload, runner link, deliveries). Those matches stand; nothing in F's one-file CSS fix
touches wireframe-specified copy or controls.

## Run environment
GridOps has no e2e seed script and its only login is GitHub OAuth, so (same approach as the cycle-1
U pass and S/F's own tooling) every `/api/v1/*` call was intercepted and mocked in-process to match
the brief's Contract section shapes.

- **Serve:** `npx vite dev --host 127.0.0.1 --port 3100`, started and torn down by this session.
- **Driver:** a throwaway Playwright script (`.u-walk.mjs`, removed after the run) using
  `/Users/andreangelantoni/aftersight-srv/node_modules/playwright` (Playwright 1.63.0, bundled
  Chromium 153.0.8010.12) in headless mode, plain `page.route('**/api/v1/**', …)` plus Playwright
  clicks/role locators (same reasoning as the cycle-1 pass: `u-drive.mjs`'s `login()` assumes an
  HTMX email/password form, and GridOps is a React/TanStack-Router SPA with no HTMX).
- **Mock state:** two seeded GitLab connections (`Mobile app` / project, `Platform group` / group,
  valid UUID v4 ids — a first attempt with non-variant-conformant ids tripped zod's `.uuid()` and
  is recorded under Hedged), one seeded GitLab pool (`gitlab-pool`, targets `Platform group`), one
  GitHub installation, and stateful create/verify/rotate handlers.
- **Login:** `/api/v1/auth/session` mocked to an authenticated admin viewer.
- **Environment workaround (carried from cycle 1, not a #141 defect):** this session's bundled
  Chromium (153.0.8010.12) rejects the pre-existing pool-name `input[pattern]` attribute (present on
  both pool routes before #141) and silently blocks submit; the script strips the attribute before
  clicking Create pool, same workaround F used in its own verification.
- **Evidence paths (session-scratch, not committed):** 7 screenshots (`row-320.png`, `row-360.png`,
  `row-390.png`, `row-768.png`, `row-1280.png`, plus two debug captures of the add-connection reveal
  and the pool-create form) and the full per-step JSON (action, expected, observed, per-button
  geometry table, hit-test result, captured POST body) at
  `/private/tmp/claude-501/-Users-andreangelantoni-Projects-gridops--claude-worktrees-gitlab-141-ui-3ae7f5/026565be-3ca5-4937-951c-4703a5817ecc/scratchpad/u141/{results.json, row-*.png}`.

## State matrix walked

| # | Control / page | State | Reached via | Result |
|---|---|---|---|---|
| 1 | Settings → Connections | GitLab rows present (one/many) | **Hard load** (deep link) | PASS — rows render on first paint |
| 2 | Settings → Members → Connections | round trip | **SPA** (sidebar `Link`, client-side route change) | PASS — no discrepancy vs the hard-load render |
| 3 | GitLab row action cluster (Edit/Verify/Rotate webhook secret/Remove) | **per-button geometry + `elementFromPoint` hit test, not page-level overflow** | SPA (same rendered row) | **PASS at 320, 360, 390, 768, 1280 px** — every button's own rectangle lies inside the card and `elementFromPoint` at its centre returns that same button; no page-level horizontal overflow at any width. Measured coordinates match F's own table exactly (e.g. 360 px: Edit 61–105, Verify 109–165, Rotate webhook secret 169–323, Remove 255–323). At 320/360/390 the cluster wraps (Remove drops to its own right-aligned line below `sm`); at 768/1280 it stays on one line. See screenshots `row-320.png` … `row-1280.png`. |
| 4 | Add connection → GitLab → Connect | error: bad token | SPA (dialog) | PASS — "GitLab rejected the access token." shown verbatim |
| 5 | Add connection → GitLab → Connect | success (one, project kind) | SPA | PASS — 201 reveal: secret + Copy, "Webhook URL" + its own Copy, "Job events" instruction, and the pre-existing create toast ("GitLab connection added.") still fires underneath (A round 2 warn 4 / S's "Create toast kept" is unaffected by F's rework) |
| 6 | New pool → Targets | one (GitLab project) | SPA (real combobox open → search → select) | PASS — "Platform options · GitLab" section appears, collapsed by default (wireframe F) |
| 7 | New pool → Platform options → Run untagged jobs | toggle | SPA | PASS — switch flips to the "on" state |
| 8 | New pool → Create pool | submit | SPA | PASS — captured POST body: `scope: "gitlab"`, `gitlabConnectionIds: ["<the selected id>"]`, `runUntagged: true`, `installationId: 1`, `repositoryIds: []`, `runnerGroupId: 1` — matches the brief's Frame F shape exactly |
| 9 | Webhooks list | one (GitLab delivery) | SPA | PASS — GitLab badge visible on the delivery row |
| 10 | Settings → Connections | 360 px (and 320/390) | SPA + resize | PASS — see row 3; no overflow, every action reachable by its own hit box |

**Zero/empty state:** not re-walked this cycle; the cycle-1 pass recorded this as a flagged gap
(zero-connections empty-state copy is unit-tested by T, not independently re-clicked), and nothing
in F's rework touches that state. Still a (small) gap, carried forward, not newly introduced.

**Console / state assertions:** after fixing two mock-fixture bugs in the walkthrough script itself
(invalid UUID variant nibbles causing a real "Invalid UUID" validation rejection, and a missing
`labels` field on the mocked pool-list item causing a `PoolRow` `TypeError` on the post-create
navigate) — both script defects, not production defects, see Hedged — the final run shows zero
unexpected console errors outside the walkthrough's own deliberate 400 response.

## Findings
**None — the row-action layout defect S reported in cycle 1 is confirmed fixed.** Per-button
geometry and hit-testing at 320, 360, 390, 768 and 1280 px all pass; Edit is no longer clipped at
320/360 px (previously 13/44 px visible at 360, 0 at 320). No new defect was found in the areas
re-walked.

Three environment/process notes, not #141 defects:
1. **Carried from cycle 1:** this session's bundled Chromium throws on the pre-existing pool-name
   `pattern` attribute and silently blocks submit; worked around in the harness, not a code change.
2. **Walkthrough-script defects (mine, not production):** an early iteration of this script's mock
   GitLab connection ids used UUID strings with a non-standard variant nibble (e.g.
   `11111111-1111-1111-1111-111111111111`), which zod's `.uuid()` correctly rejects — the resulting
   "Invalid UUID" callout was the schema working as designed against a bad test fixture, not a
   product bug. Fixed by using variant-conformant UUIDs (`…-4111-8111-…`). Separately, an early
   mock of the pools list omitted the `labels` field that `PoolRow` (`runner-pools.index.tsx:90`)
   reads unconditionally, producing a `TypeError` after a successful create's navigate; fixed by
   completing the fixture. Neither is a #141 defect — recorded for completeness since S's note asked
   U to be precise about what each failure means.
3. The pool detail Overview page still shows a GitLab pool's Account as its GitHub installation
   login — the already-tracked #159 gap the brief marks out of scope; not re-reproduced this cycle
   (no new information).

## Evidence
- Per-button geometry table (all five widths) and the captured create-payload JSON: see
  `/private/tmp/claude-501/-Users-andreangelantoni-Projects-gridops--claude-worktrees-gitlab-141-ui-3ae7f5/026565be-3ca5-4937-951c-4703a5817ecc/scratchpad/u141/results.json`.
- Screenshots: `row-320.png` (actions wrap to two lines, all four fully inside the card),
  `row-360.png` (Edit/Verify/Rotate webhook secret on line one, Remove right-aligned on line two,
  no clipping — this is the exact state S reported as broken, now fixed), `row-390.png`,
  `row-768.png`, `row-1280.png` (one line, unchanged from cycle 1), plus
  `debug-after-create.png` (the reveal dialog: secret, Webhook URL + Copy, Job events note, create
  toast behind it) and `debug-after-submit.png` (post-create navigation, no error callout).
- Both paths are session-scratch, not committed to the branch, per this role's conventions.

## Verdict
**PASS** — S's REWORK item 1 (Edit clipped at 320/360 px) is confirmed fixed by direct per-button
geometry measurement and `elementFromPoint` hit-testing (S's REWORK item 2's exact ask of U), at
320, 360, 390, 768 and 1280 px, with no page-level overflow at any width and no regression to the
768/1280 one-line layout. The surrounding GitLab states spot-checked this cycle (add-connection
reveal, targets picker, Platform options toggle, pool-create payload, delivery badge) all still
match the brief and the approved wireframe, and the console is clean outside the walkthrough's own
deliberate error response. Ready for S.
