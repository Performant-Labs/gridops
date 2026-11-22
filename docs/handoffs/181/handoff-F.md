# Handoff-F: Phase 6 - Fleet GitLab pools scale on pending jobs, not only to min_count

**Date:** 2026-11-21
**Branch:** claude/gitlab-181-scaling-89ec07
**Issue:** #181

## What was done
- `crates/gridops-core/src/autoscaling.rs`
  - Moved the SQL and row mapping of `queued_job_assignments` into a private helper, `github_queued_jobs(database, pool_id, owned)`. The helper also selects `wj.id`, so it returns `(job_id, QueuedJob)`. `queued_job_assignments` now calls it and drops the id, so its output is unchanged.
  - New `pub(crate) enum FleetQueuedJob { Github(QueuedJob), Gitlab }`, with a `github()` filter in the style of `KeyedJobOwner::github`.
  - New `pub(crate) async fn fleet_queued_jobs(database, pool_id)`. It calls `queued_job_owners` once and keeps this pool's owners in owner order. It fetches the GitHub rows through the helper and puts each one in its owner's position by job id; an owned id the helper didn't return is skipped. Each GitLab owner becomes `FleetQueuedJob::Gitlab`.
  - Doc comments updated: `queued_job_owners` (GitLab jobs are now listed by `fleet_queued_jobs` too), `owned_queued_jobs` and `queued_job_assignments` (each gets a pointer to the new accessor).
- `crates/gridops-core/src/fleet/targets.rs`
  - `attribute_queued_jobs` reads `fleet_queued_jobs` and returns `Vec<(FleetQueuedJob, usize)>`.
  - A GitLab job's platforms are all the profiles; `platform_supports_labels` doesn't filter it.
  - Only a spare runner with no repository covers a GitLab job.
  - The doc comment explains why a `None` spare may stand for a GitLab runner.
  - `choose_platform` adds a GitLab job to `queued[index]` only, never to `waiting` or `prioritised`, and its doc comment says so.
  - `platform_jobs` filters with `.filter_map(|(job, _)| job.github())` after attribution. Its doc says it returns GitHub jobs only.
- `crates/gridops-core/src/autoscaling_gitlab_tests.rs`: only the module doc comment changed, as the brief's Scope 1 asks; no assertion changed.
- `CHANGELOG.md`: a Fixed entry under Unreleased for #181, with the Bitbucket finding.

## Design decisions
- **One enum that wraps `QueuedJob`, not a struct with `Option<i64>` (A warn 1).**
  - The brief allowed either. The type has to cross from `autoscaling.rs` to `fleet/targets.rs`, so it is `pub(crate)` and defined beside the accessor. It is not re-exported in `lib.rs`.
  - A struct would have copied `QueuedJob`'s fields, and `platform_jobs` would then have to convert it back. The enum's `Gitlab` variant carries nothing, because attribution doesn't filter it by label and `choose_platform` doesn't prioritise it.
  - `targets.rs` has no second mapping type.
- **Owner order by id (A warn 2).**
  - The accessor walks the owners list and pulls each GitHub row from a `HashMap` keyed by job id. It does not concatenate the two platforms' jobs and does not re-sort.
  - Owned ids that the helper's re-check (`wj.status='queued' AND wr.status!='completed'`) drops are skipped, as `queued_job_assignments` drops them today.
  - For a GitHub-only pool, owner order equals the helper's `ORDER BY wj.created_at,wj.id` (see `evidence.md`), so attribution input is the same sequence as before.
- **Pending only.** GitLab entries come only from `queued_job_owners`, whose candidates are tier 1 (`pending`) with `every_waiting_status=false`. There is no extra status check and no second `gitlab_jobs` query.
- **`behind` in the reconciler.** `main.rs` (the pass loop and the `NoDemand` guard) and `fleet.rs` are untouched. The guard now gets a correct `behind`.

## Reuse / extend-vs-new
- The brief's recommendation is to extend, and this extends the named analogue: GitHub fleet demand through `attribute_queued_jobs` / `choose_platform` / `platform_jobs`, with ownership from `queued_job_owners`.
- The one new function, `fleet_queued_jobs`, and the one new type, `FleetQueuedJob`, are what the brief's Seam (A-B1) asks for.
- The SQL that used to be inline in `queued_job_assignments` was moved into a shared private helper, not duplicated.

