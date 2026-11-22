# Handoff-S (re-audit): Phase 8 - Fleet GitLab pools scale on pending jobs, not only to min_count (#181)

**Date:** 2026-11-21
**Branch:** claude/gitlab-181-scaling-89ec07 (head 728a37d)
**Issue:** #181
**Brief:** docs/handoffs/181-brief.md
**Previous S verdict:** docs/handoffs/181/handoff-S.md (REWORK, test-only: R1, R2)
**Handoff-T reviewed:** docs/handoffs/181/handoff-T-rework.md (commit 728a37d); earlier handoff-T-red.md, handoff-T-green.md
**Handoff-A reviewed:** docs/handoffs/181/handoff-A.md (plan PASS); docs/handoffs/181-diff-result-r1.md (diff gate PASS)
**Handoff-F reviewed:** docs/handoffs/181/handoff-F.md; F's no-change rework entry in decisions.md
**Diff audited:** `git diff 42e8474..HEAD` (full), with focus on `git diff 35c3891..HEAD` (the rework)
**Operator-facing report:** N/A (no visual surface)

## A precondition
Confirmed. A passed the plan and the diff gate. The rework changed no production file, so A's diff verdict still applies: `git diff 35c3891..HEAD -- crates/` touches only `crates/gridops-core/tests/gitlab_pool_fleet.rs` and `crates/gridops-reconciler/src/gitlab_lifecycle_tests.rs`.

## T precondition
Confirmed. `handoff-T-rework.md` reports "Blocking issues: None".

## Visual-diff-tool precondition
N/A. Rust workspace with no UI.

## Preview / spec sanity check
Unchanged from the first audit: no defect in the brief. One correction is on me, not the brief. My R2 suggestion ("assert the platform stays behind") could not detect the defect, as T showed (see R2 below).

## Visual diff results
N/A.

## Re-verification of R1 and R2 (my own runs)

Environment: `CARGO_BUILD_JOBS=2 RUST_TEST_THREADS=2`, the shared target dir from `.cargo/config.toml`, and `CARGO_TARGET_DIR` unset. I reverted each mutation with `git checkout -- <file>`, and `git status --short` was empty after each one.

### R1: closed
`gitlab_lifecycle_tests::jobs_that_are_not_pending_place_no_fleet_runner_for_a_pool_at_min_zero` (gitlab_lifecycle_tests.rs ~2185).
- The fix sets `idle_timeout_minutes=100000000`, which keeps `maybe_scale_down` from resetting `desired_count`. So `provision` really runs against 0 active runners, and the `NoDemand` guard is what decides.
- The new `desired == 2` assertion keeps the test from silently becoming vacuous again.
- The doc comment now describes the real path.
- **M1** (main.rs:2450, `if active >= pool.min_count {` changed to `if false && active >= pool.min_count {`): the test **FAILS** at gitlab_lifecycle_tests.rs:2205 with `left: 2, right: 0`. The other two #181 reconciler tests pass. It fails for the right reason.
- Unmutated: passes.

### R2: closed. T's shape is valid; mine was not.
`a_spare_runner_registered_to_a_github_repository_does_not_cover_a_gitlab_job` (tests/gitlab_pool_fleet.rs ~451).

**Why my suggested shape could not work.** I checked T's argument against `choose_platform` (targets.rs ~1500-1560), and it holds:
- `queued[index] += 1` runs for every attributed job, whichever spare covered it.
- In `deficit`, an idle runner registered to a repository counts as `active` only when that repository is in `waiting[index]`.
- So on one platform, with a GitLab job and a repository-10 spare: correct code gives `queued=1`, `active=0`, behind. M2 also gives `queued=1`, `active=0`, behind.
- The flag is the same either way. The coverage rule only changes which platform a job is attributed to, so it can only be observed with two or more platforms.

