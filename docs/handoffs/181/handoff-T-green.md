# Handoff-T-green: Phase 7 - Fleet GitLab pools scale on pending jobs (#181)

**Date:** 2026-11-21
**Branch:** claude/gitlab-181-scaling-89ec07
**Issue:** #181
**Handoff-F reviewed:** docs/handoffs/181/handoff-F.md
**Handoff-T-red:** docs/handoffs/181/handoff-T-red.md

## GREEN confirmation
All runs used `CARGO_BUILD_JOBS=2 RUST_TEST_THREADS=2`. No test file changed between T-red and F (`git diff 255023a HEAD` touches no test file except the module doc comment in `autoscaling_gitlab_tests.rs`, which the brief allows).

- `cargo test -p gridops-core`: every target ok (lib 310 passed; `gitlab_pool_fleet` 5 passed, both new tests ok).
- `cargo test -p gridops-reconciler`: 125 passed, 0 failed. The three new tests are ok: `a_pending_job_places_one_fleet_runner_for_a_pool_at_min_zero`, `pending_jobs_scale_a_fleet_gitlab_pool_up_to_max_count`, `jobs_that_are_not_pending_place_no_fleet_runner_for_a_pool_at_min_zero`.
- **Mutation spot-check** (temporary edit, reverted, tree clean): in `choose_platform`, count only GitHub jobs into `queued[index]`. Both new core tests then FAIL (`a_pending_gitlab_job_puts_a_fleet_platform_behind...` and `a_pool_with_github_and_gitlab_jobs_counts_both...`), the 3 existing tests stay ok. So the tests pin the behavior, not the implementation.

## Tier 1 results
- `cargo fmt --check`: exit 0. PASS.
- `cargo +1.98.1 clippy --workspace --all-targets --locked -- -D warnings -A clippy::unused_async_trait_impl` (CI's command): Finished, no errors. PASS.
- `cargo test -p gridops-api --test fleet_http`: 149 passed, 1 failed: `release_preflight::the_release_preflight_fails_early_without_a_macos_arm64_machine`. Same result F reported; F showed it fails identically on unchanged master. The fixture machine reports the host's OS and arch, so it can only pass on a non-macOS-arm64 host. Unrelated to #181. Not blocking. `recovery.rs` and `placement.rs` tests pass.
- Build/server: no change to server startup code; no UI, no HTTP surface, so no curl smoke applies.

## Tier 2 results
- Coverage: each brief criterion has a test (see below). PASS.
- Test quality: 5 tests, each names a behavior, fails for the feature assertion (verified in RED), at the cheapest tier (core integration over a real SQLite DB for `choose_platform`/`platform_jobs`; reconciler pass only for the `NoDemand` guard that needs a pass). No duplicates: the core tests pin attribution, the reconciler tests pin placement and the max cap. The "not pending" test is a deliberate guard (green before and after), recorded as such. Proportionate. PASS.
- Type safety/clippy: clean; new items `pub(crate)` only. PASS.
- Error handling: the accessor skips owned GitHub ids the helper drops (same as before); DB errors propagate with `?`. PASS.
- Data integrity / migration: no schema change, no migration. N/A.
- API contract: no public signature changed. PASS.
- Security: no new input or auth surface. N/A.
- Evidence appendix: F's `evidence.md` has 7 facts with source line and verbatim excerpts. I re-checked the `NoDemand` guard and owner-order excerpts against source; no further facts needed for my tests (my tests rely only on diff-visible behavior and the facts listed).
- Playwright: not applicable (no UI).

## Acceptance criteria status
- A pending GitLab job on a min-0 fleet pool places a runner: PASS (`a_pending_job_places_one_fleet_runner_for_a_pool_at_min_zero`; core `a_pending_gitlab_job_puts_a_fleet_platform_behind...`).
- Scales up to `max_count`: PASS (`pending_jobs_scale_a_fleet_gitlab_pool_up_to_max_count`).
- A starting runner covers the job; no extra runner: PASS (pass 2 assertion; core spare-runner step).
- Mixed pool counts both, lists GitHub only: PASS (`a_pool_with_github_and_gitlab_jobs_counts_both_and_lists_only_github_jobs`).
- Not-pending jobs are not demand: PASS (`jobs_that_are_not_pending_place_no_fleet_runner...`).
- Existing GitHub fleet behavior unchanged: PASS (admission_tests, fleet_http recovery/placement, rest of reconciler suite unchanged and green).

## Blocking issues
None.

## Advisory notes
- The one `fleet_http` failure is host-dependent and predates #181; CI's Linux runner is the place it passes.
- A `None` spare covering a GitLab job relies on a pool linked to GitLab placing only GitLab runners on the fleet (documented in F's comments and evidence.md). In a pool that is both an organization GitHub pool and GitLab-linked this is an accepted approximation.
- Process note: the first run of the full core+reconciler suite exceeded the shell timeout; it completed in the background with the results above.
