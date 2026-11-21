# Handoff-S: Phase 8 - GitLab reconciler runner lifecycle (register, roll back, clean up) (#139)

**Date:** 2026-11-20
**Branch:** claude/gitlab-139-lifecycle-53c8ce (audited head 05131a9; base b22c451)
**Issue:** #139 (brief: `docs/handoffs/139-brief.md`, "Done when (consolidated)" is authoritative)
**Handoff-T reviewed:** `docs/handoffs/139/handoff-T-green.md` (and `handoff-T-red.md`)
**Handoff-A reviewed:** `docs/handoffs/139/handoff-A.md` (plan gate, round 5, PASS) and `docs/handoffs/139/handoff-A-dup.md` (anti-duplication gate, PASS)
**Handoff-F reviewed:** `docs/handoffs/139/handoff-F.md` (+ `evidence.md`)
**Operator-facing report:** N/A (no visual surface)

## A precondition
Confirmed. Handoff-A (round 5) is PASS with 0 blocks; its two warnings were taken up by F (probe is per pass; `load_pool` is used only outside the start loop). Handoff-A-dup on the finished diff is PASS (two info/warn notes, neither blocking).

## T precondition
Confirmed. Handoff-T-green reports "Blocking issues: None". F edited no test file after T-red (`git diff 2aa6002 --name-only` shows none), and T's mutation spot-check (replace the `delete_runner` call in `gitlab_lifecycle::deregister` with `Ok(())`) failed the expected test.

## Visual-diff-tool precondition
N/A. This is a Rust backend change with no rendered surface (no UI, no Playwright suite). The Browser-tool and Visual-diff-tool preconditions do not apply, and no HTML report is produced.

## Preview / spec sanity check
Done (non-visual form). The brief's contract is internally consistent and matches the code it names: I re-read the cited facts (`GITLAB_EXIT_REASONS`, `GITLAB_DEFAULT_WAIT_TIMEOUT_SECS`, `reported_gone`, `abandon_unstarted`, `0047_gitlab.sql` cascades, `save_pool_fleet`'s Docker-only `requires_docker_socket`, line 628 of `targets.rs`) and found no defect in the source of truth. The one thing that has moved is the repository around the brief, not the brief: see "Merge readiness" below. No ADVISORY-HOLD.

## What I ran myself (not only read from T/F)

