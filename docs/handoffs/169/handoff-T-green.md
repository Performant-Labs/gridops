# Handoff-T-green: Phase 7 - Runner rows and dashboard take the pool platform from the registry (#169)

**Date:** 2026-11-21
**Branch:** claude/gitlab-169-runner-rows
**Issue:** #169
**Handoff-F reviewed:** docs/handoffs/169/handoff-F.md
**Handoff-T-red:** docs/handoffs/169/handoff-T-red.md

## GREEN confirmation
- `npx vitest run`: `Test Files 53 passed (53)`, `Tests 602 passed (602)`. All 13 RED Vitest tests are GREEN, with no regressions. Matches F.
- `CARGO_BUILD_JOBS=2 RUST_TEST_THREADS=2 cargo test -p gridops-api --no-fail-fast`: unittests 125 passed (includes the 2 #169 tests); `control_plane_contracts` 36 passed; `fleet_http` 143 passed, 1 failed (see below). Matches F exactly.
- Spot-check (behavior removed): I changed GitLab's `runnerDestination` fallback from `"GitLab"` to `runner.accountLogin`. `tests/runner-row-platform.test.ts` failed 1 of 17 ("a GitLab pool's runner with no name and no address shows GitLab"). Reverted; working tree clean. The RED run had already shown the other tests failing without the feature.
- No test needed repair. F flagged none, and I found none wrong.

## Tier 1 results
| Check | Result |
|---|---|
| `npm run typecheck` | PASS |
| `npm run lint` (`--max-warnings=0`) | PASS |
| `npx vitest run` | PASS (602/602) |
| `cargo fmt --check` | PASS |
| `cargo clippy -p gridops-api --all-targets -- -D warnings` | FAIL on 5 pre-existing `clippy::unused-async-trait-impl` sites in untouched files (fleet/boundary.rs, fleet/enrollment.rs, resources/route_security.rs, gridops-host-agent helper.rs and native.rs). With that lint and `assert-is-empty` allowed it finishes clean. Same as T-red's baseline; not caused by this diff. |
| `cargo test -p gridops-api` | PASS except one host-environmental failure (below) |
| Server starts / API smoke | Covered by the router-level Rust tests (`as_admin GET /api/v1/runners`, `/api/v1/runner-pools`), which pass. No live server run (headless-only, no deploy). |
| Playwright | Not used by the project's scripts for this change; not run. |

`fleet_http::release_preflight::the_release_preflight_fails_early_without_a_macos_arm64_machine` fails. Not blocking: F's diff to Rust is only the `runners` and `runner_pools` handlers in `resources.rs`; the preflight script calls only `/api/v1/fleet…`, and the fixture's "Linux machine" runs the real host agent whose `host_os()` is `cfg!(target_os = "macos")` (`crates/gridops-host-agent/src/probe.rs:17`), so on this Mac it reports macOS and the preflight passes `macos-machine`. Should pass on Linux CI.

## Tier 2 results
- **Coverage per criterion:** every Done-when item has a test (table below). PASS.
- **Test quality:** each test names a behavior and sits at the cheapest sufficient tier (Rust through the real router because the bug is in SQL; jsdom component render for the row and dashboard; source guard for the registry contract). Baseline GitHub/Bitbucket/older-server cases are deliberate regression guards, not duplicates. The suite is proportionate: 2 Rust tests, one 11-case `it.each` plus one job-name test, one dashboard test, and added guard patterns. Nothing flagged for deletion or merge. PASS.
- **Type safety:** typecheck clean. `PoolBindings` members are required, so the row and dashboard call them without `?.`. No `any` in changed files. The `hasPoolBindings` type guard is a narrowing, not a cast. PASS.
- **Error handling / edge cases:** unknown scope gives "—" / "Unknown destination" / "Unknown" and never echoes the raw scope; empty-string scope is unknown, not absent; absent `poolScope` (older server) resolves to GitHub; missing connection parts give no stray `/`; a gone connection gives null `gitlabConnectionName`. All tested. PASS.
- **Data integrity (count sweep, done independently):** `total_runners` at resources.rs:1329 was the only count that treated the LEFT JOIN's NULL row as a runner; the fix (`r.id IS NOT NULL AND r.deleted_at IS NULL`) is verified by the 0 / 2 / live-only test. The other `LEFT JOIN runners` counts (resources.rs:361, :535, :5434; webhooks.rs:836) all use `COUNT(CASE WHEN r.deleted_at IS NULL AND r.<col> ... THEN 1 END)` or `COUNT(DISTINCT ... r.id)`; a NULL row fails `r.deleted_at IS NULL AND r.status IN (...)` / `r.busy=1`, so they are right. The Overview's `runners` metric (line 342) uses `DISTINCT … THEN r.id`, which yields NULL for the NULL row, so it is also right. PASS.
- **API contract:** `/api/v1/runners` items add `poolScope` (NOT NULL column, read as `String`) and `gitlabConnectionName` (nullable); `Runner` type has both optional. `/api/v1/runners` is not in the OpenAPI spec (per A), so no spec change. PASS.
- **Security:** no new inputs; the runners list auth path is unchanged; no new data beyond a pool scope and a connection name the user could already see on the pool list. PASS.
- **Migration safety:** no migration. N/A.
- **Evidence appendix:** `docs/handoffs/169/evidence.md` already lists the facts the tests rely on in unchanged code (NOT NULL columns, the two LEFT JOINs, the GitHub-default precedent, Bitbucket-requires-GitHub), each with verbatim source excerpts. I re-read the cited sources; nothing to add. PASS.
- **Scope guard:** `tests/platform-neutral-source.test.ts` now covers `runner-row.tsx` and `index.tsx`; both import and call `platformForPool` from the registry; the GitLab-code-one-home test still passes (in the 602).

## Acceptance criteria status
| Criterion | Status | Backed by |
|---|---|---|
| Source guard covers runner-row.tsx and index.tsx | PASS | `tests/platform-neutral-source.test.ts` |
| Runner row destination (sheet row, narrow line, right column; text and title) for GitHub repo, GitHub org, Bitbucket, GitLab name / address only / neither, unknown, older server | PASS | `tests/runner-row-platform.test.ts` (11 cases + job-name test) |
| Dashboard scope text: repository, organization, GitLab, Unknown | PASS | `tests/dashboard-pool-scope.test.ts` |
| Runners list returns `poolScope` and `gitlabConnectionName` (null for GitHub) | PASS | `the_runners_list_carries_each_runners_pool_scope_and_gitlab_connection_name` |
| Pool list `totalRunners` 0 for empty pool, right count with a deleted runner | PASS | `the_pool_list_counts_no_runners_for_an_empty_pool_and_only_live_runners_otherwise` |
| Headless UI walkthrough of /runners, a GitLab pool detail, / | NOT RUN by T (owned by U) | Hand to U: the dashboard and runner rows are UI surfaces |
| lint, typecheck, npm test, fmt | PASS | above |
| clippy, cargo test | PASS except the pre-existing/host-environmental items above | above |

## Blocking issues
None.

## Advisory notes
- Hand to U: walk `/runners`, a GitLab pool's detail page and `/` for a GitHub and a GitLab pool. The jsdom tests cannot see real breakpoints, so U should confirm the narrow line and right-hand column at real widths.
- The two clippy lints and the macOS-only preflight failure are pre-existing; worth a separate issue, not this one.
