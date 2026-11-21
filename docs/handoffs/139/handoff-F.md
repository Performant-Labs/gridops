# Handoff-F: Phase 5 - GitLab reconciler runner lifecycle (register, roll back, clean up)

**Date:** 2026-11-20
**Branch:** claude/gitlab-139-lifecycle-53c8ce (base b22c451; the T-red commit is 2aa6002)
**Issue:** #139

## What was done

**gridops-core**
- `crates/gridops-core/src/gitlab/mod.rs`: adds D17's `RunnerSummary { id: u64, description: String }` (`Debug, Clone, PartialEq, Eq`) next to `CreatedRunner`, and `GitLabApi::list_runners(&GitLabCredentials, &Scope)`.
- `crates/gridops-core/src/gitlab/fake.rs`: adds `FakeMethod::ListRunners` and `FakeGitLab::with_runner(&Url, FakeRunner)`.
  - `with_runner` seeds a runner without moving the id counter.
  - `list_runners` returns the scope's runners on that base URL, matched by scope kind and id. It honours `fail_next` and is recorded in `calls()`.
- `crates/gridops-core/src/fleet/service.rs`: the GitLab arm of `ensure_target_identity_for_pool` (D11). A `gitlab_project` or `gitlab_group` target is valid when both hold:
  - the pool has a `runner_pool_gitlab_connections` row for the target's connection;
  - the target is that connection's own scope (kind and `scope_id`).
- `crates/gridops-core/src/fleet/targets.rs`:
  - D6: `sync_gitlab_targets` and `gitlab_connection_target`. They are called beside `sync_github_targets` in `list_targets` and `save_pool_fleet`.
  - `pool_targets`: for a GitLab pool (one with GitLab memberships, or `scope='gitlab'`), returns its connections' targets in membership order.
  - `link_pool_target`: a GitLab arm (D11).
  - Round-2 block 2: the single image substitution point, `docker_runner_image(pool_image, gitlab)` plus `GITLAB_NO_RUNNER_IMAGE` and `is_gitlab_pool`. `pool_docker_image` now calls it.
- `crates/gridops-core/src/fleet/placement.rs`: `reported_diagnostic(pool, placement, kind)`, the newest applied result's `DiagnosticCode`, next to `reported_gone`. `reported_gone` is unchanged.
- `crates/gridops-core/src/fleet/retry.rs`: the `INSERT…SELECT` copies `gitlab_connection_id`, and never `gitlab_runner_id` (round-2 block 4).

**gridops-reconciler**
- `crates/gridops-reconciler/src/gitlab_lifecycle.rs` (new): the GitLab helpers.
  - Connection selection (`next_connection`) and credentials from the vault.
  - The `gridops-runner:<instance>:<uuid>` marker and its parser.
  - `register`, `deregister` (`NotFound` is done) and `deregister_runner` (logs, never fails).
  - The `Exit` mapping, from `exitReason` or the Inspect diagnostic.
  - The once-per-runner count or reset (`left_service`), and the manager exit hook (`runner_exited`).
  - The fleet arms: `fleet_target` (with the Docker-socket refusal), `fleet_material`, `fleet_rollback`, `fleet_runner_exited` and `fleet_start_failed`.
  - The orphan sweep (`maybe_sweep`).
  - The placeholder client `UnwiredGitLab` / `unwired_client()`.
- `crates/gridops-reconciler/src/main.rs`:
  - The `Reconciler.gitlab` field, wired to `unwired_client()`.
  - `Runner.gitlab_connection_id` and `gitlab_runner_id`, also added to the `runners()` SELECT.
  - `ManagerRunners` reads each listed runner's `exitReason` through a `ListedManagerRunners` / `ListedRunner` shape. `ManagerView::exit_reason` is added.
  - `load_pool`, which shares `load_pools`' one SELECT through a `pool_query!` macro.
  - `clear_provision_failures`, extracted next to `record_provision_failure`.
  - `reconcile_pool`:
    - the GitLab exit hook in the status loop;
    - a reload of the pool after that loop when a count or reset happened;
    - the host-off event compares the pass's entry state;
    - the one-start probe (round-4 W2).
  - `provision_on_host`: the GitLab connection, `CiPlatform::Gitlab`, the Docker image rule, and the `gitlab_connection_id` insert.
  - `start_admitted_runner`:
    - a GitLab arm: `create_runner`, then the `gitlab` request fields and `gitlabRunnerToken`;
    - `gitlab_runner_id` is set by the UPDATE after the POST;
    - rollback deletes the GitLab runner;
    - a GitLab start does not reset the circuit.
  - The sweep's call site in `reconcile()`.
  - `delete_runner` dispatches GitLab rows to the GitLab arm.
