# Handoff-S: Phase 8 - Fleet GitLab pools scale on pending jobs, not only to min_count (#181)

**Date:** 2026-11-21
**Branch:** claude/gitlab-181-scaling-89ec07 (head 35c3891)
**Issue:** #181
**Brief:** docs/handoffs/181-brief.md
**Handoff-T reviewed:** docs/handoffs/181/handoff-T-red.md, docs/handoffs/181/handoff-T-green.md
**Handoff-A reviewed:** docs/handoffs/181/handoff-A.md (plan review, PASS); `docs/handoffs/181-diff-result-r1.md` (diff gate, PASS)
**Handoff-F reviewed:** docs/handoffs/181/handoff-F.md
**Operator-facing report:** N/A (no visual surface)

## A precondition
Confirmed: A returned PASS on the amended plan (0 block, 2 warn), and the diff gate (`181-diff-result-r1.md`) returned PASS with no BLOCK findings.

## T precondition
Confirmed: T-green reported zero blocking issues (`handoff-T-green.md`, "Blocking issues: None").

## Visual-diff-tool precondition
N/A. Rust workspace, no UI, no HTTP surface. No browser or pixel diff applies.

## Preview / spec sanity check
The brief is consistent with the code it cites. I re-checked the root cause against `autoscaling.rs`, `fleet/targets.rs` and `main.rs`: GitLab jobs raise `desired_count` through `assigned_queued_jobs`, never reach `choose_platform`, and `provision`'s `NoDemand` guard (`active >= min_count`) then blocks placement. The brief's seam (A-B1), its doc list and its tests list agree with each other. No source-of-truth defect, so no ADVISORY-HOLD.

## Visual diff results
N/A.

## Spec compliance

| Brief requirement | Checked | Compliant |
|---|---|---|
| Scope 1: `choose_platform` counts pending GitLab jobs the pool owns, scaled by `queue_scale_factor` | `choose_platform` adds each GitLab job to `queued[index]`; `deficit` multiplies `queued` by the factor (targets.rs ~1514-1520, ~1560) | YES |
| Ownership from `queued_job_owners`, no second label rule | `fleet_queued_jobs` makes one `queued_job_owners` call; no `gitlab_jobs` query | YES |
| Seam: `QueuedJob`, `queued_job_assignments`, `owned_queued_jobs`, `repository_capacities`, `provider_capacities`, `next_runner_*`, `queued_job_ownership`, `platform_jobs` signature unchanged | Diff touches none of them except moving `queued_job_assignments`' SQL into a private helper; its output is the same rows | YES |
| One `pub(crate)` accessor, single owners call, owner order, no re-sort, no second `gitlab_jobs` query | `fleet_queued_jobs` walks owners and places each GitHub row by job id, skipping ids the helper drops (A warn 2) | YES |
| Private job type, `Option` repository or enum | `pub(crate) enum FleetQueuedJob { Github(QueuedJob), Gitlab }`, defined once beside the accessor, not re-exported (A warn 1) | YES |
| GitLab job counts on every profile (not filtered by `platform_supports_labels`) | `FleetQueuedJob::Gitlab => true` in the platform filter | YES |
| Only a spare runner with no repository covers a GitLab job | `FleetQueuedJob::Gitlab => repository.is_none()` (targets.rs:1456) | YES in code. **No test pins it** (see R2) |
| Adds nothing to `waiting` or `prioritised` | `let FleetQueuedJob::Github(job) = job else { continue }` after `queued[index] += 1` | YES |
| `attribute_queued_jobs` doc says a `None` spare is an org GitHub runner or a GitLab runner, and why that is safe | Doc paragraph present, cites `fleet::provision`'s GitLab-connection-first check | YES (readability note A3) |
| `platform_jobs` drops GitLab entries after attribution | `.filter_map(\|(job, _)\| job.github())` | YES |
| Doc comments updated: `queued_job_owners`, `attribute_queued_jobs`, `choose_platform`, `platform_jobs`, new accessor, `autoscaling_gitlab_tests.rs` module doc (doc line only) | All present; the module-doc diff changes no assertion | YES |
| Scope 2: no change to the `provision` loop or `NoDemand` guard | `git diff master...HEAD` does not touch `main.rs` or `fleet.rs` | YES |
| Scope 3: GitHub-only results unchanged; existing assertions unchanged | For GitHub-only pools owner order equals the helper's `ORDER BY wj.created_at,wj.id` (`github_job_candidates` order; the merge re-sort runs only with GitLab candidates). No existing assertion edited | YES |
| No migration, no new dependency, no `Cargo.lock` change | Confirmed in the diff stat | YES |
| CHANGELOG entry under Unreleased | Under `## Unreleased` > `### Fixed`; states the Bitbucket finding | YES |
| Bitbucket unchanged; PR must state the finding | Untouched. CHANGELOG states it. **O must also put it in the PR body** | YES |
| Tests: #181 regression, up to max, no-demand, core, mixed pool, no test-only `pub` items | All five exist; only `gitlab_poll_tests::row` became `pub(super)` inside a `#[cfg(test)]` module | PARTLY: the no-demand test is vacuous (R1) |