**Does T's test pin "only a spare runner with no repository covers a GitLab job"?** It pins the exclusion half, "never one registered to a GitHub repository". That is the half the brief marks binding, and the half R2 asked for. I traced the setup:
- **Setup:** one pool with two platforms (x64 first, arm64 second) and two jobs. The GitLab job is older (queued at 10) and the GitHub job belongs to repository 10. Both jobs can run on both platforms, so the stable `sort_by_key(len > 1)` keeps owner order and the GitLab job is attributed first. The only runner is a repository-10 spare on arm64.
- **Correct code:** the GitLab job skips the repository-10 spare. With no room and no machine it falls back to the first platform, x64. The GitHub job then takes the repository-10 spare on arm64. Result: `platform_jobs(arm64) == [10]` and `platform_jobs(x64) == []`.
- **Under M2:** the GitLab job takes the repository-10 spare on arm64, and the GitHub job is pushed to x64. Both assertions flip.
- **Other mutations caught:** the test also fails under any other mutation that lets a repository-registered runner cover a GitLab job, for example `=> repository.is_some()`, because the trace is identical.
- **Assertions:** the observables are the public `platform_jobs` results, not internals. Both the arm64 and x64 assertions carry signal.
- **Order dependence:** the test depends on owner order and the first-platform fallback. T documented both in the doc comment and in `evidence.md`.
- **Robustness:** if owner order ever flipped (GitHub job first), the test would still pass on correct code. It would then lose sensitivity, not turn red falsely, so it is not brittle in the harmful direction.

**M2** (targets.rs:1456, `FleetQueuedJob::Gitlab => repository.is_none(),` changed to `=> true,`): the R2 test **FAILS** at gitlab_pool_fleet.rs:538 with `left: [], right: [10]`. The other 5 tests pass. Unmutated: all 6 pass.

**Extra probe, M2b** (same line changed to `=> false,`: no spare ever covers a GitLab job): all 6 core tests still **pass**. So the inclusion half ("a `None` spare does cover a GitLab job") is not pinned by attribution.
- On one platform, `deficit` counts a `None` spare as active anyway, so `behind` still drops to false. That is why the existing "until a spare runner covers it" test stays green.
- The behavioural effect of M2b appears only on a multi-platform GitLab pool: the job is attributed by room instead of to the spare's platform, which could start an extra runner on another platform.
- R2 did not ask for this, and the brief's required core test ("false once a spare `None` runner covers it") exists and is satisfied as the brief words it. I record it as advisory A4, not a finding.

## Done-when (brief) confirmation

| Brief done-when | Evidence | Met |
|---|---|---|
| A GitLab pool with min 0 on a fleet machine and one pending job places exactly one runner | `a_pending_job_places_one_fleet_runner_for_a_pool_at_min_zero`: desired 1, `workload_intents` count 1, runner `starting`, and still 1 after a second pass. T-red showed it RED before the fix (0 vs 1). Passes now. | YES |
| It scales up to max_count | `pending_jobs_scale_a_fleet_gitlab_pool_up_to_max_count`: 3 pending jobs, max 2. Per-pass totals never exceed 2, and the final total is 2 (RED before the fix: `[0,0,0]`). Passes now. | YES |
| No demand stays no demand | Repaired R1 test passes, and M1 proves it reaches the guard. | YES |
| GitHub fleet scaling unchanged | No existing assertion is edited in `42e8474..HEAD`. The only edits to existing test files are a module-doc line and one `pub(super)` helper. The full core and reconciler suites pass. The `fleet_http` suite, which covers GitHub placement and recovery, passes apart from 2 known non-#181 failures (see below). `main.rs` and `fleet.rs` are untouched, and for a GitHub-only pool owner order equals the old SQL order (first audit). | YES |

## Spec compliance
Unchanged from `handoff-S.md`: every Scope 1-3 item is compliant in production code. The production code is identical (`git diff 35c3891..HEAD -- crates/` touches tests only). The row that was "PARTLY" (the vacuous no-demand test) is now YES, and the "only a `None` spare covers it" row now has a test for its binding half.

## Quality audit

