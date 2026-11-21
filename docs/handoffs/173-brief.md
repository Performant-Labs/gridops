# [GitLab: Tart image prep loses gitlab-runner, and the pool hides the failure](https://github.com/Performant-Labs/gridops/issues/173)

Part of epic #136. Found by #143's live end-to-end run (check C2c). Two parallel sessions own #171/#174 (GitLab connection verification and the connection form) and #172 (pool validation); do not touch those files.

## Need

1. **The prep script loses the binary.** `scripts/prepare-tart-gitlab-runner-image.sh` installs `gitlab-runner` in the running VM, prints its `--version`, then the EXIT trap runs `tart stop` without flushing the guest's disk. The write is lost: a fresh clone of the image has no `/Users/admin/gitlab-runner`. A `sync` after the install fixed it in a live rerun (binary present, 136,204,080 bytes; a macOS job then passed). `scripts/prepare-tart-bitbucket-runner-image.sh` has the same shape (install, no `sync`, `tart stop` in the trap). `scripts/prepare-tart-runner-image.sh` (GitHub) already runs `sync` in its guest script: confirm and leave it.
2. **The pool hides the failure.** A Tart GitLab runner whose image has no `gitlab-runner` exits 78 (`runner_script` in `crates/gridops-tart-agent/src/gitlab.rs`). 78 is outside `GITLAB_EXIT_REASONS`, so the runner listing carries no `exitReason`, the reconciler's `gitlab_lifecycle::Exit::from_exit_reason` returns `Unknown`, and `left_service` does nothing for `Unknown`. Live: the `gl-macos` pool started a VM about once a minute for 6 minutes, each runner exited 78, the pool's events said "Provisioned 1" each time, `provisionFailureCount` stayed 0, a GitLab runner was registered and deleted on each attempt, and the job stayed pending.

## Scope

### Prep scripts

- `prepare-tart-gitlab-runner-image.sh`: after the install, flush the guest's disk (`sync` in the guest, inside or right after the guest script), then stop the target VM and wait until Tart reports it stopped before going on.
- Then verify in a **fresh clone** of the target image: clone it to a temporary name, boot it headless, wait for the guest agent as today, check that the runner binary exists, is executable and `--version` runs, stop it and delete the temporary clone (on success, on failure and on an interrupted run: the trap cleans up both VMs). Only one VM runs at a time: the target is stopped before the clone boots.
- Success is printed only after that check passes. If it fails, exit non-zero with a message naming the missing path, and delete the target image so no broken image is left under the target name.
- `prepare-tart-bitbucket-runner-image.sh`: the same `sync`, stop, fresh-clone check (`test -x <runner_root>/bin/start.sh`), cleanup and failure handling. Share the clone-boot-check-cleanup logic between the two scripts if that stays simple (for example a small sourced helper under `scripts/`); otherwise keep each script self-contained, with the same shape.
- Keep each script's arguments, environment variables, exit codes for bad input (64, 69) and the guest paths unchanged.

### Exit 78 is a provisioning failure

- Give exit 78 a name in the exit-reason table: add a constant (e.g. `GITLAB_EXIT_RUNNER_MISSING = 78`, reason `"runner_missing"`, or a better name you justify) to `GITLAB_EXIT_REASONS` in `crates/gridops-core/src/provisioning.rs` **and** the copy in `crates/gridops-tart-agent/src/main.rs` (the Tart agent does not depend on core; keep names and values equal). Update the doc comments that say the table is exactly 0/75/77 and the `runner_script` doc.
- The Tart agent's `GET /v1/runners` then reports `exitReason` for a 78 exit; the manager relays it unchanged (confirm `crates/gridops-manager` needs no change beyond tests).
- Reconciler (`crates/gridops-reconciler/src/gitlab_lifecycle.rs`): a new `Exit` variant for it. `left_service` treats it as `RegistrationRefused` is treated: write a readable `failure_reason` on the runner row (it names the problem and the fix: the pool's Tart image has no `gitlab-runner`; prepare it with `scripts/prepare-tart-gitlab-runner-image.sh`), record an `error` system event on the pool with the outcome name, and `count_failure` so the pool's failure count and backoff rise. It must not clear provision failures. Make the reason as visible on the pool as `registration_refused`'s is (the same event and runner-row paths; check what the pool page and API surface for `registration_refused` and match it).
- Fleet path: `Exit::from_inspect` and `fleet_start_failed` map diagnostics. If a fleet (host agent) runtime can run a Tart GitLab runner and observe exit 78, map it to an existing `DiagnosticCode` (e.g. `ImageUnavailable`) and count it the same way; if no fleet runtime can produce 78 (the Docker runner image always ships `gitlab-runner`), say so in the handoff with the evidence and change nothing there. Do not add a `DiagnosticCode` variant unless the protocol needs one.
- No UI change is expected: the pool already shows its events, failure count and runner `failure_reason`. If the walkthrough of what exists shows the reason is not visible, report it rather than building UI.

## Constraints

- No deploy. Nothing GitLab ships to Mars until epic #136 is done.
- Rust builds share this Mac: keep `CARGO_BUILD_JOBS=2` and `RUST_TEST_THREADS=2` (`.claude/settings.local.json`).
- Tests never boot a real VM: the scripts are tested with a fake `tart` (`GRIDOPS_TART_BINARY`) that records its calls. Run no more than one real Tart VM at a time anywhere.
- Work headlessly.

## Done when

- A script test (bash with a fake `tart` that records each call and can be told the clone has no binary) shows, for the GitLab script and the Bitbucket script: the guest runs `sync` after the install; the target is stopped before the verification clone is made or booted; the clone is checked for the binary (`test -x` and, for GitLab, `--version`) and then stopped and deleted; success is printed only after the check; a missing binary exits non-zero, prints the missing path, deletes the target and the clone, and prints no success line. The test runs in CI (the existing `cargo test` of `gridops-tart-agent`, which already reads the GitLab script with `include_str!`, or a script test wired into `ci.yml`/`tart-installer.yml`), and the bash it runs is compatible with macOS's bash 3.2.
- Tests cover the exit-78 case in the reconciler: a Tart GitLab runner that exits with the new `exitReason` fails with the readable `failure_reason`, records the error event, raises the pool's `provisionFailureCount` by one (counted once across repeated reconcile passes) and its backoff, and does not reset failures; a following `completed` exit still clears them. Existing `registration_refused`, `idle_expired` and `completed` tests keep passing.
- Exit-reason tests: core and the Tart agent map 78 to the new name (the tart agent's `exit_statuses_map_to_the_exit_reason_table` no longer lists 78 as unmapped), the two tables are equal, and the Tart agent's listing reports `exitReason` for a runner that exited 78; the manager test shows it survives the re-parse.
- `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test` for `gridops-core`, `gridops-tart-agent`, `gridops-manager` and `gridops-reconciler`, and `bash -n` / `shellcheck` (if installed) on the changed scripts pass.
- `docs/macos-runners.md` (GitLab and Bitbucket image sections) and `docs/gitlab-runners.md` say the scripts verify the image in a fresh clone, and name the new exit reason where the exit reasons are listed. `CHANGELOG.md` gets an entry.
