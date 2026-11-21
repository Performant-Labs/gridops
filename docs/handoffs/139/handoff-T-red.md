# Handoff-T-red: Phase 4 - #139 GitLab reconciler runner lifecycle

**Date:** 2026-11-20
**Branch:** claude/gitlab-139-lifecycle-53c8ce (base b22c451)
**Brief / wireframe reviewed:** `docs/handoffs/139-brief.md`, the "Done when (consolidated)" list (items 1-16) and the round-4 amendments. No wireframe: there is no UI surface.

## A precondition

Confirmed. A returned PASS on the round-4-amended plan (round 5, `handoff-A.md`, decisions.md entry 2026-11-20T16:30:28-08:00), with two warnings. The tests follow warning 1's reading: the probe limit is one GitLab start per pass.

## Tests authored

All test code avoids `unwrap`, `expect` and `panic!`, as the workspace lints require. Expected values come from the published contract, never typed by hand: `GITLAB_EXIT_REASONS`, `GITLAB_DEFAULT_WAIT_TIMEOUT_SECS`, `FailureCode`, `DiagnosticCode`, and the fake's token format.

### gridops-core

| Test | File | Pins | Tier, and why |
|---|---|---|---|
| `listed_runners_are_the_scopes_own_on_that_instance_seeded_ones_included` | `crates/gridops-core/src/gitlab/fake_tests.rs` | Item 15 (D17). `list_runners` returns a scope's own runners per base URL, including `with_runner`-seeded ones. A seeded runner is deletable. It honours a one-shot `fail_next(ListRunners)` and is recorded in `calls()`. | Unit, beside the existing fake tests: the fake is test support used by everything else. |
| `gitlab_targets_are_valid_only_for_pools_linked_to_their_connection` | `crates/gridops-core/src/fleet/admission_tests.rs` | Item 14 (D11). `ensure_target_identity_for_pool` accepts a project or group target whose connection is linked to the pool. It refuses an unlinked one, and one whose membership was removed. | Unit: a `pub(crate)` function, reached only from inside the crate. |
| `link_pool_target_links_a_gitlab_target_only_for_a_pool_of_its_connection` | same | Item 14 (D11). `link_pool_target` links a GitLab target for its own pool only, and is idempotent. | Unit, same reason. |
| `saving_a_gitlab_pool_links_one_target_per_connection_to_its_platforms` | `crates/gridops-core/tests/gitlab_pool_fleet.rs` (new) | Item 13 (D6) at `save_pool_fleet`. One `gitlab_project` target and one `gitlab_group` target, linked to the pool's platform and returned by `pool_targets`. No duplicates on a re-save. A GitHub pool gets none. | Integration: public `targets` API over a real migrated database. |
| `the_target_list_for_granting_machines_includes_gitlab_connections` | same | Item 13 (D6) at `list_targets`, the list an administrator grants machines from. | Integration, same. |
| `a_gitlab_pool_without_a_docker_image_cannot_save_a_docker_platform` | same | Item 4 (`save_pool_fleet` half), refusal form. Returns `FleetError::InvalidRequest("GitLab pool has no runner image")` and saves nothing. A GitHub pool keeps the stock-image fallback. A GitLab pool's own image is used as given. | Integration, same. |

### gridops-reconciler: `crates/gridops-reconciler/src/gitlab_lifecycle_tests.rs` (new; mounted in `main.rs` as `#[cfg(test)] mod gitlab_lifecycle_tests;`)

Two environments, both reconciler-level integration tests. That is the cheapest tier that sees a pass end to end.

- **Manager path:** the stand-in runner manager of `start_tests.rs`, plus `FakeGitLab` injected as `app.gitlab`.
- **Fleet path:** an approved, active Linux Docker machine. The test plays its agent and records command results through the real `RegistryService::record_command_result`, as `host-agent` does.

**Manager path (built-in host, Docker and Tart)**