| Area | Result | Notes |
|------|--------|-------|
| API consistency | N/A | No public signature changed |
| Error handling | PASS | Unchanged |
| UI/UX match to spec | N/A | No UI |
| Accessibility | N/A | No UI |
| Architecture gate | PASS | A PASS (plan and diff). The rework is test-only |
| Code organization | PASS | `spare_runner` gained one `Option<i64>` parameter; its two existing callers pass `None` |
| Security | N/A | No new surface |
| Performance | PASS | Unchanged |
| Visual regression | N/A | No UI |
| Naming consistency | PASS | The new test name states the rule it pins |
| Test quality (`testing/test-quality.md` §7) | PASS | Both repaired or added tests name one behaviour, fail in isolation for the right reason (M1, M2), sit at the cheapest sufficient tier (reconciler for the guard in `provision`; core for attribution) and assert behaviour. The R2 test doesn't duplicate the mixed-pool test (that one uses a `None` spare on one platform). Proportionate: one test repaired, one added. No delete-or-merge findings |

### Verification runs (head 728a37d)
- `cargo fmt --all -- --check`: exit 0.
- `cargo test -p gridops-core`: 422 passed, 0 failed, 1 ignored (pre-existing). `gitlab_pool_fleet`: 6/6.
- `cargo test -p gridops-reconciler`: 125 passed, 0 failed. `gitlab_lifecycle_tests`: 36/36.
- `cargo +1.98.1 clippy -p gridops-core -p gridops-reconciler --all-targets --locked -- -D warnings` (CI toolchain): clean.
- `cargo test -p gridops-api --test fleet_http`: 148 passed, 2 failed:
  - `release_preflight::the_release_preflight_fails_early_without_a_macos_arm64_machine` is the known host-dependent failure, which F reproduced on master 42e8474.
  - `busy_database::a_result_the_api_cannot_take_still_ends_the_session` is an agent-session fault-proxy timing test with no autoscaling or attribution path. Re-run on its own, the `busy_database` module passed 3/3 (6/6 each run; host load average about 5.6). F and T saw `fleet_http` pass on the same production code. I classify it as a load-dependent flake, not a #181 regression.
- Mutations M1, M2 and M2b were each reverted, and `git status --short` was empty afterwards.

## Scope check
No change from the first audit. The rework touched only the two test files and handoff docs, which is in scope for a test-only REWORK. No production, migration, dependency or Bitbucket change.

## Verdict

PASS. All acceptance criteria are met, the implementation matches the spec, and quality is acceptable. Ready for O to commit and open the PR.

- R1 closed: the no-demand test now reaches the `NoDemand` guard and fails under M1.
- R2 closed: T's two-platform `platform_jobs` test validly pins "never a spare registered to a GitHub repository" and fails under M2. My single-platform `behind` suggestion could not have detected the defect, and T was right to deviate from it.

## Advisory notes (non-blocking)
- **A4 (new).** The inclusion half of the coverage rule (a `None` spare covers a GitLab job) survives mutation M2b (`=> false`).
  - It matters only for a GitLab pool with several fleet platforms, where the job could be attributed away from the spare's platform and start an extra runner.
  - A follow-up test would mirror the R2 test: two platforms, a GitLab job, and a `None` spare on the second platform. Assert the job doesn't make the first platform behind, through `choose_platform`'s returned platform.
  - This is not required by the brief or by R2. Leave it to a later story if O wants it.
- **A2, A3** from `handoff-S.md` still stand as advisory: an over-attributing comment in the regression test, and the mid-paragraph doc comment in `attribute_queued_jobs`.
- **For O (PR body).** State the Bitbucket finding from the brief: no Bitbucket queued-job signal exists, and Bitbucket runners can't run on fleet machines, so only GitLab fleet demand was missing.
- **For O (pipeline).** A self-reported test-only REWORK was routed to F. It is already recorded in decisions.md (#678). Nothing for this story.
- **Flake.** `fleet_http::busy_database::a_result_the_api_cannot_take_still_ends_the_session` failed once under full-suite load and passed in 3 of 3 isolated reruns. It is unrelated to #181. Track it separately if it recurs in CI.