- `crates/gridops-reconciler/src/fleet.rs`:
  - `provision`: a GitLab pool takes its target from the selected connection and inserts `ci_platform='gitlab'` with `gitlab_connection_id`.
  - `register_and_start`: a GitLab arm. The shared tail (`store_bootstrap` + `issue_command`) is extracted as `start_with_material`.
  - `follow` and `finish`: capture the guarded update's `rows_affected()` and call the GitLab outcome helpers once per runner.

## Design decisions

- **Exactly once per runner, everywhere.**
  - Every GitLab count or reset sits behind a guarded `UPDATE … WHERE status NOT IN ('failed','deleted')` with `rows_affected() == 1`, as round-3 block 1 requires. This covers the manager status loop, `follow`, `finish` and the fleet rollback.
  - One exception: `retry_failed_start` retires the old row itself, once per placement, by its unique retry index (evidence.md). So `finish` treats `retried` in that call as the leave-service moment. Without that, a failed start that is retried was never counted (`a_failed_fleet_start_is_retried…` caught this).
- **Which fleet failures count.** Every failed GitLab placement counts once, whatever the failure: a Prepare refusal, a Start failure, or `RegistrationRefused`. So does every fleet `create_runner` failure (round-3 warning) and a failed bootstrap store. A `RegistrationRefused` also gets the readable diagnostic and an error event.
- **Fleet resets are exit-only (round 3).** An Inspect `ResourceUnavailable` with no diagnostic (completed) or with `IdleExpired` resets. A `placement.state == "stopped"` with no Inspect result is `Exit::Unknown`: no count and no reset, because nothing proves the registration.
- **Configuration refusals count as start failures.** A GitLab Docker start with no runner image (manager path) and a GitLab Docker platform without `requires_docker_socket` (fleet path) are refused before anything is registered or reserved. Each is counted through `record_provision_failure` with the pass's pool, and returned as the pool's `Err`, whose text is readable. The brief did not say whether to count them. Not counting would log a "Pool reconciliation failed" error every 30 s forever. Counting gives the existing backoff, then `blocked` with the "Provisioning circuit opened" event, which suits a configuration error that will not heal by itself.
- **The Docker-socket check applies to Docker platforms only.** `save_pool_fleet` stores `requires_docker_socket` only for Docker (`platform.requires_docker_socket && runtime_kind == Docker`), so a Tart or native platform could never pass the check. The brief's host-agent fact is about #147's Linux Docker agent.
- **`exitReason` without touching `ManagedRunner`.** The brief asks for `#[serde(default)]` on `ManagedRunner`. But `main.rs`'s test module builds `ManagedRunner { id, state, labels }` as a literal, and F does not edit tests. So `ManagerRunners` deserializes `from` a `ListedManagerRunners` whose runners are `{ #[serde(flatten)] ManagedRunner, exitReason }`, and keeps an `exit_reasons` map by container id. It has the same wire behaviour; an older manager sends no `exitReason`.
- **Connection choice.** The connection with the fewest live GitLab runners of the pool, then `membership.created_at, connection.id`. There is no one-runner cap (ephemeral runners).
- **A GitLab pool's `pool_targets`.** A pool with GitLab memberships (or `scope='gitlab'`) returns only its connections' targets, never the installation's GitHub organization target. That organization target was why every fleet test failed at RED ("fleet operation is not authorized").
- **Marker.** `gridops-runner:<host[:port]>:<runners.id>`, where the port appears only when it is not the scheme's default (`Url::port()`). The parser uses `rsplit_once(':')`, so the instance may carry a port. It accepts only a UUID runner id.
- **The sweep's notion of live.** A row is live when it exists, has `deleted_at IS NULL` and has `status != 'deleted'`. That includes `starting`, and also `failed`: a failed row is deleted by the same pass's failed loop, and the sweep then catches its runner. Each connection's listing or credential error and each delete error is logged and skipped.
- **Shared-tail extraction in `fleet.rs`.** `store_bootstrap` + `issue_command` moved verbatim into `start_with_material`, so the GitLab arm can roll back on its error without duplicating the start command. GitHub behaviour is unchanged.

