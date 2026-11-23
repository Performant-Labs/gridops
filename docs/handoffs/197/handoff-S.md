# Handoff-S: Phase 10 - Fleet health shared contract (#197)

**Date:** 2026-11-22
**Branch:** claude/fleet-197-contract (head 45dfed6, working tree clean when the audit started)
**Issue:** #197 (wave 0 of epic #198)
**Handoff-T reviewed:** `docs/handoffs/197/handoff-T-green.md` (rounds 1 and 2) and `handoff-T-red.md` (rounds 1 and 2)
**Handoff-A reviewed:** `docs/handoffs/197/handoff-A.md` (plan review, round 4, PASS), `handoff-A-dup.md` (PASS), and the diff gate's `docs/handoffs/197-diff-result-r1.md` (BLOCK, since resolved) and `-r2.md` (PASS)
**Handoff-F reviewed:** `docs/handoffs/197/handoff-F.md` (rounds 1 and 2)
**Spec read:** issue #197 in full (`gh issue view 197`), the wave-1 issues' "Owns" sections (#190, #191, #193, #194, #195, #199, #200, #201, #202, #203, #204), #192's and #201's tables, `docs/handoffs/197-brief.md` (including D-1 to D-4 and W-5 to W-20)
**Operator-facing report:** none (see "Visual-diff-tool precondition": there is no reference to diff against). Evidence screenshots: `docs/handoffs/197/screenshots/`

## A precondition
Confirmed: A returned **PASS** on the plan (round 4) and on the diff (A-dup). The diff gate's two BLOCKs (B-1 marker-removal proof, B-2 page test) were resolved in round 2 and its round 2 returned PASS.

## T precondition
Confirmed: T-green reports **zero blocking issues** (round 1 and round 2). The only red it carries is 14 `gridops-manager` `SUN_LEN` failures, which come from the macOS socket-path length for this worktree's path, in a crate the branch doesn't touch (`git diff` of that crate is empty).

## Visual-diff-tool precondition (visual / web projects only)
Partly met, and not applicable as a pixel diff. I'm stating exactly what was and wasn't possible:
- **Tools:** ImageMagick `compare` is on PATH. Playwright wasn't installed in the project, so I installed it in the scratchpad (outside the repo) and drove the existing Chromium build headless. The `mcp__Claude_in_Chrome__*` tools are not in my tool list, and the user's standing rule is headless only, so I used Playwright.
- **No reference exists.** `/fleet/health` is a new route. The brief says "Wireframe: N/A (the page renders placeholder cards only)". There is no preview HTML, no design spec slice and no prior baseline to diff against, so a live-versus-reference pixel delta cannot be computed. I did not substitute prose for a diff: I did not produce one, and I'm not claiming a pixel-delta result.
- **The U (UI walkthrough) stage did not run in this pipeline** (no `handoff-U.md`, and the last commit is `a-dup`). T-green's advisory said "U must walk the live page". S is therefore the first real-browser look at the page, so I did that, headless, as a functional walk (below). Pixel-level review of the real sections belongs to #195, #202 and #203, whose "Done when" lists carry the 360/1280 light/dark screenshot items.
- **Consequently no HTML wipe-slider report was written.** It compares live to a reference; with no reference it would be an empty artefact.

### Headless walk of `/fleet/health` (functional, not a pixel diff)
Vite dev server on 127.0.0.1, Chromium headless via Playwright, `/api/**` intercepted: the session as an admin, `GET /api/v1/fleet/health` answered with `src/features/fleet-health/fixture.json`, anything else 404.

| Viewport | Theme | Console errors / page errors | Horizontal scroll | Slots present | "Coming in #N" cards | Health request |
|---|---|---|---|---|---|---|
| 360×800 | dark and light | none | none (360/360) | 6 of 6 | #203, #195, #195, #203, #202, #202 | `GET …/fleet/health?window=24h` |
| 768×1024 | dark and light | none | none (768/768) | 6 of 6 | same six | same |
| 1280×800 | dark and light | none | none (1280/1280) | 6 of 6, two side by side in the `lg` row (489 px each) | same six | same |
| 1280×800, health answers 403 | dark | only the browser's own "403" resource line | none | 0 slots, "Administrators only" shown | none | |