| Test | Pins |
|---|---|
| `a_docker_start_registers_a_gitlab_runner_and_hands_the_manager_its_token` | Item 1, Docker. Shape of `create_runner`: scope, tags, `run_untagged`, marker `gridops-runner:gridops.example.test:8443:<runners.id>`. The row columns. The D10 request: `platform`, `mode`, `runtime`, `gitlab.url`, `waitTimeoutSeconds`, `gitlabRunnerToken`, and the four absent keys. Token secrecy: no `glrt-` in any `runners` column, event, wait reason, audit event, or the reconciler's own log. |
| `a_tart_start_registers_a_gitlab_runner_and_sends_the_tart_image` | Items 1 (Tart) and 3. A Tart-only pool with `docker_image=''` registers and sends `tart_image`. |
| `a_docker_start_without_a_runner_image_is_refused_before_registering` | Item 4 (manager half), refusal form. No `CreateRunner` call, no POST, no row `starting`, and a readable reason naming the connection and "no runner image". |
| `a_failed_manager_start_deregisters_the_runner_and_the_next_pass_starts_again` | Item 6 (manager). The GitLab runner is deleted, the lease is released (`DELETE /v1/admissions/…`), the row is failed and counted once. The next pass registers runner 2. |
| `an_exited_gitlab_runner_is_deregistered_replaced_and_proves_the_circuit` | Items 7, 8 and 10, and round-4 W2. For each of `completed` and `idle_expired`: the probe pass on an open circuit makes exactly one start, and a POST success resets nothing. The exit deletes the row and makes one `DeleteRunner` call, with no GitHub removal. The circuit is reset (0, closed, no `retry_at`) and the pool is not `blocked`. The pool is refilled to desired in that pass, with no error events. |
| `a_drained_gitlab_runner_is_deregistered_from_gitlab_not_github` | Items 7 (drain) and 8. |
| `a_failed_gitlab_delete_does_not_fail_the_pass_and_the_sweep_retries_it` | Items 7 and 11. After a `DeleteRunner` failure the row still leaves service, there is no "Pool reconciliation failed", and the replacement starts. Once `lastGitlabRunnerSweepAt` is backdated, the sweep removes the leftover. |
| `a_refused_registration_counts_once_with_a_readable_diagnostic` | Items 9 and 10, and the round-4 block. The refusal is counted once over three passes. The counting pass ends in `backoff` and starts nothing. Readable `registration_refused`. Deregistered. |
| `three_refused_gitlab_runners_open_the_circuit` | Item 10. The circuit is open and `blocked`, with one "Provisioning circuit opened" event. No start is made while it is open. |
| `an_old_completed_runner_does_not_reset_a_count_raised_after_it` | Item 10, round-3 block 1. The first exit resets the count (1 to 0). A completed runner whose removal keeps failing does not reset a later count of 2. |
| `an_unwired_gitlab_client_fails_each_start_cleanly_until_the_circuit_opens` | Item 16, brief-gate W-3 and round-4 W4. Each `create_runner` failure (`Upstream("GitLab client not available yet")`) counts once and leaves no row `starting` and no GitLab runner. Every lease is released. `reconcile()` returns `Ok`, and `pool-linux` is still started. The circuit opens at 3 with a readable reason and holds the next attempt back. |

**Orphan sweep (D17)**

| Test | Pins |
|---|---|
| `the_sweep_deletes_only_this_deployments_orphans` | Item 11. Deleted: the runners of a deleted row, of a missing row, and of a connection whose only pool was removed by `targets::delete_pool`. Kept: `starting` and `online` rows, unmarked runners, other-instance markers, malformed markers. Sets `lastGitlabRunnerSweepAt`. |
| `a_failing_connection_does_not_stop_the_sweep_which_respects_its_interval` | Item 11. A listing failure on one connection and a delete failure on another are skipped, the healthy connection is still swept, and `reconcile()` returns `Ok` with the pools reconciled. Nothing is listed within the interval. After 601 s everything is swept. |

**Fleet path (`Fleet` harness)**