## Quality audit

| Area | Result | Notes |
|---|---|---|
| API consistency | N/A | No public signature changed |
| Error handling | PASS | DB errors propagate with `?`; an owned GitHub id the helper drops is skipped, as before |
| UI/UX match to spec | N/A | No UI |
| Accessibility | N/A | No UI |
| Architecture gate | PASS | A PASS (plan), diff gate PASS. Layers and dependency direction unchanged |
| Code organization | PASS | One accessor, one enum, one shared helper; no duplicated SQL |
| Security | N/A | No new input or auth surface |
| Performance | PASS | One `queued_job_owners` call per attribution, as before; one extra `HashMap` over the pool's own owned jobs |
| Visual regression | N/A | No UI |
| Naming consistency | PASS | `fleet_queued_jobs`, `FleetQueuedJob`, `github_queued_jobs`, `github()` follow `KeyedJobOwner::github` |
| Test quality (`testing/test-quality.md` §7) | FAIL | R1 (one test cannot fail for the right reason) and R2 (a binding seam clause is unpinned). Details below |

Verification I ran myself (all on head 35c3891, `CARGO_BUILD_JOBS=2 RUST_TEST_THREADS=2`, shared target dir, tree clean afterwards):
- `cargo fmt --check`: clean.
- `cargo test -p gridops-core --test gitlab_pool_fleet`: 5 passed.
- `cargo test -p gridops-reconciler gitlab_lifecycle_tests`: 36 passed, including the three new tests.
- Two temporary mutations, each reverted with `git checkout -- <file>` (`git status --short` empty afterwards). Results under "Test-quality audit".
- I did not re-run the full core, reconciler or `gridops-api` suites, or clippy. I rely on F and T for those (core 421 passed, reconciler 125 passed, `fleet_http` 149 passed, 1 host-dependent `release_preflight` failure that F showed also fails on master, clippy on 1.98.1 clean).

## Test-quality audit (rubric §7, every new test)