Screenshots: `docs/handoffs/197/screenshots/s-health-{360,768,1280}-dark-20261122.png`, `s-health-{360,1280}-light-20261122.png`, `s-health-1280-403-20261122.png`.
- The six slots stack in the order live strip, areas of concern, fleet score, category legend, machine table, machine detail. At 1280 the areas-of-concern and fleet-score slots share a row, and the `min-w-0` wrapper does not overflow.
- Window picker: the three buttons are 24 px tall at 360. That is the app's existing density (nav items are 28 px), so I'm recording it as advisory only.
- **A defect found only in the browser: the sidebar marks two items as the current page. See finding 1.**

## Spec compliance

Checked against issue #197's body, with the brief's decisions (D-1 to D-4, W-5 to W-20) layered on top where it deliberately departs from the issue text.

| Spec item | Checked | Compliant |
|---|---|---|
| §1 Design doc `docs/design/fleet-health.md` | Read in full. Fleet and machine score tables (weights 40/20/15/15/10 and 25/25/25/15/10, "100 at"/"0 at"), the concern tables (windowed and live), buckets, job categories and default rules, API shape, retention, testing, ownership. Compared with #192's and #201's tables line by line: identical. | YES |
| §2 Migration | `migrations/0050_fleet_health.sql` (0049 was taken; the brief corrects the number). Every table `STRICT` with explicit `CHECK`s and a comment. `job_category_rules` seeded with the four default rules (`other` is the fallback, no rule). `workflow_jobs.category` and `gitlab_jobs.category` nullable. `fleet_health_hourly`, `fleet_host_hourly`, `fleet_health_recompute`, `fleet_health_snapshots` have the columns the issue lists, the 9-bucket JSON histograms and the `hour % 3600000 = 0` checks. Indexes on both jobs tables' `completed_at`. Deviations, all recorded: `fleet_health_recompute` has `requested_at` beside `hour`; the `gitlab_jobs` columns `job_name`, `project_path`, `started_at`, `completed_at` (brief D-1, because 0047 has no times); two partial "unclassified" indexes. Verified by running `fleet_health_schema` (5 passed). | YES (the Mars timing is not verified, see O's list) |
| §3 Shared types, `merge_concerns`, source trait, fake | `fleet_health/mod.rs`: `JobCategory`, `HealthWindow` (`1h`/`24h`/`7d`), `FleetHealth`, `HostHealth`, `ScoreComponent {key,label,weight,score:Option<u8>,deduction:Option<String>}`, `CategoryUsage`, `HostLiveStats`, `FleetLiveStats`, `Concern` with `subject` fleet/host(id)/pool(id), the scoring-engine input types, and `merge_concerns` (severity, then oldest `since`, one per key). `FleetHealthSource` is an `#[async_trait]` with `windowed` and `live`. The fake serves io, Jupiter, callisto, europa and MacBookPro at budgets 22,800/19,200/8,000/12,800/0 with a slow host, start failures, a backlog and all three severities. I ran `fleet_health_contract` (11 passed). `JobCategory` also has `fix-agent` (never stored); the design doc says so. | YES |
| §3 `category.rs` implemented here | `compile(rules) -> Classifier`, `classify(...)`, rules in `position` order, first match wins, `other` when nothing matches, an invalid regex skipped with a warning and named by `validate`. Tested live. | YES |
| §4 Stubs and hooks table | Every row checked in the diff: `pub mod fleet_health;` in core `lib.rs`; the four reconciler `run_pass` calls in one `fleet_health_passes` (log-and-continue, before the manager-unavailable early return); the three 90-day `DELETE`s in `cleanup_retention`; the three category routes in `session_routes()` (brief W-5, replacing the issue's `fleet/mod.rs`); the health route and its `TOKEN_SCOPED_ROUTES` row; `mod` lines for `health`, `health_live`, `health_source`; the `AppState` field and builders; route file, nav item, `settings.general.tsx`, the Overview hook, `api.ts`, `chart-theme.ts`, `score-gauge.tsx`. Each stub has the final signature, a doc comment naming its issue, and a placeholder body. | YES |
| §4 Classification at the three real writers (D-1) | GitHub webhook, reconciler poll and GitLab writer each bind `category` in `VALUES` only, through `category_for_insert` on the caller's connection; `ON CONFLICT` never touches it. GitLab name, project path, start and finish times are filled once. | YES |
| §5 Wave-1 tests written here | 47 Rust (`#[ignore = "contract: #N"]`): core 17 (#190 ×2, #192 ×9, #193 ×6), api 19 (#190 ×1, #194 ×5, #199 ×5, #201 ×8), reconciler 11 (#190 ×3, #191 ×4, #200 ×4). 18 Vitest (`it.skip("[contract: #N] …")`): #195 ×6, #199 ×4, #202 ×4, #203 ×4. I mapped every "Done when" bullet in the brief's copies to a test; all are present, with the issues' own expected values (for example #192's "7 min 12 s … −28" sentence). D-2 places the Vitest ones in `tests/`. | YES |
| §5 "fails against the stubs once its marker is removed" | **Independently confirmed for all 47 Rust tests** by running the ignored tests: core `-- --ignored fleet_health` 17 of 17 failed; api bin `-- --ignored contract` 19 of 19 failed; reconciler `-- --ignored contract` 11 of 11 failed (each on a stub assertion). For the 18 Vitest ones I relied on T-red round 2's scratch-branch run (all 18 fail) and T-green's spot-check; I did not edit the worktree to repeat it. | YES |
| §6 Guard job and script | `scripts/fleet-health-guard.mjs` (no dependencies), its own tests (13 of 13 pass when I ran `node --test`), the `Fleet health guard` job in `ci.yml` (PRs only, read permissions). Scope rule, add-only shared files, contract tests editable only by removing the issue's own markers, "closes none → passes", "closes two → fails", contract tests stay #197's whatever glob matches. The owners file matches each wave-1 issue's "Owns" text. The grep check is automatic (`checkOwners` over `git ls-files`). | YES |
| §6 "Fleet health guard fails on a test PR that edits a file outside an issue's list / edits a contract test; both closed afterwards" | Not done, and not doable by F or T (needs a push and GitHub). Covered by the guard's unit tests only. | **NOT VERIFIED, O** |
| §7 Synthetic week and speed-check helper | `testdata.rs` (`build_week`: 7 days, five machines, about 50,000 jobs, a sample every 30 s, in batches) and `speed_check.rs` (slow statements over 100 ms via the #179 log, write transactions over 50 ms). Both behind `test-support` (dev-dependencies only, so no production build includes them). The helper fails a deliberately slow query in its own test binary (`fleet_health_speed_check`: 4 passed, 8.8 s, which I ran). | YES |
| §7 "The contract tests for #190, #191, #200, **#193 and #201** use both" | #190, #191 and #200 do (`speed_check::check` with `build_week` or a 10,000-row seed). The #193 and #201 contract tests do not use either one: `speed_contract_tests.rs` and `health_live_contract_tests.rs` seed a handful of rows and never call `speed_check::check` or `build_week` (grep confirms). The brief copied only the Done-when lists, where #193's and #201's timing items are "not a test", so the sentence in #197's own §7 was dropped. | **NO, finding 2** |
| §8 OpenAPI and UI fixture | Health path (with `window` enum 1h/24h/7d, 200/400/401/403/503, `fleet:read` bearer plus session) and the three job-category paths (session only, 200/401/403, 422 on save and preview) are in `docs/openapi/gridops-v1.json`. I compared the health 200 example with `fixture.json` by deep equality: identical. `types.ts` has the zod schemas. `npm run validate:openapi` is valid (71 warnings, 4 more than HEAD, all missing `operationId`s like the existing operations). | YES |
| D-4 admin-only health, tokens, 422 | A member gets 403, no session 401, an admin's `fleet:read` token accepted; job-category routes are session-only and admin-only (stubs check the caller, then answer 501). `ApiError::Invalid` renders `{error, code, details}` without changing any existing body. Both `docs/fleet-api.md` tables updated. | YES |
| Done when: migration on a copy of the Mars DB (2.9 GB) in under a minute | Not run (needs ssh to production). | **NOT VERIFIED, O** |
| Done when: a test deserializes the UI fixture into the Rust types | `the_ui_fixture_is_the_rust_types_the_fake_and_the_openapi_example`: pass | YES |
| Done when: `GET /api/v1/fleet/health` returns the fixture and `/fleet/health` renders every placeholder card | The handler is covered by `health_wiring_tests.rs` through the real router. The page: `tests/fleet-health-page.test.ts` (4 passed) and my headless walk above (6 of 6 slots, six placeholder cards, at 360/768/1280). | YES |
| Done when: full workspace checks | T-green ran fmt, CI's clippy (`+1.98.1`, `--all-targets`), the workspace tests, `tsc`, lint, vitest (642 passed, 18 skipped), `vite build` and `validate:openapi`. I re-ran `cargo fmt --check`, `tsc --noEmit`, the core integration tests, the guard tests and the page, shared, navigation and dashboard-pool-scope Vitest files (23 passed): all green. | YES |
| Done when: speed-check helper fails a slow query in its own test | `a_deliberately_slow_statement_fails_the_check`, `a_long_write_transaction_fails_the_check`: pass | YES |
| Done when: grep check | Automatic, in the guard tests | YES |

## Quality audit

| Area | Result | Notes |
|------|--------|-------|
| API consistency | PASS | The health route follows `/fleet/workload` (`FleetUser<Read>` plus `require_admin`, token row, both docs tables, OpenAPI security). The 422 body is additive. The category routes sit under `refuse_bearer` as the brief decided. |
| Error handling | PASS | 400/401/403/503 documented and pinned; the writers never fail on a classifier error (logged, `None`); the passes log and continue. |
| UI/UX match to spec | PASS with finding 1 | Every slot renders; loading, 403 ("Administrators only") and failure-with-Retry states are tested and walked. |
| Accessibility | **FAIL** | **Finding 1.** `/fleet/health` marks both Machines and Health with `aria-current="page"` (and `data-status="active"`). |
| Architecture gate | PASS | A round 4 PASS, A-dup PASS. Nothing here conflicts with A. |
| Code organization | PASS, one nit | No TODOs, debug output or commented-out code. `const escape` at `scripts/fleet-health-guard.mjs:89` is unused (the diff gate's NIT-1, carried since round 1 and never fixed): finding 3. |
| Security | PASS | Admin gating tested for health and the three category routes; no secrets; the CI job uses `pull_request` with read-only permissions. One hardening note below (the guard reads its own rules from the PR's branch). |
| Performance | PASS (unverified on Mars) | Reads are bounded by indexed time columns; the synthetic-week check passes at about 8.8 s. The Mars-copy timing is O's. |
| Visual regression | N/A | No reference and no Playwright suite in the project. See above. |
| Naming consistency | PASS | Keys, wire spellings and issue terms match the issue and the design doc (`host-offline:<host>`, `1h`/`24h`/`7d`, `fix-agent`). |
| Test quality (`testing/test-quality.md` §7) | **FAIL, missing coverage only** | Per test: the contract tests name one behaviour each, assert the issues' exact values, and fail against stubs for the right reason (checked). T's round-2 controls removed the two vacuous passes. Per suite: proportionate (47 + 18 contract tests for 11 issues' Done-when bullets, plus live tests for what #197 implements); no tautological, duplicate-signal or coverage-padding test found, so there is no "delete or merge" finding. The gap is finding 2: two required volume tests are missing. |

## Findings

### 1. Machines is still marked as the current page on `/fleet/health` (production, accessibility)
- **What I saw (Chromium, 360/768/1280):** the sidebar's `<a href="/fleet">Machines</a>` carries `aria-current="page"` and `data-status="active"` on `/fleet/health`, while Health carries them too. The visual highlight is right (only Health has the selected background), so F's `isActivePath` change works for the class, but a screen reader announces two current pages. The brief adopted "correct active state for Machines vs Health" and the design doc says "Machines is not highlighted on it".
- **Cause:** `NavLinkItem` (`src/components/app-shell.tsx`, the `<Link aria-current={active ? "page" : undefined}`) passes `aria-current` itself, but TanStack Router's `Link` (1.170.18) sets `aria-current: "page"` and `data-status: "active"` after the caller's props for any link its own prefix match calls active, and `/fleet` is a prefix of `/fleet/health`. `isActivePath` only drives the class names. Nothing tests the rendered link; `tests/navigation.test.ts` covers the pure function.
- **Fix (smallest):** make the router's notion follow the app's, for example `activeOptions={{ exact: !active }}` on that `Link`, so a link the app calls inactive is router-active only on an exact match. Add a render-level assertion that exactly one sidebar link has `aria-current="page"` on `/fleet/health`, and that Machines is the one on `/fleet` and on `/fleet/machines/<id>`. No wave-1 issue owns `app-shell.tsx`, so if this is left it stays.

### 2. #193 and #201 have no realistic-volume contract test (test, spec §7)
- #197 §7 says the contract tests for **#190, #191, #200, #193 and #201** use the synthetic week and the speed-check helper, "so CI enforces the #179 lock rules on realistic volume, not just in instructions". Only #190, #191 and #200 do. #193's `speed_indexes` reads up to 7 days of jobs and groups them; #201's `collect_live` is a bounded read with a 50 ms target; nothing in CI would flag a statement over 100 ms from either on 50,000 jobs.
- **Needed:** one marked test each, in the files that already hold their contract tests, run under `speed_check::check` over `testdata::build_week`:
  - `#[ignore = "contract: #193"]`: `speed::speed_indexes` over the week returns an index for the machine(s) the week makes slower, with no statement over 100 ms.
  - `#[ignore = "contract: #201"]`: `collect_live` over the week returns the seeded running and queued counts (each category sum equals its total), with no statement over 100 ms.
  - Each needs a positive assertion on the result so it **fails against the stub once its marker is removed**, as every other contract test does; a test that only checks "no slow statement" passes against an empty stub.
  - Keep the whole speed-check set under 60 s in CI: today it is about 8.8 s per `build_week`, and two more callers make five. If that is tight, one smaller week for these two is acceptable if the handoff says so.
- This needs **T** (F can't edit tests). Per the routing, F's rework pass should list it under "Tests missing (for T)".

### 3. Unused helper in the guard script (production, trivial)
- Delete `const escape = …` at `scripts/fleet-health-guard.mjs:89`. It was the diff gate's NIT-1 in rounds 1 and 2 and is still there. Because this touches the guard script, **the two live guard test PRs must be cut after this change** (they are O's anyway).

## Scope check
Delivered the whole of #197's scope as one PR, as the brief requires, with no stub filled beyond what #197 implements (`category.rs`, `merge_concerns`, `api.ts`, `chart-theme.ts`, `score-gauge.tsx`, the fake source, the guard, the speed-check helper, the synthetic week). Additions beyond the issue's text, all brief decisions: the GitLab job name/path/time columns and writer (D-1), `host_live_reads` and `fleet_queue_breakdown` for #201 (D-3 b, f), the `ApiError::Invalid` 422 (D-3 d), the nav `isActivePath` fix, `useFleetHealth` on the Overview with `metricTiles`' second parameter (D-3 c). No over-delivery found. Under-delivery: finding 2, and the two O-only acceptance items below.

## Verdict

REWORK — the following must be addressed before commit:

1. **(F, production)** Finding 1: stop Machines from being `aria-current="page"` on `/fleet/health` (`NavLinkItem` in `src/components/app-shell.tsx`), and list in F's handoff the render-level nav test for T.
2. **(T, new tests)** Finding 2: add the #193 and #201 realistic-volume contract tests described above, marked `contract: #193` / `contract: #201`, each failing against the stubs once unmarked, with the speed-check set still under 60 s.
3. **(F, trivial)** Finding 3: delete the unused `escape` in `scripts/fleet-health-guard.mjs`.

Everything else I audited is compliant. This is a `production` rework (items 1 and 3 change production files, item 2 changes tests): not `test-only`.

### Merge conditions that are O's, not a rework (carried from F, T-green, A and the diff gate; none is a pipeline failure)
These are #197 "Done when" items that need a push or production access. They are **not verified** and the PR must not merge without them:
- **Mars timing:** apply migrations 0001 to 0050 to a read-only `VACUUM INTO` copy of the Mars database (2.9 GB), record the wall time and the Mac's `uptime` load next to the Done-when item in the PR description, and delete both copies even on failure (brief, "The Mars database timing check"). The one step to watch: `ALTER TABLE … ADD COLUMN … CHECK` and the two partial `category IS NULL` indexes each read every `workflow_jobs` and `gitlab_jobs` row, so on a large table the time is worth seeing before this migration runs at API start-up.
- **Two guard test PRs** against `Performant-Labs/gridops` only, cut from the head **after** this rework (it changes the guard script and contract test files), one editing a file outside an issue's list and one editing a contract test; both must fail `Fleet health guard`, then be closed and their branches deleted; record both head SHAs.
- After merge: add `Fleet health guard` to `master`'s required checks; post the correcting comments on #190 (insert sites, D-1), #194 (admin-only, D-4) and #201 (the host read and queue variant); remind #204 to move `FakeFleetHealthSource` behind `test-support`.

## Advisory notes
- **The guard reads its own rules from the PR's branch.** On a `pull_request` event the checkout is the PR's merge result, so a wave-1 PR that edits `scripts/fleet-health-guard.mjs` or `docs/design/fleet-health-owners.json` is judged by its own edited copy. Those edits are themselves flagged as outside the issue's files, so an honest mistake is caught; only a deliberate edit of both the owners list and the files would pass. Hardening, if wanted: check out the base ref for the script and the owners file. Also, the job comment "runs no code from the branch but the guard itself" is slightly off, since the guard is from the branch.
- **#190's issue text is stale** (it still owns "the `category` lines at the insert sites"); the guard would refuse that edit. The correcting comment on #190 must be posted before wave 1 starts.
- The window picker's buttons are 24 px tall at 360 px (below the 44 px touch-target guideline). It matches the app's existing density and #195 owns the frame; leave it to #195.
- `CHANGELOG` says new jobs "get a `category` column": true, but the value stays NULL until #190 fills `category_for_insert`. Harmless.
- A member's Overview load makes one `GET /api/v1/fleet/health` that returns 403 and stops; the browser logs it once. It follows the workload chart's pattern (D-4), so I left it.
- This audit's evidence, not committed: the walk script and raw results are in the session scratchpad; the screenshots are in `docs/handoffs/197/screenshots/`. The light and dark runs at 360 and 1280 both rendered correctly (the first dark/light emulation did not switch the app theme, because the app uses its own `dark` class, so I re-ran with the class removed for the light shots).