| Test | Pins |
|---|---|
| `a_fleet_start_registers_the_runner_and_seals_the_gitlab_material` | Item 2. With two connections, the earliest is selected. `create_runner` shape and row columns. The intent's target is the connection's own target. The sealed material decodes as `GitlabRunnerMaterial::GitlabRunner { token, url, wait_timeout_seconds: 600 }`. One `start_runner`. Token secrecy, logs included. |
| `a_fleet_platform_without_the_docker_socket_is_refused_before_registering` | Item 5(a). No `CreateRunner`, no intent, no host command, no row `starting`, and a readable "docker socket". |
| `a_refused_fleet_prepare_counts_once` | Item 5(b). |
| `a_failed_fleet_registration_abandons_the_placement_and_counts_once` | Items 6 and 16 (fleet `create_runner` fails). No `start_runner` row. The placement is `failed` with no unreleased allocation. Counted once. |
| `a_failed_bootstrap_store_deletes_the_gitlab_runner_once_and_the_next_start_works` | Item 6 and round-4 W3. The bootstrap store is made to fail by a test trigger on `fleet_bootstrap_material`. Exactly one `DeleteRunner`, `gitlab_runner_id` is NULL, no `start_runner`, capacity released, counted once. The next start registers runner 2. |
| `a_failed_fleet_start_is_retried_as_a_new_gitlab_runner_on_the_same_connection` | Items 6 (a `start_runner` row exists) and 12. Counted once, deregistered, no capacity held. The retry row copies `gitlab_connection_id`, not `gitlab_runner_id`, and registers runner 2. |
| `an_exited_fleet_gitlab_runner_is_deregistered_resets_the_count_and_is_replaced` | Items 7 and 10, for Inspect `ResourceUnavailable` both without a diagnostic and with `IdleExpired`: out of service, deregistered, one cleanup, count 2 reset to 0, replaced, no error events. |
| `a_drained_fleet_gitlab_runner_is_deregistered_and_cleaned_up` | Items 7 (drain) and 8. |
| `a_start_time_registration_refusal_counts_once_with_a_readable_diagnostic` | Item 9 (Start-time). Uses `Failed { Configuration, RegistrationRefused }`. |
| `a_late_inspect_registration_refusal_counts_once_with_a_readable_diagnostic` | Item 9 (late Inspect). |
| `three_refused_fleet_gitlab_runners_open_the_circuit` | Item 10 (fleet). |
| `an_old_completed_fleet_placement_does_not_reset_a_count_raised_after_it` | Item 10, round-3 block 1 (fleet). Reset once (1 to 0). `follow` and `finish` revisiting the placement do not reset a later count of 2. |
| `the_fleet_probe_of_an_open_circuit_is_one_start` | Round-4 W2 (fleet). Desired 3 and an open circuit give exactly one start, and it registers GitLab runner 1. |

**Readable-diagnostic contract the tests use** (brief-gate W-2): at least one recorded text names the connection (`Acme API`) and the outcome, case aside. The texts checked are `runners.failure_reason`, `runner_events.message` and `runner_pool_waits.reason`. The outcomes are: "no runner image", "docker socket", "registration_refused", and the placeholder error text. No recorded text contains `glrt-`.

### Test-only fixture edits (not production code)

- **`start_tests.rs`:**
  - `ManagerLog` now records POST bodies (`bodies()`) and has a `removal_fails` switch.
  - It records any other request (`other_requests()`, which is how lease releases are seen) and answers it with 204.
  - It has `exit(runner_id, exitReason)`.
  - `Setup` owns a `FakeGitLab` and builds `Reconciler { gitlab: Arc::new(gitlab.clone()), … }`.
- **`main.rs` (test module only), `builtin_host.rs`:** the `Runner` literals gain `gitlab_connection_id: None` and `gitlab_runner_id: None`, and `reconciler_with_github` gets `gitlab`.
- **`gridops-reconciler/Cargo.toml`:** the dev-dependency `gridops-core` with `features = ["test-support"]`, as the brief specifies.

## RED confirmation

### 1. gridops-reconciler: compile RED, missing contract types only

`cargo test -p gridops-reconciler --no-run`:

```
error[E0560]: struct `Reconciler` has no field named `gitlab`                --> start_tests.rs:506, main.rs:4836
error[E0560]: struct `Runner` has no field named `gitlab_connection_id`      --> builtin_host.rs:72, main.rs:3846
error[E0560]: struct `Runner` has no field named `gitlab_runner_id`          --> builtin_host.rs:73, main.rs:3847
error[E0599]: no method named `with_runner` found for struct `FakeGitLab`    --> gitlab_lifecycle_tests.rs:1004
error[E0599]: no variant ... named `ListRunners` found for enum `FakeMethod` --> gitlab_lifecycle_tests.rs:1134, 1160, 1163
error: could not compile `gridops-reconciler` (bin "gridops-reconciler" test) due to 10 previous errors
```

Every error names a type or member the brief tells F to add: the `Reconciler.gitlab` field, the two `Runner` fields, D17's `with_runner`, and `FakeMethod::ListRunners`. There is no other error.