## Reuse / extend-vs-new

The brief's seam is followed: the shared functions gain GitLab arms, and `gitlab_lifecycle.rs` holds only the GitLab counterparts.
- `next_connection` is the counterpart of `next_bitbucket_connection`.
- `register` / `deregister` / `deregister_runner` are the counterparts of the Bitbucket start arm and `deregister_bitbucket_runner`.
- `provision_on_host`, `start_admitted_runner`, `delete_runner`, `fleet::provision`, `register_and_start`, `follow` and `finish` are extended, not paralleled.
- `record_provision_failure` and `clear_provision_failures` are shared with GitHub and Bitbucket. The reset is the one extracted function, with no second copy.
- The image rule lives once, in core `targets::docker_runner_image`.
- `load_pool` shares `load_pools`' query text.
- The placeholder is a private named type in `gitlab_lifecycle.rs` (warning 8).

## Architecture notes for A

- **Layers.** The core contract gains D17 (a trait method and a type) and `placement::reported_diagnostic`. Core `targets` gains `docker_runner_image`, `GITLAB_NO_RUNNER_IMAGE`, `is_gitlab_pool`, `sync_gitlab_targets` and `gitlab_connection_target`, all public. The reconciler gains one module.
- **New dependency direction.** `fleet.rs` → `gitlab_lifecycle` → `super::{load_pool, record_provision_failure, clear_provision_failures, …}`. No core dependency on the reconciler.
- **No migration** (D16), no `ExecutionConfiguration` change (D1), and the API is not touched.
- **Following A's round-5 warnings.**
  - Warning 1: the probe is per pass.
  - Warning 2:
    - `load_pool` is used only outside the start loop (the status loop and `fleet::tick`); the start-loop sites keep the pass's pool;
    - after the status loop, `None` ends the pool's pass with `Ok(())`;
    - the host-off event compares `entry_state`.

## Deviations from spec / wireframe

- **`ManagedRunner` itself does not get the field.** The `exitReason` read lives on `ManagerRunners` through a listing shape, because a test literal of `ManagedRunner` would not compile otherwise. See Design decisions.
- **Configuration refusals are counted.** The image and Docker-socket refusals are counted (see Design decisions). The brief requires only the refusal.
- **No busy reset on the manager path.** The optional "busy" reset (round 3: "may also reset") is not implemented. `busy` is only ever set for a GitLab runner by #140's webhook, which is not on master, so the code would be untestable today (brief-gate R2 W-1). Resets are exit-only on both paths.
- Otherwise none. There is no UI.

## Tier 1 self-check (incl. tests now GREEN)

