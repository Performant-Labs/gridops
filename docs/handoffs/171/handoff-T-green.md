# Handoff-T-green: Phase 7 - GitLab token needs manage_runner; refused deletions become pool warnings (#171, #174)

**Date:** 2026-11-21
**Branch:** claude/gitlab-171-scopes-cbe8b5
**Issue:** #171 (+ #174)
**Handoff-F reviewed:** docs/handoffs/171/handoff-F.md
**Handoff-T-red:** docs/handoffs/171/handoff-T-red.md

## Re-entry after diff gate round 1 and F's rework (2026-11-21)

F changed production in `gitlab_lifecycle.rs` (B-1 `client_safe_reason`, W-1 `warn_pool`). I re-verified and repaired the tests; I changed no production code.

Test changes (all in `crates/gridops-reconciler/src/gitlab_lifecycle_tests.rs`):
- **B-1 pinned.** `a_failed_gitlab_delete_does_not_fail_the_pass_and_the_sweep_retries_it` now fails the delete with `Upstream("502 from GitLab: internal-detail-7f3a")` and asserts neither the event message nor its metadata contains the detail or "502". Before, the test passed whether or not the detail leaked, which is the gap the gate found.
- **W-1 pinned.** New `a_pool_whose_sweep_warning_cannot_be_written_does_not_silence_the_others`: a trigger aborts the refusal insert for `pool-a` only; the sweep must still warn `pool-b`. Before, the only failure test aborted every insert, so it could not tell "continue" from "stop".
- **NIT-1.** `HOUR_AND_MORE` renamed `TWO_HOURS`, with a comment on the double backdating.

Mutation check (temporary, production restored with `git checkout` afterwards, `git status` shows only the tests file modified): making `Upstream` use the full `Display`, and making `warn_linked_pools` return on the first pool failure, made exactly the two tests above FAIL (`20 passed; 2 failed`). With the real code both pass.

Results after the changes (CARGO_BUILD_JOBS=2, RUST_TEST_THREADS=2):
- `cargo test -p gridops-reconciler` -> 120 passed, 0 failed (was 119; +1 new test).
- `cargo test -p gridops-reconciler gitlab_lifecycle_tests` -> 32 passed.
- `cargo test -p gridops-core --lib gitlab::` -> 36 passed.
- `cargo fmt --all --check` -> clean. `cargo +1.98.1 clippy -p gridops-reconciler --all-targets --locked -- -D warnings -A clippy::unused_async_trait_impl` -> clean.
- `npm run lint`, `npm run typecheck` -> clean. `npx vitest run` -> 53 files, 603 tests passed.
- F's claim that the `fleet_http` release-preflight failure predates the branch (fails at `7164fda`) is accepted: the test is host-dependent fleet code this diff does not touch. gridops-api GitLab tests passed in my earlier run and no API or core production file changed in the rework.
- B-2 (UI walkthrough) is U's Phase 9, not a T item; F's seed SQL is in handoff-F.md for U.

Blocking issues after re-entry: None.

---

## GREEN confirmation (first pass)
All Phase-4 tests that were RED now pass. I changed no tests and no production code (F reported "Tests that look wrong: None"; I found none).

- `cargo test -p gridops-core --lib gitlab::` -> 36 passed (the 12-case scope table passes).
- `cargo test -p gridops-reconciler` -> 119 passed, 0 failed (all six RED reconciler tests, including the sweep dedup test, now ok).
- `npx vitest run tests/gitlab-descriptor.test.ts` -> 53 passed (both RED tests ok).
- `cargo test -p gridops-api --no-fail-fast`: every target ok except one (see Tier 1). The three `manage_runner` contract pins pass.

Behavior-vs-implementation spot-check: the RED run in Phase 4 already showed each test failing on the missing behavior alone (rule not enforced, no refusal row, old help text), with setup assertions passing. The sweep test asserts exact row counts per pool across four phases, so removing the dedup, the connection key or the reason key would fail it. I did not re-mutate production code.

