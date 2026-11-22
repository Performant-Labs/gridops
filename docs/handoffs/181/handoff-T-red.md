# Handoff-T-red: Phase 4 - Fleet GitLab pools scale on pending jobs, not only to min_count (#181)

**Date:** 2026-11-21
**Branch:** claude/gitlab-181-scaling-89ec07 (head 724fa83; no production code yet)
**Brief / wireframe reviewed:** docs/handoffs/181-brief.md (amended in a19e9e6). Wireframe: N/A (no UI surface).

## A precondition

Confirmed: A returned **PASS** on the amended plan (`docs/handoffs/181/handoff-A.md`, commit 724fa83), with 0 blocks and 2 warns.
- **Warn 1** (the job type: one `pub(crate)` enum wrapping `QueuedJob`, beside the accessor). The type is not public, so no test pins its shape. The tests go through `choose_platform`, `platform_jobs` and the reconciler pass only.
- **Warn 2** (the accessor keeps owner order by id and skips ids the helper didn't return). This is pinned indirectly: every existing GitHub fleet test (`fleet/admission_tests.rs` `jobs_any_platform_can_run_go_where_machines_have_room`, `gridops-api` `tests/fleet_http/recovery.rs`) must still pass unchanged. A checks the ordering itself at Phase 7.

## Tests authored

Five new tests, at two tiers, and one fixture change. No `pub` item was added for tests. The only visibility change is a test helper inside a `#[cfg(test)]` module: `gitlab_poll_tests::row` is now `pub(super)`, so the lifecycle tests can seed `pending` rows the way the poll tests do (brief: "Seed `pending` rows the way `gitlab_poll_tests.rs` … does"), without a second copy of that SQL.

### Core: `crates/gridops-core/tests/gitlab_pool_fleet.rs` (integration test over a real migrated SQLite database, public API only)

`choose_platform` and `platform_jobs` are `pub` and their logic is SQL plus attribution, so a migrated database is the cheapest tier where the rule runs. The file already seeds a `scope='gitlab'` pool with connections, so it needs no second fixture. It now has two new `Database` helpers: `pending_gitlab_job` (one `pending` `gitlab_jobs` row) and `spare_runner` (a starting, not-busy fleet runner with no repository: a `runners` row, a `fleet_operations` row and an admitted `workload_intents` row). It also has a free helper, `behind`.

1. **`a_pending_gitlab_job_puts_a_fleet_platform_behind_until_a_spare_runner_covers_it`** (line 321). Covers the brief's Tests "Core" item and Scope 1.
   - Setup: `gl-pool` has min 0, max 2 and autoscaling on, one fleet Docker platform (linux/x64) and one pending job on `conn-project`.
   - Precondition: `assigned_queued_jobs == 1`, so the pool owns the job.
   - Then `choose_platform(..).1 == true`. After `spare_runner` it is `false`.
   - The job and the pool both carry the tags `["docker","arm64"]` on an x64 platform. This also pins the brief's rule that a GitLab job "is not filtered by `platform_supports_labels`": `arm64` is a system label and would drop the job from an x64 platform if it were filtered.
2. **`a_pool_with_github_and_gitlab_jobs_counts_both_and_lists_only_github_jobs`** (line 367). Covers the "Mixed pool" item and the Seam bullet "`platform_jobs` drops GitLab entries".
   - Setup: `gh-pool` is an organization pool linked to `conn-project`. It has one queued GitHub job (repository 10, queued now so it is not inside the priority window), one pending GitLab job and one spare runner with no repository.
   - Precondition: `assigned_queued_jobs == 2`.
   - `platform_jobs` returns exactly `[(10, ["self-hosted","linux"])]`, the GitHub job only. It is tested through `platform_jobs`; `repository_for_platform`'s visibility is not widened.
   - `choose_platform` is behind. With two jobs and one spare runner, the platform is behind only if the GitLab job is counted.

### Reconciler: `crates/gridops-reconciler/src/gitlab_lifecycle_tests.rs` (a full `run_pass` against the real database, a played fleet agent and `FakeGitLab`)

The regression lives in `provision`'s `NoDemand` guard together with `maybe_scale_up`'s desired count, so only a reconcile pass shows it. Per the brief, I extended the existing `Fleet` fixture and did not build a second one:
- `Fleet::new(requires_docker_socket)` now delegates to `Fleet::with_pool(requires_docker_socket, pool_sql)`. Its SQL and behaviour are unchanged, and all 10 existing `Fleet` tests still pass.
- The new constructor `Fleet::autoscaling(max_count)` gives `pool-gl` `min_count` 0 and autoscaling on, with desired 0 and queue scale factor 1 (the default).
- New methods: `Fleet::gitlab_jobs(statuses)`, which seeds `ACME` project jobs tagged `docker` through `gitlab_poll_tests::row`, and `Fleet::placed()`, which counts `workload_intents` rows for `pool-gl`, one per placed fleet runner.

3. **`a_pending_job_places_one_fleet_runner_for_a_pool_at_min_zero`** (line 2120). This is the #181 regression.
   - Setup: max 3, one pending job.
   - Pass 1: the precondition `desired_count == 1` holds (the scaler saw the demand, as the prod event did). Then `placed == 1` and the runner's row is `starting`.
   - Pass 2: `placed` is still 1, because the starting runner covers the job.
4. **`pending_jobs_scale_a_fleet_gitlab_pool_up_to_max_count`** (line 2150). This covers "Up to max".
   - Setup: max 2, three pending jobs, three passes.
   - The total is recorded after each pass and must always be at most 2, and the final total must be 2. The test doesn't assume one runner per pass: one pass may place both.
5. **`jobs_that_are_not_pending_place_no_fleet_runner_for_a_pool_at_min_zero`** (line 2179). This covers "No demand stays no demand".
   - Setup: one `created` job and one `manual` job. `desired_count` is forced to 2, as if it were still raised from earlier work, so that `provision` actually runs and only the not-behind guard stops it.
   - After two passes, `placed == 0` and there were no `CreateRunner` calls.
   - This is a **guard**: it is green before the fix, by design (see below).

## RED confirmation

Commands, run with `CARGO_BUILD_JOBS=2 RUST_TEST_THREADS=2` and the shared target dir:

```
cargo test -p gridops-core --test gitlab_pool_fleet
running 5 tests
test a_pending_gitlab_job_puts_a_fleet_platform_behind_until_a_spare_runner_covers_it ... FAILED
test a_pool_with_github_and_gitlab_jobs_counts_both_and_lists_only_github_jobs ... FAILED
(3 existing tests ok)
---- a_pending_gitlab_job_puts_a_fleet_platform_behind_until_a_spare_runner_covers_it stdout ----
panicked at crates/gridops-core/tests/gitlab_pool_fleet.rs:346:5:
one pending GitLab job and no runner: the platform is behind
---- a_pool_with_github_and_gitlab_jobs_counts_both_and_lists_only_github_jobs stdout ----
panicked at crates/gridops-core/tests/gitlab_pool_fleet.rs:419:5:
two queued jobs (one GitHub, one GitLab) and one spare runner: behind
test result: FAILED. 3 passed; 2 failed

cargo test -p gridops-reconciler          (whole crate)
test gitlab_lifecycle_tests::a_pending_job_places_one_fleet_runner_for_a_pool_at_min_zero ... FAILED
test gitlab_lifecycle_tests::pending_jobs_scale_a_fleet_gitlab_pool_up_to_max_count ... FAILED
panicked at crates/gridops-reconciler/src/gitlab_lifecycle_tests.rs:2130:5:
assertion `left == right` failed: one pending job places one fleet runner
  left: 0
 right: 1
panicked at crates/gridops-reconciler/src/gitlab_lifecycle_tests.rs:2166:5:
assertion `left == right` failed: three pending jobs bring the pool to max_count, per pass: [0, 0, 0]
  left: Some(0)
 right: Some(2)
test result: FAILED. 123 passed; 2 failed
```

Why each failure is the right reason:
- **Everything compiles.** No test fails on an import, a missing symbol or a setup error.
- **Core tests.** The ownership precondition (`assigned_queued_jobs`) passes in both tests, so the pool owns the jobs and the only failing step is `choose_platform`'s `behind`.
  - In the mixed test, the GitHub-only `platform_jobs` assertion runs first and passes. The spare-runner insert also works: before the fix, `behind` is `false` because the one GitHub job counted is covered by that spare. The failure is therefore that the GitLab job is not counted.
- **Reconciler regression.** The `desired_count == 1` precondition (line 2125) passes, so `maybe_scale_up` saw the pending job, which matches the prod event. The failure is the placement: 0 intents. That is the `NoDemand` guard acting on a platform that is never behind.
- **Fixture probe (temporary edit, reverted).** I set the `autoscaling` constructor to `min_count=1`, which is prod's 5:44 PM PST workaround. With that, test 3 passes in full, including the second-pass "no second runner" assertion and the `starting` status. So the fixture can place a fleet runner, and `min_count` 0 together with "not behind" is the only reason it doesn't. The file was restored afterwards: it contains `min_count=0,max_count` exactly once.
- **Guard test 5 is green before the fix, by design.** It pins behaviour the fix must not change: only `pending` jobs are fleet demand, and the `NoDemand` guard stays.
  - Probe (temporary `min_count=1`, reverted): it then fails with `left: 1, right: 0`. This shows that `provision` does run under the forced `desired_count=2`, and that only the not-behind guard keeps the count at 0.
  - The test will fail if F counts `created` or `manual` jobs as fleet demand, or weakens the guard.
- **Existing suites.** The other 123 reconciler tests pass, including all 10 that use `Fleet::new`, the whole of `gitlab_poll_tests`, and the `start_tests`. The 3 existing `gitlab_pool_fleet` tests pass.

Lint and format:
- `cargo fmt --check` is clean.
- `cargo clippy -p gridops-core --test gitlab_pool_fleet -- -D warnings` is clean.
- `cargo clippy -p gridops-core -p gridops-reconciler --all-targets -- -D warnings` on the local toolchain (1.99.1) reports 5 `assert!`-emptiness errors. All are in code this change doesn't touch: `autoscaling.rs:1810` and `:1896`, `autoscaling_gitlab_tests.rs:288` and `:384`, and `builtin_host.rs:119`. None is in the new tests. CI pins 1.98.1 because newer clippy brings new lints (`.github/workflows/ci.yml:88-93`). F should run the pre-PR clippy check on 1.98.1, or note this drift.

## Ready for F

RED is valid. F may implement against these tests. Four of the five tests fail on the feature assertion. The fifth is a guard that stays green and must stay green. GREEN means all five pass, with no change to any existing assertion. In particular, `fleet/admission_tests.rs`, `autoscaling_gitlab_tests.rs` (only its module doc line may change), the rest of `gitlab_pool_fleet.rs`, the whole reconciler crate, and `gridops-api` `tests/fleet_http/{recovery,placement}.rs` must stay unchanged.
