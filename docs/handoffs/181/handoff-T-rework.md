# Handoff-T-rework: Phase 6 re-entry - S test-only rework R1, R2 (#181)

**Date:** 2026-11-21
**Branch:** claude/gitlab-181-scaling-89ec07
**Issue:** #181
**Findings:** docs/handoffs/181/handoff-S.md (R1, R2)
**Dispatch:** O sent this round to T after the router sent it to F by mistake (F confirmed no production change is needed).

No production file changed. `git diff` touches only `crates/gridops-core/tests/gitlab_pool_fleet.rs`, `crates/gridops-reconciler/src/gitlab_lifecycle_tests.rs`, and the files under `docs/handoffs/181/` (this handoff, `decisions.md`, `evidence.md`).

## R1: the "not pending" reconciler test never reached the `NoDemand` guard

**Test:** `gitlab_lifecycle_tests::jobs_that_are_not_pending_place_no_fleet_runner_for_a_pool_at_min_zero` (option 1: repaired in place).

**Change:**
- The fixture update now sets `idle_timeout_minutes=100000000` beside `desired_count=2`. With no runners, `last_activity` is 0, and `now - 0` is about 1.8e12 ms, far below the timeout's 6e12 ms. So `maybe_scale_down` returns early, desired stays 2, and `provision` runs against 0 active runners. The platform is not behind (only `created`/`manual` jobs), so the guard `active >= min_count` (0 >= 0) returns `NoDemand`.
- A new assertion checks that `desired_count` is still 2 after both passes, so the test fails loudly if a later change resets it and makes the test vacuous again.
- The placement assertion now has a message. The doc comment explains the path and why the idle timeout is raised.

**Mutation M1** (`crates/gridops-reconciler/src/main.rs:2450`, `if active >= pool.min_count {` changed to `if false && active >= pool.min_count {`; reverted with `git checkout --`):

    test gitlab_lifecycle_tests::jobs_that_are_not_pending_place_no_fleet_runner_for_a_pool_at_min_zero ... FAILED
    panicked at crates/gridops-reconciler/src/gitlab_lifecycle_tests.rs:2205:5:
    assertion `left == right` failed
      left: 2
     right: 0

Under M1 the guard is gone and the pass places 2 runners. Unmutated, the test passes. So it now fails for the right reason.

**Correction to T-red:** T-red said this test "will fail if F ... weakens the guard" and offered a `min_count=1` probe as evidence. Both were wrong for the old fixture. The probe ran a different path (scale-down resets desired to 1, and `0 >= 1` is false). The claim holds only from this rework on.

## R2: no test pinned "only a spare runner with no repository covers a GitLab job"

**Test added:** `a_spare_runner_registered_to_a_github_repository_does_not_cover_a_gitlab_job` in `crates/gridops-core/tests/gitlab_pool_fleet.rs`. `spare_runner` now takes `repository: Option<i64>`, and the two existing callers pass `None`.

**Why it doesn't assert "stays behind" as S suggested:** I first wrote S's literal shape (one platform, a pending GitLab job, a starting spare registered to repository 10, assert `behind`). It passed **under M2 too**, so it couldn't detect the rule. The reason is `choose_platform`'s `deficit` (`targets.rs:1539-1547`):
- it adds every attributed job to `queued`, whichever runner "covers" it;
- it ignores an idle repository runner unless a job of that repository is waiting on its platform.

So on one platform the `behind` flag can't see which runner covered a GitLab job. The only place the rule shows is attribution: which platform a job lands on, and what `platform_jobs` lists for that platform.

**Shape:**
- An organization pool linked to `conn-project` has two Linux Docker platforms, x64 first and arm64 second.
- It has a pending GitLab job (recorded at 10, so it is attributed first because owners are oldest first) and a queued GitHub job of repository 10. Both jobs can run on either platform.
- Its only runner is a starting spare on arm64, registered to repository 10.
- Correct result: the GitLab job can't take that spare. With no room and no machine it counts for x64, the first platform. The spare stays with repository 10's job, so `platform_jobs(arm64) == [10]` and `platform_jobs(x64)` is empty.
- If the repository-10 runner covered the GitLab job, the GitHub job would lose its runner and move to x64. That is the under-scaling and mis-placement S describes, in observable form.

**Mutation M2** (`crates/gridops-core/src/fleet/targets.rs:1456`, `FleetQueuedJob::Gitlab => repository.is_none(),` changed to `FleetQueuedJob::Gitlab => true,`; reverted with `git checkout --`):

    test a_spare_runner_registered_to_a_github_repository_does_not_cover_a_gitlab_job ... FAILED
    panicked at crates/gridops-core/tests/gitlab_pool_fleet.rs:538:5:
    assertion `left == right` failed: the repository-10 runner stays with repository 10's job, not the GitLab job
      left: []
     right: [10]
    test result: FAILED. 5 passed; 1 failed

Unmutated, all 6 tests in the file pass.

## GREEN and Tier 1 (after both mutations were reverted; `git status` showed only the two test files)
All runs used `CARGO_BUILD_JOBS=2 RUST_TEST_THREADS=2` and the shared target dir.
- `cargo test -p gridops-core`: 422 passed, 0 failed, 1 ignored (pre-existing) across all targets. The lib has 310; `gitlab_pool_fleet` has 6, up from 5 by the new R2 test. PASS.
- `cargo test -p gridops-reconciler`: 125 passed, 0 failed. PASS.
- `cargo fmt --all -- --check`: exit 0. PASS.
- `cargo +1.98.1 clippy -p gridops-core -p gridops-reconciler --all-targets --locked -- -D warnings` (CI's pinned toolchain): Finished, no warnings. PASS.
- The same clippy on the local 1.99.1 reports 5 `assert!`-on-empty lints. They are in `autoscaling.rs`, `autoscaling_gitlab_tests.rs` and one reconciler test site, none of them in the two files this round changed. This is the toolchain-drift advisory already recorded at T-red, so it doesn't block this round.

## Tier 2 (test quality, rubric §7)
- R1 test: it names one behaviour (non-pending jobs are not demand at the `NoDemand` guard), fails in isolation under M1, needs a reconciler pass (the guard lives in `provision`), and asserts placements, not internals. Its `desired_count` check is a precondition guard against going vacuous again.
- R2 test: it names one behaviour (a repository runner never covers a GitLab job), fails under M2, sits at the cheapest tier (core over a migrated SQLite DB), and doesn't duplicate the mixed-pool test, which uses a `None` spare and one platform.
- Proportionate: one test repaired, one added, no deletions needed.
- Evidence appendix: 4 entries appended to `evidence.md` for unchanged-code facts these tests rely on: the scale-down idle timeout, owners oldest first, the attribution sort and first-platform fallback, and the deficit's repository-runner filter. Each excerpt was copied from source.

## Blocking issues
None.

## Advisory notes
- S's A2 and A3 remain advisory and were not part of this round.