## Architecture notes for A
- Layers touched: gridops-core only. The `fleet::targets` → `autoscaling` dependency direction is unchanged; it already called `crate::queued_job_assignments`.
- New crate-internal items: `autoscaling::FleetQueuedJob` and `autoscaling::fleet_queued_jobs`, both `pub(crate)`. No new `pub` item.
- Unchanged public signatures: `QueuedJob`, `queued_job_assignments`, `owned_queued_jobs`, `repository_capacities`, `provider_capacities`, `next_runner_*`, `queued_job_ownership`, `choose_platform` and `platform_jobs`.
- The private `attribute_queued_jobs` changed its return type from `Vec<(crate::QueuedJob, usize)>` to `Vec<(FleetQueuedJob, usize)>`.
- No schema change and no migration. No new dependency.
- `archChanged` is reported true, because a private return type changed and a crate-internal accessor was added. The public interfaces are unchanged.

## Deviations from spec / wireframe
None. Wireframe: N/A.

## Tier 1 self-check (incl. tests now GREEN)
All runs used `CARGO_BUILD_JOBS=2 RUST_TEST_THREADS=2` and the shared target dir.

```
cargo test -p gridops-core --test gitlab_pool_fleet
test a_pending_gitlab_job_puts_a_fleet_platform_behind_until_a_spare_runner_covers_it ... ok
test a_pool_with_github_and_gitlab_jobs_counts_both_and_lists_only_github_jobs ... ok
test result: ok. 5 passed; 0 failed

cargo test -p gridops-core            (every target, summed)
passed 421 failed 0                   (lib: 310 passed, including autoscaling, autoscaling_gitlab_tests and fleet/admission_tests)

cargo test -p gridops-reconciler
test gitlab_lifecycle_tests::a_pending_job_places_one_fleet_runner_for_a_pool_at_min_zero ... ok
test gitlab_lifecycle_tests::jobs_that_are_not_pending_place_no_fleet_runner_for_a_pool_at_min_zero ... ok
test gitlab_lifecycle_tests::pending_jobs_scale_a_fleet_gitlab_pool_up_to_max_count ... ok
test result: ok. 125 passed; 0 failed

cargo test -p gridops-api --test fleet_http
test result: FAILED. 149 passed; 1 failed
  failed: release_preflight::the_release_preflight_fails_early_without_a_macos_arm64_machine
  It fails identically on unchanged master 42e8474 (run in a temporary worktree, since removed):
  "PASS  macos-machine: macOS ARM64 machine ready ... fleet-host". The fixture machine reports this
  Mac's OS and architecture, so the test can only pass on a non-macOS-arm64 host such as CI's Linux
  runner. It is unrelated to #181. Every recovery.rs and placement.rs test passes.

cargo fmt --check                     exit 0
cargo +1.98.1 clippy --workspace --all-targets --locked -- -D warnings -A clippy::unused_async_trait_impl
    Finished (clean; this is CI's exact command, .github/workflows/ci.yml:113, on CI's pinned toolchain)
```

## Evidence appendix
`docs/handoffs/181/evidence.md` has 7 facts:
- the GitLab runner placed with no repository;
- the GitHub candidate order, and that the merge re-sorts only when GitLab candidates exist;
- that only `pending` GitLab jobs are owners;
- the `None`-runner rule in the deficit;
- the `NoDemand` guard;
- the pass loop.

## Tests that look wrong (for T)
None. One environment-dependent test, which is not T's and predates #181, is noted under Known issues.

## Known issues
- `gridops-api` `release_preflight::the_release_preflight_fails_early_without_a_macos_arm64_machine` fails on a macOS arm64 dev host, and also on master. It is unrelated to this change.
- Plain `cargo clippy ... -D warnings` without CI's `-A clippy::unused_async_trait_impl` reports 2 errors in `gridops-host-agent` (`helper.rs:188` and `native.rs:200`). This change doesn't touch them, and CI allows that lint.
- The `Docker` profile without `requires_docker_socket` failure that A noted (`gitlab_lifecycle::fleet_target`) can now be triggered by a pending job as well as by `min_count`. A recorded it as existing behaviour and out of scope.

## Files changed
- `crates/gridops-core/src/autoscaling.rs`
- `crates/gridops-core/src/fleet/targets.rs`
- `crates/gridops-core/src/autoscaling_gitlab_tests.rs` (module doc comment only, as the brief asks)
- `CHANGELOG.md`
