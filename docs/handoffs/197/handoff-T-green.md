# Handoff-T-green: Phase 7 - Fleet health shared contract (#197), round 3 (S rework)

**Date:** 2026-11-22
**Branch:** claude/fleet-197-contract (head 3367dd0; test edits uncommitted, for the driver to commit)
**Issue:** #197
**Handoff-F reviewed:** `docs/handoffs/197/handoff-F.md` (round 3 at the top)
**Handoff-S reviewed:** `docs/handoffs/197/handoff-S.md` (findings 1 to 3)

Rounds 1 and 2 are kept unchanged below, under "Rounds 1 and 2". F reported nothing wrong in the tests ("Tests that look wrong: None"). F listed two tests as missing, and the pipeline went from S straight to F without a T-red round, so I authored both here. Both are tests only; I changed no production code.

## Tests authored this round

| Test | Pins | Tier | Why this tier |
|---|---|---|---|
| `tests/app-shell-nav.test.ts` (new, 5 cases, live) | S finding 1: on `/fleet/health`, `/fleet`, `/fleet/machines/abc`, `/queued-jobs` and `/` exactly one sidebar link has `aria-current="page"` and exactly one has `data-status="active"`, and it is the right one. Machines is current on `/fleet/machines/abc`, Health only on `/fleet/health`. | integration (jsdom, real `AppShell` under a memory-history router) | `navigation.test.ts` covers the pure `isActivePath`, but the defect was in the router's own attributes on the rendered link, which only a render reaches. There is no Playwright suite. |
| `speed_contract_tests.rs::a_week_of_50000_jobs_finds_the_slow_machine_with_no_statement_over_100_ms` (#193, `#[ignore = "contract: #193"]`) | Spec section 7: #193's index read runs on `testdata::build_week` (50,400 jobs) under `speed_check::check`. Jupiter's jobs are made 1.5 times slower, so the positive assertions are that Jupiter's index is above 1.2 and the other three machines' are below 1.1 and below Jupiter's, at both runner sizes (2 and 3 CPUs). | integration (SQLite) | The statement-time rule needs a real database at volume. |
| `health_live_contract_tests.rs::the_live_read_over_a_week_of_jobs_counts_the_running_and_queued_with_no_slow_statement` (#201, `#[ignore = "contract: #201"]`) | Spec section 7: #201's `collect_live` runs on `build_week` plus 7 running jobs (on io and Jupiter) and 40 queued ones, under `speed_check::check`. `running_total` is 7, `queued_total` is 40, each category map sums to its total (NULL counted as other), and io's running count is 5. | integration (SQLite) | Same. |

Design notes:
- **The 100 ms rule has a positive assertion in each Rust test**, so it fails against the stub once unmarked (an empty stub passes a "no slow statement" check).
- **#193's thresholds are loose on purpose.** The fleet median includes Jupiter's own runs, so the real index is about 1.4 (about 0.92 for the others); a median over "the rest of the fleet" would give 1.5 and 1.0. Both definitions satisfy "above 1.2, below 1.1".
- **The UPDATE that slows Jupiter is guarded** by `rows_affected() > 10_000`, so a silent no-op cannot make the test vacuous.
- **Time:** each test builds a week once, about 8 s, so the speed-check set is now five `build_week` callers: about 45 s of the 60 s budget, run in parallel binaries in CI. If #193 or #201 find it tight, a smaller week is the lever.

## GREEN confirmation

Run with `CARGO_BUILD_JOBS=2 RUST_TEST_THREADS=2`.

| Command | Result |
|---|---|
| `npx vitest run` | 57 files passed, 4 skipped; **647 passed, 18 skipped** (F's 642 plus my 5) |
| `npx tsc --noEmit` | exit 0 |
| `npm run lint` | clean (`--max-warnings=0`) |
| `npm run build` | `built in 992ms`, typecheck clean |
| `npm run validate:openapi` | valid, 71 warnings (unchanged) |
| `node --test scripts/fleet-health-guard.test.mjs` | 13 of 13 |
| `cargo fmt --all -- --check` | clean |
| `cargo +1.98.1 clippy --workspace --all-targets --locked -- -D warnings -A clippy::unused_async_trait_impl` (CI's command) | clean |
| `cargo test --workspace --locked --no-fail-fast` | **1138 passed, 14 failed, 51 ignored** (49 before; the two new contract tests are the difference) |

The 14 failures are all `gridops-manager` tests with `path must be shorter than SUN_LEN` (the macOS unix-socket path limit for this long worktree path). `git diff` of that crate against the merge-base is empty, so this branch did not touch it. This is the same environmental red as rounds 1 and 2, and CI is unaffected.

**Spot-check that the new tests pin behavior.**
- **Nav test, F's fix reverted** (`app-shell.tsx` restored from `HEAD~1`, then put back; the diff of `src` is clean afterwards): 1 of 5 fails, `/fleet/health`, with `what a screen reader announces: expected [ '/fleet', '/fleet/health' ] to deeply equal [ '/fleet/health' ]`. The other four pass, as they should: the prefix problem only affects a path that sits under another item's.
- **#193 test, run with `-- --ignored` against the stub:** fails at `Error: no speed result for Jupiter at 2000`, after the week, the UPDATE guard (over 10,000 rows) and the `speed_check` wrapper all ran. Right reason: the stub returns no results.
- **#201 test, run with `-- --ignored` against the stub:** fails with `assertion left == right failed: the seeded running jobs, no finished one, left: 0, right: 7`. Right reason: the stub reports an empty fleet.
- The 47 earlier contract tests are unchanged since round 2's marker-removal proof (61 of 61 non-environmental failures against the stubs).

## Tier 1 results
Everything in the table above. F's reported results reproduce: 642 vitest tests (mine: 647 with the new 5), `tsc`, lint, guard 13 of 13, build. No discrepancy.

## Tier 2 results
- **Test coverage per criterion:** PASS. S finding 1 now has a render-level test, and finding 2's two missing realistic-volume tests exist. Every "Done when" item from rounds 1 and 2 is still backed.
- **Test quality (`test-quality.md` section 7):** PASS. Each new test names one behavior, fails in isolation for the right reason (above), sits at the cheapest tier that reaches the behavior, and asserts behavior (what the screen reader sees; the index and counts), not implementation. The suite is proportionate: 5 nav cases are one parametrized behavior, and the two Rust tests are the two spec section 7 required, with no duplicate of #190/#191/#200's.
- **Type safety:** PASS. `tsc` is clean; no `any` in the new test (`router as never` is the repo's existing pattern in `tart-host-card.test.ts`).
- **Error handling / data integrity / API contract / security / migration:** unchanged by this round (F changed one `Link` prop and deleted a dead constant). Covered in rounds 1 and 2.
- **Playwright:** the project has no Playwright suite. U did not run in this pipeline, and S walked the page headless.
- **Evidence appendix:** one entry appended to `evidence.md` (round 3, T): the `<aside>` the nav test selects on and the root-loader read, both in unchanged code, copied verbatim.

## Acceptance criteria status
- S finding 1 (one current page in the sidebar): PASS, `tests/app-shell-nav.test.ts` (fails without F's change).
- S finding 2 (#193 and #201 realistic-volume contract tests): PASS as authored, marked, and failing against the stubs for the right reason.
- S finding 3 (unused `escape`): PASS, the guard script's 13 tests pass.
- All earlier criteria: unchanged and green.

## Blocking issues
None.

## Advisory notes
- **Environmental:** the 14 manager `SUN_LEN` failures (above).
- **Still O's, not verifiable here:** the Mars-copy timing, the two live guard test PRs (cut after this round), the required check, the #190/#194/#201 comments, the #204 reminder.
- **The U stage did not run.** S's headless walk is the only real-browser look; the nav defect it found is now covered by a test.

---

# Rounds 1 and 2 (kept unchanged)

# Handoff-T-green: Phase 7 - Fleet health shared contract (#197)

**Date:** 2026-11-22
**Branch:** claude/fleet-197-contract (worktree `.claude/worktrees/gridops-197`)
**Issue:** #197
**Handoff-F reviewed:** `docs/handoffs/197/handoff-F.md`
**Handoff-T-red:** `docs/handoffs/197/handoff-T-red.md`

## Test-side repairs (all items under F's "Tests that look wrong")

I changed tests only, never production code. All are uncommitted edits for the driver to commit.

| # | File | Change |
|---|---|---|
| 1 | `gitlab/http_tests.rs`, `gitlab/fake_tests.rs`, `webhooks/gitlab_tests.rs`, `gitlab_poll_tests.rs` | `name` added to the four `gitlab::Job` literals (`Some("build")` to match the HTTP payload, else `None`) |
| 2 | `tests/fleet_health_contract.rs` | `Concern.link` literal is `Some(..)` |
| 3 | `registry/query_tests.rs` | the second root-domain id is a distinct UUID (`replacen("10000000","11000000",1)`) |
| 4 | `tests/fleet_schema.rs` | pinned `gitlab_jobs` index list adds `gitlab_jobs_completed_idx` and `gitlab_jobs_unclassified_idx` (0050, D-3c) |
| 5 | `db.rs` `fresh_database_applies_every_migration` | table count 74 to 79 |
| 6 | `webhooks/gitlab_tests.rs` | the stale `pending` event now expects the W-16 name fill: the row is equal except `updated_at`, and `job_name` is asserted to be `rspec`. The test still pins "a later pending changes no state, no lookup" |
| 7 | clippy `--all-targets` (1.98.1) | `TestClock::set` gets `#[allow(dead_code, reason = ...)]` (the #194 contract tests use it once unmarked); the no-effect `end` expression in the speed check; a doc backtick; `type_complexity` (the GitLab-column check is now one `json_each` query); `float_cmp` (`abs() < 1e-9`); `is_multiple_of`; `needless_question_mark`; `usize` to `i64` via `i64::try_from` |

One regression of my own was caught by the full run and fixed: the rewritten GitLab-column query in `fleet_health_schema.rs` had an ambiguous `id` (now `gitlab_jobs.id`).

## GREEN confirmation

Run with `CARGO_BUILD_JOBS=2 RUST_TEST_THREADS=2`.

- `cargo test --workspace --locked --no-fail-fast`: 1137 passed, 49 ignored, 15 failed. The 15 failures:
  - 14 are `gridops-manager` tests, all `Error: path must be shorter than SUN_LEN`. That is the macOS unix-socket path limit for this long worktree path. `git diff $(git merge-base master HEAD) -- crates/gridops-manager` is empty, so this branch did not touch that crate. Environmental, not this change.
  - 1 was `fleet_health_schema::the_upgrade_keeps_existing_jobs_and_leaves_them_unclassified`: my ambiguous-column bug above. Fixed, then re-run: `fleet_health_schema` 5 passed, `fleet_health_contract` 11 passed, `fleet_health_speed_check` 4 passed (8.82 s), `fleet_schema` 17 passed.
- `npx vitest run`: 55 files passed, 4 skipped; 638 passed, 18 skipped (the contract markers).
- `node --test scripts/fleet-health-guard.test.mjs`: 13 of 13.

**Spot-check that the tests pin behavior (the marked contract tests against F's stubs).**
- Rust ignored contract tests run against the stubs:
  - core: 16 of 17 fail.
  - api: 18 of 19 fail, with assertion failures (`{"error":"Job category rules arrive with #199."}`, empty `[]` concerns, the fixture returned for every window, `dependency_unavailable` instead of stale).
  - reconciler: 11 of 11 fail.
- Frontend: with `it.skip` turned into `it`, all 18 contract tests fail (reverted afterwards).
- Two tests pass against stubs. Both are negative or property checks that an empty or placeholder stub satisfies; they exist to guard the real code once wave 1 lands:
  - `no_score_is_ever_outside_0_to_100_on_1000_seeded_inputs` (#192);
  - `a_machine_with_no_cpu_budget_never_raises_host_offline` (#201).
  This is an advisory, not a blocker (see below).

## Tier 1 results

| Check | Result |
|---|---|
| `cargo fmt --all --check` | PASS (applied `cargo fmt --all` to my edits) |
| `cargo +1.98.1 clippy --workspace --all-targets --locked -- -D warnings -A clippy::unused_async_trait_impl` (the CI command) | PASS: `Finished`, no warnings. F reported 9 test-file lints here; all fixed above |
| Workspace tests | PASS apart from the 14 environmental manager failures |
| `npx tsc --noEmit` | PASS |
| `npm run lint` (CI's: `src tests *.config.ts eslint.config.js`) | PASS (linting `scripts/` is not in CI's lint, and shows `no-undef` for Node globals) |
| `npx vite build` | PASS |
| `npm run validate:openapi` | PASS: valid, 71 warnings (F: 4 new, missing `operationId`, same as existing operations) |
| Server start / API smoke | Covered by `health_wiring_tests.rs` and `control_plane_contracts` through the real routers (an admin GET returns the fixture, a member gets 403, the 422 body shape). No live server was started |
| F's numbers cross-checked | Differences from F's report: none beyond the test repairs above. F's speed-check time (8.6 to 8.7 s) matches mine (8.82 s) |

## Tier 2 results

| Check | Result |
|---|---|
| Coverage per acceptance criterion | PASS (below) |
| Test quality (suite proportionate; each test names a behavior; cheapest tier) | PASS. No redundant tests found; the contract tests are one per wave-1 "Done when" |
| Type safety | PASS: `tsc` clean; no `any` in `src/features/fleet-health`, `job-categories-section.tsx`, `tests/fleet-health*`; Rust clippy pedantic clean |
| Error handling | PASS: 400 bad window, 401/403, the 422 `Invalid` body, the 503 and stale path, and the job-category 403 for non-admins are pinned |
| Data integrity | PASS: STRICT tables and CHECKs, indexed window reads and retention DELETEs, the upgrade keeps rows, category set once (`ON CONFLICT` leaves it), GitLab name/path/times set once |
| API contract | PASS: OpenAPI vs the handler (statuses, the window enum, security equal to workload's), and `fixture.json` vs the Rust and zod types |
| Security | PASS: health needs a fleet admin (token `fleet:read` allowed); job-category routes are session-only (`refuse_bearer`), admin-only, and with no token rows |
| Migration safety | PASS (additive): new tables and nullable columns, with the 49 to 50 upgrade test. The repo's migrations are forward-only, as 0047 to 0049 are |
| Playwright | N/A: the project has no Playwright config |
| Evidence appendix | PASS: the repairs add no reliance on unchanged source beyond the entries F already lists (`webhooks/gitlab_tests.rs:163-170` covers the `build_name` fact test 6 now asserts) |

## Acceptance criteria status

Every one of #197's criteria is backed by a live test:
- Migration 0050 (STRICT tables, columns, seeded rules, indexes, upgrade): `fleet_health_schema.rs`, `fleet_schema.rs`.
- Shared types, wire spellings, fixture, fake source, the speed-check helper and the synthetic week: `fleet_health_contract.rs`, `fleet_health_speed_check.rs`.
- GitLab job writer rules (W-16) and the webhook and poll category rules (W-15): `jobs_tests.rs`, `webhooks.rs` tests, `fleet_health_wiring_tests.rs`.
- API wiring, the token row and OpenAPI, and `ApiError::Invalid`: `health_wiring_tests.rs`.
- The fleet queue breakdown matches the Overview: `fleet_queue_tests.rs`.
- Retention at 90 days and the four passes succeeding on an empty database: `fleet_health_wiring_tests.rs` (reconciler).
- Frontend shared modules and navigation: `fleet-health-shared.test.ts`, `navigation.test.ts`.
- The ownership guard: `fleet-health-guard.test.mjs`.
- Wave-1 "Done when" lists: the 18 frontend, 11 reconciler, 18 api and 16 core marked contract tests, each shown failing against the stubs (except the two noted).

## Blocking issues

None. The suite is GREEN apart from the 14 environmental `gridops-manager` SUN_LEN failures in an untouched crate (they depend on the checkout path length, not on this change).

## Advisory notes

1. **Coverage hole (carried from RED, not a pass):** no test renders `/fleet/health` end to end with every placeholder card, and there is no Playwright suite. U must walk the live page, including the Overview Health tile and the nav highlight (F fixed `isActivePath` so Machines and Health are not both active; no test pins the Machines highlight on `/fleet/health`, but the existing navigation tests pass).
2. The two vacuously-green contract tests above stay as wave-1 guards.
3. Still O's (needs push or production access): the guard's live sample PRs, and the speed measurement on a copy of the Mars database.
4. The speed-check set measured about 8.8 s per `build_week` binary; three `build_week` callers plus the 10k backfill should stay under the 60 s CI budget.

---

# Round 2 (after F's round 2, head cceb4c6)

**Date:** 2026-11-22
**Handoff-F reviewed:** `docs/handoffs/197/handoff-F.md` (round 2)
**Handoff-T-red:** `docs/handoffs/197/handoff-T-red.md` (round 2)

F's round 2 changed only `src/features/fleet-health/page.tsx` (six `data-health-section` slots) and `docs/design/fleet-health.md`. No Rust file changed, so the round-1 Rust results above stand. F listed no "Tests that look wrong", and I changed no test or production file this round.

## GREEN confirmation
- `npx vitest run tests/fleet-health-page.test.ts` (the round-2 page test): 4 passed. At T-red it was 3 failed and 1 passed.
- `npx vitest run`: 56 files passed, 4 skipped; **642 passed, 18 skipped** (the 18 are the marked contract tests). Matches F.
- **Spot-check, behavior removed:** I renamed `data-health-section` to `data-removed-section` in `page.tsx`. The page test went back to 3 failed, 1 passed (the same three as the RED), so it pins the slots and not the implementation. I restored the file, and `git status` is clean.

## Tier 1
| Check | Result |
|---|---|
| `npx tsc --noEmit` | PASS |
| `npm run lint` (`--max-warnings=0`) | PASS |
| `npx vite build` | PASS (`built in 925ms`) |
| `node --test scripts/fleet-health-guard.test.mjs` | PASS, 13 of 13 |
| Rust (fmt, clippy, workspace tests) | Not re-run: no Rust file changed since the round-1 PASS. The 14 `gridops-manager` SUN_LEN failures stay environmental |
| F's numbers cross-checked | Same as F (642/18, 13/13) |

## Tier 2
- **Coverage / diff-gate B-2:** PASS. `/fleet/health` renders the fixture into six slots, the window picker, the 403 and the failure/Retry states, in `tests/fleet-health-page.test.ts`.
- **Test quality:** PASS. Four tests, each one behavior, at the cheapest tier (jsdom with a stubbed `fetch`); the 403 test is a deliberate regression guard and is not redundant with the failure test.
- **Type safety:** PASS. `HealthSlot` is a closed union, `tsc` clean, no `any`.
- **Layout risk:** the `min-w-0` block wrapper is layout-neutral today. It is not pinned by a test, so U should look at the row when walking the page.
- **Evidence appendix:** nothing new. The round relies on no unchanged source behavior.

## Acceptance criteria status
All round-1 criteria stand. B-2 is backed by the four page tests. B-1 was met by T-red's marker-removal run.

## Blocking issues
None.

## Advisory notes
- Still O's, as before: the guard's two sample PRs (W-1), the Mars-copy timing (W-2), and the #204 reminder to move `FakeFleetHealthSource` behind `test-support` (W-5).
- The page test passes against placeholders only by design: it asserts slots, not text (warn 28).
- U must walk `/fleet/health` live: no Playwright suite exists, so this surface is a coverage hole for headless checks.