```
$ cargo fmt --all -- --check
(clean)

$ cargo test -p gridops-reconciler gitlab_lifecycle_tests
test result: ok. 26 passed; 0 failed; 0 ignored; 0 measured; 68 filtered out

$ cargo test -p gridops-core --lib gitlab
test result: ok (fake + D11 admission tests)
$ cargo test -p gridops-core --lib
test result: ok. 264 passed; 0 failed

$ cargo test -p gridops-core --test gitlab_pool_fleet
test a_gitlab_pool_without_a_docker_image_cannot_save_a_docker_platform ... ok
test saving_a_gitlab_pool_links_one_target_per_connection_to_its_platforms ... ok
test the_target_list_for_granting_machines_includes_gitlab_connections ... ok
test result: ok. 3 passed; 0 failed

$ cargo test --workspace --no-fail-fast
(see "Full workspace run" below)

$ cargo clippy --workspace --all-targets --keep-going -- -D warnings
17 findings, every one in a file this change does not modify (clippy 0.1.99's
`assert_is_empty` / `unused_async_trait_impl` lints):
  gridops-core/src/autoscaling.rs:1615,1701; gridops-reconciler/src/builtin_host.rs:119;
  gridops-host-agent/src/helper.rs:178, native.rs:200; gridops-api/src/fleet/boundary.rs:78,
  fleet/enrollment.rs:46, resources/route_security.rs:16; gridops-agent/src/catalog.rs:209,
  changes.rs:206, tools.rs:886; gridops-manager/src/main.rs:228, vm_slots.rs:755;
  gridops-tart-agent/src/main.rs:434,2789,2811,3227.
No finding is in a file F created or changed. The one F finding (single_match_else in
provision_on_host) was fixed.
```

Full workspace run, `cargo test --workspace --no-fail-fast`: 878 passed, 12 failed.
- `gridops-reconciler`: 94 passed (68 existing and 26 new). `start_tests.rs`'s `a_successful_probe_clears_the_blocked_state_in_the_same_pass` passes unmodified across the `clear_provision_failures` extraction (R4-W-3).
- `gridops-core`: 264 lib tests passed, and `gitlab_pool_fleet` 3/3.
- The 12 failures are environmental. Each also fails on the pre-change commit 2aa6002, run in a throwaway worktree that was then removed:
  - 11 in `gridops-manager` (`provision_lock_tests`, `route_timeout_tests`, `tart_outage_tests`), each `Error: path must be shorter than SUN_LEN`. The test Unix-socket paths under this workstation's `TMPDIR` exceed macOS's limit.
  - 1 in `gridops-api`: `release_preflight::the_release_preflight_fails_early_without_a_macos_arm64_machine`. The fixture agent reports the build host's OS (`probe::host_os()` uses `cfg!(target_os)`), so on this Darwin arm64 workstation the "Linux machine" is a Mac.
- Both pass only on a Linux CI host; neither touches code this change edits.

## Evidence appendix

`docs/handoffs/139/evidence.md`

## Tests that look wrong (for T)

None are wrong. One fixture limit, for T's information:
- `main.rs`'s test module builds `ManagedRunner { id, state, labels }` as a literal, which is why the `exitReason` read lives on `ManagerRunners` rather than on `ManagedRunner` (brief: "ManagedRunner / ManagerView (read `exitReason`, `#[serde(default)]`)").
- If T would rather have the field on `ManagedRunner`, the literal needs `exit_reason: None` and F's listing shape can collapse back. This is not required.

## Known issues

- **Pre-existing clippy failures (toolchain drift).** `cargo clippy --workspace --all-targets -- -D warnings` fails on 17 findings in files this change does not touch (listed above). The Done-when's clippy item cannot pass on this toolchain without edits to those files, which are outside #139's scope (drive-by). T noted the `autoscaling.rs` and `builtin_host.rs:119` ones at RED.
- **Not covered by a test until #140 lands.** The busy reset on the manager path is not implemented (see Deviations).
- **Pull request note for item 4.** The PR description must record, per Done-when item 4: master was 50ed9b4 locally (origin c575130), and `GITLAB_RUNNER_IMAGE` is absent from both, so the refusal form is asserted.

## Files changed

- crates/gridops-core/src/gitlab/mod.rs
- crates/gridops-core/src/gitlab/fake.rs
- crates/gridops-core/src/fleet/service.rs
- crates/gridops-core/src/fleet/targets.rs
- crates/gridops-core/src/fleet/placement.rs
- crates/gridops-core/src/fleet/retry.rs
- crates/gridops-reconciler/src/gitlab_lifecycle.rs (new)
- crates/gridops-reconciler/src/main.rs
- crates/gridops-reconciler/src/fleet.rs