| Check | Result |
|---|---|
| `cargo fmt --all -- --check` | clean |
| `cargo test -p gridops-reconciler` | 94 passed, 0 failed (68 existing + 26 new `gitlab_lifecycle_tests`), including `start_tests::a_successful_probe_clears_the_blocked_state_in_the_same_pass` unmodified |
| `cargo test -p gridops-core --lib` | 264 passed, 0 failed |
| `cargo test -p gridops-core --test gitlab_pool_fleet` | 3 passed |
| `cargo test -p gridops-api --no-fail-fast` | 60 + 32 + 143 passed, 1 failed: `release_preflight::the_release_preflight_fails_early_without_a_macos_arm64_machine`. This is the environmental failure F and T report (the fixture "Linux machine" reports the build host's OS; this workstation is Darwin). Not touched by this change. |
| CI's exact clippy gate: `cargo +1.98.1 clippy --workspace --all-targets --locked -- -D warnings -A clippy::unused_async_trait_impl` (toolchain 1.98.1 is what `.github/workflows/ci.yml` pins; separate target dir) | finished with 0 warnings, 0 errors |
| Not re-run by me | `gridops-manager` / `gridops-tart-agent` tests. F and T report 11 `SUN_LEN` failures that also occur at 2aa6002; neither crate is touched by this diff. |

The 17 clippy findings F and T call "toolchain drift" are confirmed as drift: they appear only on local clippy 0.1.99. The Done-when's `cargo clippy --workspace --all-targets -- -D warnings` item passes under the project's pinned toolchain, so F's "known issue" does not stand against the gate CI will run.

## Spec compliance

Every item of "Done when (consolidated)" has a test that I confirmed passes. Tests are in `crates/gridops-reconciler/src/gitlab_lifecycle_tests.rs` unless named otherwise.

| # | Requirement | Compliant | Evidence |
|---|---|---|---|
| 1 | Register, manager path (Docker, Tart): tags = pool labels, `run_untagged`, `gridops-runner:<instance>:<uuid>` marker, row columns, D10 request shape (`platform/mode/runtime/gitlab{url,waitTimeoutSeconds}/gitlabRunnerToken`, four keys absent), token secrecy | YES | `a_docker_start_registers_a_gitlab_runner_and_hands_the_manager_its_token`, `a_tart_start_registers_a_gitlab_runner_and_sends_the_tart_image`. Code: `gitlab_lifecycle::register`, `start_admitted_runner` GitLab arm; `gitlab_runner_id` set by the UPDATE after the POST, as Bitbucket's is. |
| 2 | Register, fleet path: `create_runner` in `register_and_start`, sealed `GitlabRunnerMaterial::GitlabRunner{token,url,wait_timeout_seconds:600}`, target = selected connection's own | YES | `a_fleet_start_registers_the_runner_and_seals_the_gitlab_material`. Material is serde-typed (`fleet_material`), stored through the shared `start_with_material` (verbatim extraction of `store_bootstrap` + `issue_command`). |
| 3 | Tart-only pool (`docker_image=''`) registers and sends `tart_image` | YES | `a_tart_start_registers_a_gitlab_runner_and_sends_the_tart_image`. Image rule applies only when provider is `docker` (`provision_on_host`). |
| 4 | Empty runner image: `save_pool_fleet` -> `InvalidRequest("GitLab pool has no runner image")`, saves nothing; manager-path Docker start refused before `create_runner` | YES (refusal form) | `a_gitlab_pool_without_a_docker_image_cannot_save_a_docker_platform` (core integration), `a_docker_start_without_a_runner_image_is_refused_before_registering`. One substitution point: `targets::docker_runner_image`, called by `pool_docker_image` and `provision_on_host`. `GITLAB_RUNNER_IMAGE` is absent from the local `origin/master` ref (651ea09) and from local master (50ed9b4), so the refusal form is the right one. See advisory 5 (the brief's B-2(a) `FakeGitLab` assertion). |
| 5 | `requires_docker_socket`: fleet start refused in `fleet::provision` before `submit_intent`/`admit_queued`/`create_runner`; Prepare refusal counted once | YES | `a_fleet_platform_without_the_docker_socket_is_refused_before_registering`, `a_refused_fleet_prepare_counts_once`. F limits the check to Docker platforms, because `save_pool_fleet` stores the flag only for Docker (`targets.rs:628`, verified). Accepted. |
| 6 | Rollback on both paths: GitLab runner deleted (or none created), capacity released, row failed, counted once, next pass starts again; fleet branches on whether a `start_runner` row exists | YES | `a_failed_manager_start_deregisters_the_runner_and_the_next_pass_starts_again`, `a_failed_fleet_registration_abandons_the_placement_and_counts_once`, `a_failed_bootstrap_store_deletes_the_gitlab_runner_once_and_the_next_start_works` (0 `start_runner` rows; exactly one `DeleteRunner`; `gitlab_runner_id` NULL), `a_failed_fleet_start_is_retried_as_a_new_gitlab_runner_on_the_same_connection` (a `start_runner` row exists). The test decides the case from `host_commands`, as R4-B-1 requires. |
| 7 | Cleanup through `delete_runner` on each exit path: `completed`, `IdleExpired` (both paths), drain; a failed delete does not fail the pass | YES | `an_exited_gitlab_runner_is_deregistered_replaced_and_proves_the_circuit` (`completed` and `idle_expired`), `a_drained_gitlab_runner_is_deregistered_from_gitlab_not_github`, `a_failed_gitlab_delete_does_not_fail_the_pass_and_the_sweep_retries_it`, `an_exited_fleet_gitlab_runner_is_deregistered_resets_the_count_and_is_replaced` (no diagnostic, and `IdleExpired`), `a_drained_fleet_gitlab_runner_is_deregistered_and_cleaned_up`. |
| 8 | Delete dispatch: GitLab rows never take the GitHub branch | YES | `delete_runner` now has an `else if runner.ci_platform == "gitlab"` arm; the drain and exit tests assert `github_removals` stays empty. |
| 9 | `RegistrationRefused` on both paths incl. fleet Start-time and late Inspect: failed row, readable diagnostic, counted once | YES | `a_refused_registration_counts_once_with_a_readable_diagnostic`, `a_start_time_registration_refusal_counts_once_with_a_readable_diagnostic`, `a_late_inspect_registration_refusal_counts_once_with_a_readable_diagnostic`. Diagnostic names the connection and `registration_refused`, never the token. |
| 10 | Failure circuit: one refusal over several passes = count 1; three distinct refusals open it; an old completed runner does not reset a later count; a later `completed`/`IdleExpired` resets via `clear_provision_failures`; probe pass on an open circuit = one start; GitHub/Bitbucket reset unchanged | YES | `a_refused_registration_...` (3 more passes, count stays 1), `three_refused_gitlab_runners_open_the_circuit`, `three_refused_fleet_gitlab_runners_open_the_circuit`, `an_old_completed_runner_does_not_reset_a_count_raised_after_it` and its fleet twin, `the_fleet_probe_of_an_open_circuit_is_one_start`. Every count and reset sits behind a guarded `UPDATE ... rows_affected()==1`; `clear_provision_failures` is the one extracted reset shared by GitHub, Bitbucket and GitLab. |
| 11 | Orphan sweep: deletes a marked own-instance runner with no live row (including after `delete_pool`); keeps live/`starting`; never touches unmarked or other-instance; errors per connection do not stop others or fail the pass; respects interval | YES | `the_sweep_deletes_only_this_deployments_orphans`, `a_failing_connection_does_not_stop_the_sweep_which_respects_its_interval`. Code: `maybe_sweep` follows `maybe_sync_github`'s cadence (`gitlabRunnerSweepIntervalSeconds` 600, clamp 60..=3600; `lastGitlabRunnerSweepAt`); sweeps every `gitlab_connections` row; `ExternalId::try_from` is the one id conversion; `parse_marker` accepts only a UUID runner id. |
| 12 | Retry copies `gitlab_connection_id`, not `gitlab_runner_id`; the retry registers a new runner | YES | `a_failed_fleet_start_is_retried_as_a_new_gitlab_runner_on_the_same_connection`; `retry.rs` diff is the one column. |
| 13 | D6 pool-target sync, project and group, linked to profiles, no duplicates | YES | `saving_a_gitlab_pool_links_one_target_per_connection_to_its_platforms`, `the_target_list_for_granting_machines_includes_gitlab_connections`. `sync_gitlab_targets` is called beside `sync_github_targets` at both call sites. |
| 14 | D11 admission | YES | `gitlab_targets_are_valid_only_for_pools_linked_to_their_connection`, `link_pool_target_links_a_gitlab_target_only_for_a_pool_of_its_connection` (`gridops-core` lib). |
| 15 | `FakeGitLab::list_runners` / `with_runner` / `FakeMethod::ListRunners` | YES | `fake_tests::listed_runners_are_the_scopes_own_on_that_instance_seeded_ones_included`; existing fake tests green (264). |
| 16 | Placeholder client: each start fails at `create_runner`, counted once, nothing left `starting`, leases released, circuit opens at 3 with a readable reason, `reconcile()` still `Ok`, other pools reconciled | YES | `an_unwired_gitlab_client_fails_each_start_cleanly_until_the_circuit_opens`. Production wiring `gitlab: gitlab_lifecycle::unwired_client()`; `UnwiredGitLab` is a private named type returning `Upstream("GitLab client not available yet")` from every method. |

Issue-text requirements (outcomes): `IdleExpired` and `completed` leave without error and `min_count`/`desired` replaces the runner (asserted: "replaced up to the desired count in the same pass", no `error`-level events). `RegistrationRefused` is a failure with a readable diagnostic. Owns/Don't-touch: the diff against b22c451 touches only the files the brief lists (`gitlab/mod.rs` and `fake.rs` only for D17; `retry.rs` for the one column; no migration, no `gridops-api`, manager, agent, `provisioning.rs`, `domain.rs` or `gitlab_poll.rs`). `start_tests.rs` changed fixtures only (I read the whole diff: `bodies()`, `other_requests()`, `exit()`, `removal_fails`, a `FakeGitLab` on `Setup`).

Deviations from the brief that I examined and accept:
1. `exitReason` is read through a `ListedManagerRunners` / `ListedRunner` shape on `ManagerRunners`, not as a field on `ManagedRunner` (a test literal of `ManagedRunner` would not compile and F may not edit tests). Same wire behaviour; an older manager sends no field. Handoff-A-dup recorded it as info.
2. The manager-path "busy" reset is not implemented. The brief makes it optional and states it is untestable until #140's webhook is on master (brief-gate R2 W-1).
3. Configuration refusals (missing runner image on the manager path; Docker platform without the Docker socket on the fleet path) are counted through `record_provision_failure`. The brief requires only the refusal. Counting stops the pool logging a "reconciliation failed" error every pass forever and gives backoff then `blocked`, which suits an error that will not heal itself. Consistent with the brief's circuit design; recorded so O knows.

## Quality audit

| Area | Result | Notes |
|---|---|---|
| API consistency | PASS | D17 `list_runners(&GitLabCredentials, &Scope) -> Result<Vec<RunnerSummary>, GitLabError>` and `RunnerSummary { id: u64, description: String }` match the published signature exactly; `RunnerSummary` derives `Debug, Clone, PartialEq, Eq`. |
| Error handling | PASS | Registration, delete, listing and credential errors each have a path and a test. `deregister_runner` logs and never fails the pass; `NotFound` counts as deleted on every path including the sweep (round-4 W3). Fleet `create_runner` failure abandons the placement, fails the row and counts once. No `unwrap`/`expect`/`panic!`/`TODO`/`dbg!`/`println!` in `gitlab_lifecycle.rs` (grep). |
| UI/UX match to spec | N/A | No UI. |
| Accessibility | N/A | No UI. |
| Architecture gate | PASS | A round-5 PASS; A-dup PASS. No quality finding conflicts with A. |
| Code organization | PASS | `gitlab_lifecycle.rs` holds only the GitLab counterparts of the Bitbucket helpers, and the shared pipelines gained arms (`provision_on_host`, `start_admitted_runner`, `delete_runner`, `fleet::provision`, `register_and_start`, `follow`, `finish`). `load_pool` shares `load_pools`' SELECT through `pool_query!`; the image rule lives once in core. No parallel pipeline. |
| Security | PASS | All SQL is bound or built from static `concat!`ed constants. The `glrt-` token is in the manager request body or the sealed bootstrap only: `assert_no_token_stored` covers every `runners` column, events, waits, audit and the captured reconciler log. `parse_marker` is strict, and the sweep deletes only runners whose marker names this deployment's instance. Marker and credentials come from the vault, not logs. |
| Performance | PASS | One extra `next_connection` SELECT per fleet `provision` and per manager start (returns `None` for non-GitLab pools); one SELECT per sweep listing entry (`runner_live`), once per 600 s by default. Nothing in a hot loop. |
| Visual regression | N/A | No UI. |
| Naming consistency | PASS | `gitlab_runner_id`, `gitlab_connection_id`, `with_runner` (matches `with_scope`/`with_job`), `gitlabRunnerSweepIntervalSeconds`, `lastGitlabRunnerSweepAt`, `gitlabRunnerToken`, `GITLAB_NO_RUNNER_IMAGE` all follow the brief's wording. |
| Test quality (`testing/test-quality.md` §7) | PASS | See below. |

### Test-quality audit (§7 rubric over the diff's tests)

- **Per test.** I read the manager-path tests, the sweep and the fleet rollback/retry tests in full, and the helpers. Each names one behaviour in its title; each asserts an observable outcome (the fake's recorded calls, `runners` rows, `circuit` tuple, the stand-in manager's request log, capacity allocation rows) rather than an internal call; T's RED handoff shows each failing for a feature assertion against stubs, and T's mutation check shows one failing for the right reason. Expected values come from the published constants and types (`GITLAB_EXIT_REASONS`, `GITLAB_DEFAULT_WAIT_TIMEOUT_SECS`, `FailureCode`, `DiagnosticCode`), as the stub-fidelity rule requires.
- **Tier and duplication.** Manager and fleet paths are separate code (`start_admitted_runner` vs `register_and_start`/`follow`/`finish`), so their twins (refusal, three-refusals, old-completed, drain, exit) prove different branches, not the same one twice. Core tests sit at unit tier (D11, fake) and integration tier (public `targets` API over a migrated DB). I found nothing to delete or merge.
- **Proportionality.** 32 new tests (26 reconciler, 6 core) for 16 done-when items across two paths and a new module of ~840 lines is proportionate.
- **Smells.** None of: assertion-free, tautological, unreachable outcome, mock-shaped, snapshot, coverage padding.
- **Non-blocking observations** (advisory 3 and 5 below): one test bundles several behaviours in a loop; one brief assertion is intentionally absent because it could not fail.

## Scope check
F delivered the brief's scope: D6, D11, D17, the image substitution point, the retry column, the sweep, both paths, the circuit work, the placeholder client. Nothing over-delivered beyond the counted configuration refusals (deviation 3), which the brief's own failure-circuit design supports. Nothing under-delivered except the optional, #140-dependent busy reset.

## Merge readiness (read this before the PR is merged)

The audit above is of the branch at 05131a9 against the base the brief was written for (b22c451). The local `origin/master` ref has moved (651ea09; I did not fetch, this is the ref as it stood in the repository at audit time). The merge-base is still b22c451, and `origin/master` now carries `eca9f0d` (#146, the real `GitLabClient`), `e8b18bf` (#145), `c575130`, `0086aa9` (#138) and `651ea09` (#148). The brief anticipated this ("whichever lands second rebases onto it"), but the rebase is not a no-op:

1. **Textual conflicts.** `git merge-tree --write-tree HEAD origin/master` reports conflicts in `crates/gridops-core/src/fleet/targets.rs`, `crates/gridops-reconciler/Cargo.toml`, `crates/gridops-reconciler/src/main.rs` and `crates/gridops-reconciler/src/start_tests.rs`.
2. **`GitLabApi::list_runners` has three more implementors on master.** `GitLabClient` (`crates/gridops-core/src/gitlab/http.rs`), `UnconfiguredGitLab` (`crates/gridops-api/src/state.rs`) and `UnavailableGitLab` (test, `crates/gridops-reconciler/src/gitlab_poll.rs`) will not compile once D17's trait method lands. D17 assumed #146 would ship an inherent `list_runners` that the second-merging PR would wire into the trait; master's `http.rs` has no runner listing at all. So `GitLabClient::list_runners` is real new production code (project: `GET /projects/:id/runners?type=project_type`; group: `GET /groups/:id/runners?type=group_type`; paginated), and no gate in this run has seen it.
3. **Do not keep the placeholder in production after the rebase.** `origin/master`'s `main.rs` already wires `gitlab: Arc::new(GitLabClient::new()?)`. Resolving the `Reconciler.gitlab` conflict by keeping this branch's `gitlab_lifecycle::unwired_client()` would silently regress production to a client that fails every call. Use the real client and delete `UnwiredGitLab` / `unwired_client` (or clippy's `dead_code` fails the gate). Item 16's test still holds because it drives a `FakeGitLab` set to fail.
4. **One credential builder, not two.** #148 added `gitlab_credentials(app, connection_id)` in `main.rs`; this branch has `GitlabConnection::credentials` in `gitlab_lifecycle.rs`. Both unseal the same vault key. After the rebase, reuse one (A-dup ran against b22c451, where #148's helper did not exist).

Recommendation to O: treat the rebase as an integration step with its own A and T pass over the `http.rs` listing and the `main.rs` wiring (paginated HTTP code and production wiring are outside what this audit covered), or at minimum have CI and a reviewer cover those hunks. This does not change the verdict on the audited branch, because the brief assigns the rebase to the second lander and the 05131a9 code is complete and correct against the brief as written.

## Verdict

PASS — all 16 consolidated acceptance items are met and evidenced by tests that I re-ran green; the diff is within scope and the Don't-touch list; the architecture and anti-duplication gates passed; tests meet the §7 rubric; fmt and CI's pinned clippy are clean. Ready for O to commit. It is not mergeable to current master without the rebase described under "Merge readiness".

## Advisory notes

1. **Merge readiness** (above): the rebase onto the moved `origin/master` needs the four conflict resolutions, `list_runners` on three more `GitLabApi` implementors (one of them real HTTP code), the real client kept in `main.rs`, and one credential builder.
2. **PR description (Done-when item 4, R3-W-1).** Record the observed master state and which form of item 4 was written: local master 50ed9b4, local `origin/master` ref 651ea09 at audit time (F saw c575130), `GITLAB_RUNNER_IMAGE` absent from both, so the refusal form ("GitLab pool has no runner image") is asserted. Whichever of #139/#147 merges second switches the empty case to `GITLAB_RUNNER_IMAGE` in one line (`targets::docker_runner_image`).
3. **Test (non-blocking).** `an_exited_gitlab_runner_is_deregistered_replaced_and_proves_the_circuit` (`gitlab_lifecycle_tests.rs:628`) loops over `completed` and `idle_expired` and bundles the probe, deregistration, GitHub-silence, reset, refill and no-error assertions in one body. The loop is justified (two mapping arms the brief names), but a failure will not isolate its cause. Splitting the probe assertion from the exit assertions would improve diagnosis; not worth a rework cycle.
4. **Test (non-blocking).** Nothing tests the production `UnwiredGitLab` type directly; item 16 is pinned through a failing `FakeGitLab`, as the brief says. This becomes moot when the real client replaces the placeholder (Merge readiness 3).
5. **A brief assertion intentionally absent.** Brief-gate round-2 "B-2 (a) addition" asks the `save_pool_fleet` refusal test to also assert `FakeGitLab::calls()` has no `CreateRunner`. `save_pool_fleet(&SqlitePool, ...)` has no GitLab client in its signature, and the test (`crates/gridops-core/tests/gitlab_pool_fleet.rs`) injects none, so that assertion could not fail (a §6 smell). I treat the requirement as met by construction. T-red did not record this omission; O should know.
6. **Behaviours the brief left open, for O's awareness.** (a) The sweep treats a `failed` row as live (the same pass's failed loop deletes it, and the sweep then catches its GitLab runner). (b) A fleet placement that stops with no Inspect result is `Exit::Unknown`: no count and no reset, because nothing proves registration. (c) `ensure_target_access` still reads `bitbucket_connection_id` for a GitLab target; it passes for `Principal::Reconciler`, and user-scoped GitLab visibility is deferred per D11 (T noted this too).
7. **Not covered until #140 lands:** the manager-path busy reset. When #140's webhook is on master, add the busy-reset test that brief-gate R2 W-1 requires.