## Tier 1 results (CARGO_BUILD_JOBS=2, RUST_TEST_THREADS=2)
- `cargo fmt --all --check` -> clean. PASS
- `cargo +1.98.1 clippy --workspace --all-targets --locked -- -D warnings -A clippy::unused_async_trait_impl` (CI's invocation) -> clean. PASS
- `npm run lint` -> clean. PASS
- `npm run typecheck` -> clean. PASS
- `npm test` -> 53 files, 603 tests passed. PASS
- `cargo test -p gridops-core` -> all targets ok. PASS
- `cargo test -p gridops-reconciler` -> 119 passed. PASS
- `cargo test -p gridops-api --no-fail-fast` -> 1 target failed: `fleet_http release_preflight::the_release_preflight_fails_early_without_a_macos_arm64_machine` (143 passed, 1 failed). Same result as F reports. It is host-dependent (the fixture's "Linux" machine is reported as a ready macOS ARM64 machine on this Mac), lives in fleet/release-preflight code, and the diff touches nothing that test uses besides `http.rs` (F re-ran it on HEAD's `http.rs` and it still failed). Not caused by this change; not blocking. All GitLab API tests pass.
- Server start / API smoke: covered by the API integration tests through the real routers (no live server started).
- Playwright: this change adds none; not run.

## Tier 2 results
- Coverage per criterion: PASS (table below).
- Test quality: PASS. Each test names one behavior and fails for the right reason. The reconciler tests share helpers rather than duplicating setup. The three API tests are contract pins that pass before the feature (T-red explained why); they are the cheapest tier for the 400 and "nothing stored" behavior and do not duplicate the core table. Suite is proportionate to the change.
- Type safety: PASS. Clippy and tsc are clean; no `any` added.
- Error handling: PASS. Refused-delete event write failure is logged and never aborts deletion or the sweep (sweep case tested with a failing-insert trigger); `NotFound` writes nothing (tested); 502 writes a row without the `manage_runner` remedy (tested).
- Data integrity: PASS. Sweep rows leave `runner_id` NULL (FK safe); per-runner rows link `runner_id`; rollback test confirms a later successful delete writes no second row. No migration.
- API contract: PASS. Message format "The GitLab access token is missing the {scope} scope." unchanged; 400 on create, token replacement and Verify, with nothing stored/changed.
- Security: PASS. Tests assert neither the message nor the metadata contains `glpat-` or `glrt-`; `GitLabError` Display carries no token or body (evidence.md).
- Migration safety: N/A (none).
- Docs: `docs/gitlab-runners.md` no longer says `manage_runner` is unchecked; the only other scope lists in docs/, README.md and CHANGELOG.md are consistent. CHANGELOG has an Unreleased "Fixed" entry. PASS
- Evidence appendix: F's entries checked against source; I added the `created_at` millisecond fact. PASS
- Cross-check of F's results: my numbers match F's (36 / 603 / fmt / clippy / the single fleet_http failure).

## Acceptance criteria status
- Core scope table (all three pass, `api` alone passes, missing `manage_runner`, existing cases): PASS, `verify_access_reports_the_first_scope_the_token_lacks`.
- API: 400 with the exact message on create, replacement and Verify, nothing saved: PASS, the three gitlab_tests pins.
- Reconciler: one warning on `MissingScope("manage_runner")` naming connection, runner id and scope; none on `NotFound`; sweep dedup; no token: PASS, six tests in `gitlab_lifecycle_tests`.
- Vitest help text (three scopes, Maintainer for project, Owner for group, no Owner for project): PASS.
- Headless UI walkthrough (form help text, seeded warning in a pool's events): not mine; U's phase.
- Docs no longer say `manage_runner` is unchecked: PASS (grep).
- Lint, typecheck, npm test, fmt, clippy, cargo tests for the three crates: PASS, except the one unrelated host-dependent fleet_http failure above.

## Blocking issues
None.

## Advisory notes
- The unrelated `release_preflight` failure on macOS hosts is worth its own issue; it fails on this Mac independent of #171.
- The API's "Runner deleted" row at `resources.rs:4899` uses level `warn` (A's W3); needs a follow-up issue, out of scope here.
- U must walk a UI surface: the connection form help text and a pool's events list with a seeded "GitLab runner not deleted" warning (no UI test covers the events row).