| Test | 1 Names a behaviour | 2 Fails in isolation for the right reason | 3 Tier / non-duplicative | 4 Behavioural | Result |
|---|---|---|---|---|---|
| core `a_pending_gitlab_job_puts_a_fleet_platform_behind_until_a_spare_runner_covers_it` | yes | yes (RED before the fix; T's GitHub-only mutation fails it) | cheapest tier that runs the rule | yes | OK |
| core `a_pool_with_github_and_gitlab_jobs_counts_both_and_lists_only_github_jobs` | yes | yes | yes; overlap with the test above is small and the `platform_jobs` assertion is unique | yes | OK |
| reconciler `a_pending_job_places_one_fleet_runner_for_a_pool_at_min_zero` | yes | yes (RED: 0 vs 1) | needed to show the `NoDemand` guard and `maybe_scale_up` together; brief-required | yes | OK (A2) |
| reconciler `pending_jobs_scale_a_fleet_gitlab_pool_up_to_max_count` | yes | final total of 2 fails RED (`[0,0,0]`) | brief-required | yes | OK (A2) |
| reconciler `jobs_that_are_not_pending_place_no_fleet_runner_for_a_pool_at_min_zero` | claims to pin the `NoDemand` guard | **NO** | would duplicate upstream ownership tests if it worked | yes | **FAIL (R1)** |

Mutation results:
- **M1, `provision`'s guard disabled** (`if active >= pool.min_count` changed to `if false && active >= pool.min_count`, main.rs ~2450). `jobs_that_are_not_pending_place_no_fleet_runner_for_a_pool_at_min_zero` still **passes**. So it cannot fail if the guard is removed.
- **M2, any spare runner covers a GitLab job** (`FleetQueuedJob::Gitlab => repository.is_none()` changed to `=> true`, targets.rs:1456). All five tests in `gitlab_pool_fleet.rs` still **pass**. So nothing pins "never one registered to a GitHub repository".

## Findings requiring rework

### R1 (test-only): the "no demand stays no demand" test never reaches the guard it claims to pin

`crates/gridops-reconciler/src/gitlab_lifecycle_tests.rs:2179`, `jobs_that_are_not_pending_place_no_fleet_runner_for_a_pool_at_min_zero`.

Why it is vacuous:
- The test forces `desired_count=2` "so that `provision` actually runs". But a pass runs `maybe_scale_down` (main.rs:1019, body ~1533-1565) before it reads `desired_count` (main.rs ~1020-1029).
- For this pool, scale-down fires: autoscaling is on; `desired_count` 2 is above `min_count` 0; no job is owned (`created` and `manual` are not scaler demand); there is no busy runner; and with no runners `last_activity` is 0, so the idle timeout (default 5 minutes, `migrations/0003_autoscaling.sql:3`) has long elapsed.
- It resets `desired_count` to `min_count` (0). The loop `for _ in active.len()..desired` then never calls `provision`. The `NoDemand` guard is never evaluated.
- So the test's doc comment ("`provision` runs, and its platform is not behind") and T-red's statement that the test "will fail if F ... weakens the guard" are both wrong. M1 proves it: the guard can be deleted and the test stays green.
- T-red's `min_count=1` probe did not show otherwise. With `min_count=1`, scale-down resets desired to 1 and `provision` runs, because `0 >= 1` is false. That is a different path.

What the test can catch today: only a change that makes `created` or `manual` jobs reach `queued_job_owners` and `assigned_queued_jobs`. That is upstream of #181 and already covered by `autoscaling_gitlab_tests.rs`.

Required repair (T, tests only; no production change). Either of these meets the brief's "No demand stays no demand" item:
1. **Make `provision` really run.** Keep the desired count at 2 through the pass, for example by giving the pool an `idle_timeout_minutes` far beyond the epoch-millisecond age (for instance 100000000), so `maybe_scale_down` returns early. Then show by mutation, as I did in M1, that disabling the `NoDemand` guard makes the test fail (a runner is placed).
2. **Move the rule to the cheapest tier.** In `crates/gridops-core/tests/gitlab_pool_fleet.rs`, seed a `created` and a `manual` job (give `pending_gitlab_job` a status parameter) and assert `choose_platform` is not behind. Show that counting a non-pending job in `fleet_queued_jobs` fails it. Then delete the reconciler test, per rubric §8, saying so in the handoff.

Whichever is kept must be shown to fail for the right reason, and the shown mutation must be named in the handoff. Also correct the doc comment and the T-red claim.

### R2 (test-only): the seam rule "only a spare runner with no repository covers a GitLab job" is untested

`crates/gridops-core/src/fleet/targets.rs:1456`. The brief makes this binding ("never one registered to a GitHub repository"). M2 shows the rule can be dropped with every test still green. The risk is real in a pool that is both an organization GitHub pool and linked to GitLab: an idle runner registered to a GitHub repository would hide GitLab demand and the pool would stay under-scaled, which is the #181 symptom in a different shape.

Required repair (T, tests only): one core test in `gitlab_pool_fleet.rs`. A pool with one pending GitLab job and a starting spare runner registered to GitHub repository 10 (extend `spare_runner` with an optional repository; the mixed test already seeds repository 10) stays behind. Then show that M2 fails it. This can be one extra step in the mixed-pool test or a small separate test; T decides, subject to rubric §3 (one behaviour per test).

## Scope check
F delivered the brief's scope with no over- or under-delivery in production code: `autoscaling.rs`, `fleet/targets.rs`, one module-doc line in `autoscaling_gitlab_tests.rs`, and `CHANGELOG.md`. `main.rs`, `fleet.rs`, the migrations, `docs/gitlab-runners.md` and the Bitbucket path are untouched, as the brief requires. The production change is correct as written; both findings are in tests.

## Verdict

REWORK, **TEST-ONLY**: no `src/` change is required. The two findings are in test files (`gitlab_lifecycle_tests.rs`, `tests/gitlab_pool_fleet.rs`).

1. **R1:** repair or relocate `jobs_that_are_not_pending_place_no_fleet_runner_for_a_pool_at_min_zero` (gitlab_lifecycle_tests.rs:2179) so it can fail when the `NoDemand` guard is removed (or delete it in favour of a core `choose_platform` assertion, option 2). Fix its doc comment.
2. **R2:** add the test that a spare runner registered to a GitHub repository does not cover a GitLab job (targets.rs:1456), and show M2 fails it.

Re-running T-green and the S gate after the repair should be enough. No production file needs to change.

## Advisory notes (non-blocking)

- **A2.** In `a_pending_job_places_one_fleet_runner_for_a_pool_at_min_zero`, the pass-2 "no second runner" assertion holds because `desired_count` stays 1 and `active.len() < desired` is false, so `provision` is not called in pass 2 at all. Coverage by a starting runner is what the core test pins (`behind` goes false after `spare_runner`). The assertion is true and brief-required; only its comment ("the starting runner covers the job") over-attributes. Likewise, in the max-count test the `<= max_count` check is guaranteed by `scale_up_target(...).min(max_count)`; the final total of 2 is the assertion that carries #181. No change required.
- **A3.** `attribute_queued_jobs`'s doc comment (targets.rs ~1382-1393): the GitLab-safety argument was inserted mid-paragraph, so "Otherwise it needs room: runners waiting for a machine take" now reads as following "a pool not linked to GitLab owns no GitLab job", and one line runs well past the usual width. Splitting it into two paragraphs would read better. The diff gate raised the same point (W-1). It changes no behaviour, so I did not make it a finding; fix it on the next production touch.
- **For O (PR body).** The brief requires the PR to state the Bitbucket finding: no Bitbucket queued-job signal exists and Bitbucket runners can't run on fleet machines, so only GitLab fleet demand was missing. The CHANGELOG entry already says so; repeat it in the PR description.
- **Known, unrelated.** `gridops-api` `release_preflight::the_release_preflight_fails_early_without_a_macos_arm64_machine` fails on a macOS arm64 dev host. F reports the same on master 42e8474. I did not re-run it. The pending-job path can now also trigger the existing `Docker`-without-`requires_docker_socket` failure in `gitlab_lifecycle::fleet_target` (A recorded it as out of scope).