**Behavioral validation (throwaway, never staged).** To prove the code behind the compile errors is sound and fails on behavior, I temporarily added the bare missing types to production code, then restored those files byte-for-byte with `git checkout` and the backups:

- the field and the two `Runner` fields, each `#[sqlx(default)]`;
- a no-op `list_runners`;
- `RunnerSummary`;
- `ListRunners`;
- a seeding `with_runner`.

With only those stubs:

- **Clippy:** `cargo clippy -p gridops-reconciler --all-targets` reported no finding in the new or changed test code. The one warning is pre-existing, at `builtin_host.rs:119`.
- **Existing reconciler tests:** 68 passed, 0 failed (`cargo test -p gridops-reconciler -- --skip gitlab_lifecycle_tests`). The fixture changes break nothing.
- **Manager-path and sweep tests:** each fails on a feature assertion. Examples:
  - `a_docker_start_registers…`: `left: []`, `right: [FakeRunner { id: 1, scope: Project 42, tags: ["docker","linux-large"], run_untagged: true, description: "gridops-runner:gridops.example.test:8443:<uuid>" }]`
  - `a_docker_start_without_a_runner_image…`: today's code POSTs a GitHub `jitConfig` start with `image: ""`.
  - `a_failed_manager_start…`: `left: 0`, `right: 1` (no `CreateRunner`).
  - `an_exited_gitlab_runner…`: `left: 2`, `right: 1` (today's code starts `desired` while the circuit is open, so there is no single probe).
  - `a_refused_registration…`: `(0,false)` vs `(1,false)`.
  - `three_refused…`: `(0,false,"active")` vs `(3,true,"blocked")`.
  - `an_unwired_gitlab_client…`: `(0,false)` vs `(1,false)`.
  - `the_sweep…`: `[11,12,13,14,15,16,17]` vs `[11,12,15,16,17]`.
  - `a_failing_connection…`: `left: [3]`, `right: []`.
  - `an_old_completed_runner…`: `left: 1`, `right: 0` (no reset at all yet).
- **Fleet tests:** each fails in `Fleet::newest` with `no placed runner in pool-gl; the pool's events: ["Pool reconciliation failed: fleet operation is not authorized"]`. The cause is today's `fleet::provision`: it takes the first `pool_targets` entry, which is the installation's `github_organization` target, for a `scope='gitlab'` pool. `submit_intent` then refuses it. That is D6/D1, the target choice F adds, so it is the right reason.
- **Fleet harness, second stub layer:** to prove the harness itself works, I added two more throwaway stubs:
  - `ensure_target_identity_for_pool` accepts GitLab kinds;
  - `fleet::provision` uses the profile's linked target for `scope='gitlab'`.

  With those, the pool admits and places, and every fleet test fails on a feature assertion. Examples:
  - `a_fleet_start_registers…`: `left: []` (no `CreateRunner`).
  - `a_failed_fleet_registration…`: `start_runner` count `1` vs `0`.
  - `a_failed_bootstrap_store…`: `DeleteRunner` count `0` vs `1`.
  - `a_failed_fleet_start…`: `gitlab_runner_id None` vs `Some(1)`.
  - `an_exited_fleet…`: `(2,false)` vs `(0,false)`.
  - `a_refused_fleet_prepare…`, `a_start_time…` and `a_late_inspect…`: `(0,false)` vs `(1,false)`.
  - `three_refused_fleet…`: `0` vs `1`.
  - `a_fleet_platform_without_the_docker_socket…`: intents `1` vs `0`.
  - `an_old_completed_fleet…`: `1` vs `0`.
  - `the_fleet_probe…`: `3` vs `1`.

  All 26 tests fail this way.

**Two tests passed before code and were strengthened.** `an_old_completed_runner_does_not_reset…` and its fleet twin passed vacuously, since no reset existed yet. Each now first asserts the one legitimate reset (count 1 to 0). `the_fleet_probe…` passed only because the pass errored, leaving one failed row. It now also asserts that the probe registers GitLab runner 1. All three were re-run under the stubs and fail as shown above.

### 2. gridops-core lib: compile RED, D17 only

`cargo test -p gridops-core --lib --no-run`:

```
error[E0432]: unresolved import `crate::gitlab::RunnerSummary`                  --> fake_tests.rs:18
error[E0599]: no method named `with_runner` found for struct `FakeGitLab`       --> fake_tests.rs:408
error[E0599]: no method named `list_runners` found for struct `Arc<dyn GitLabApi>` --> fake_tests.rs:419, 436, 443, 451, 459, 463
error[E0599]: no variant ... named `ListRunners` found for enum `FakeMethod`     --> fake_tests.rs:455, 470-497
error: could not compile `gridops-core` (lib test) due to 19 previous errors
```

The two `?` and `sort_by_key on !` follow-on errors come from the same missing method.

### 3. gridops-core lib: D11 tests

These were run with HEAD's `fake_tests.rs` swapped in temporarily, then mine restored. Command: `cargo test -p gridops-core --lib -- gitlab_targets_are_valid_only link_pool_target_links_a_gitlab`.

```
test ...::link_pool_target_links_a_gitlab_target_only_for_a_pool_of_its_connection ... FAILED
  panicked at admission_tests.rs:3598:9:  left: 0  right: 1   (the pool's own connection's target is not linked)
test ...::gitlab_targets_are_valid_only_for_pools_linked_to_their_connection ... FAILED
  panicked at admission_tests.rs:3550:9:  target 34b2f81d-... of the pool's own connection: Err(Forbidden)
test result: FAILED. 0 passed; 2 failed
```

These are the right reasons: `_ => false` in `ensure_target_identity_for_pool`, and `link_pool_target` handles GitHub repositories only.

### 4. gridops-core integration: D6 and the image rule

`cargo test -p gridops-core --test gitlab_pool_fleet`:

```
test a_gitlab_pool_without_a_docker_image_cannot_save_a_docker_platform ... FAILED
  panicked at gitlab_pool_fleet.rs:234:5: Ok(FleetSettings { ... image: "ghcr.io/actions/actions-runner:latest" ... })
test saving_a_gitlab_pool_links_one_target_per_connection_to_its_platforms ... FAILED
  Error: the project connection's target exists
test the_target_list_for_granting_machines_includes_gitlab_connections ... FAILED
  Error: conn-project has a fleet target after list_targets
test result: FAILED. 0 passed; 3 failed
```

These are the right reasons: there is no GitLab target sync, and the GitHub stock image is substituted.

### Formatting and lint

`cargo fmt --all -- --check` is clean. Clippy on gridops-core (lib and the new integration test, HEAD `fake_tests.rs` swapped in) reports only the two pre-existing `autoscaling.rs` warnings.

## Ready for F

RED is valid. All 32 new tests fail for the right reason:

- 26 lifecycle tests;
- 2 D11 tests;
- 3 D6/image tests;
- 1 `list_runners` fake test, at compile;
- the stub-validated behavior failures described above.

F may implement against these tests.

Notes for F:

- **Names the tests assume:**
  - `gridops_core::gitlab::RunnerSummary { id: u64, description: String }`;
  - `GitLabApi::list_runners(&GitLabCredentials, &Scope)`;
  - `FakeMethod::ListRunners`;
  - `FakeGitLab::with_runner(&Url, FakeRunner)`;
  - `Reconciler.gitlab: Arc<dyn GitLabApi>`;
  - `Runner.gitlab_connection_id: Option<String>` and `Runner.gitlab_runner_id: Option<i64>`.

  These follow the brief (D17, R2-W3, warning 9). If F needs another name, the fix is in the tests: hand it back to T.
- **Placeholder test:** item 16 is tested through `FakeGitLab::fail_next(CreateRunner, Upstream("GitLab client not available yet"))`, the same error the production placeholder returns. The placeholder impl itself only needs to compile, and to return that error from every method, `list_runners` included.
- **Probe:** the fleet and manager probe assertions read A's warning 1 as one GitLab start per pass. They check the probe pass only.
- **`ensure_target_access` (`service.rs`, advisory):** for a non-GitHub target it reads `bitbucket_connection_id`, which is NULL for a GitLab target. That passes for `Principal::Reconciler`. A GitLab arm would make the intent clear. User-scoped GitLab visibility is deferred per D11.
- **Fleet admission prerequisites:** the fleet tests set `min_count=1` (the pool's `NoDemand` rule with `placement=machines` and no queued jobs). The machine's capability list declares `gitlab_project` and `gitlab_group` as ephemeral.
