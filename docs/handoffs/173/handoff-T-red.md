# Handoff-T-red: Phase 4 - Tart GitLab/Bitbucket image prep verifies its image; exit 78 is a provisioning failure (#173)

**Date:** 2026-11-21
**Branch:** claude/gitlab-173-tart-prep-164633
**Brief / wireframe reviewed:** docs/handoffs/173-brief.md (as amended in 2d6e458), docs/handoffs/173-brief-result-r1.md, docs/handoffs/173/handoff-A.md (r2). Wireframe: N/A (no UI change).

## A precondition
Confirmed: A returned PASS on the plan (Phase 3, r2), with 0 blocks and 5 warns (docs/handoffs/173/handoff-A.md). The tests follow these warns:
- **Warn 1 (trap pitfall):** interrupt cases cover both the install and the clone check. A clone whose guest agent never answers must keep the target.
- **Warn 2 (#171 coexistence):** the sibling assertion is named `assert_runner_missing` and sits right after `assert_readable`. The new test sits after `a_refused_registration_counts_once_with_a_readable_diagnostic`. No `Setup` helper is edited, and no #171 helper name is reused.
- **Warn 3 (driver shape):** the driver follows `test-install-tart-agent.sh` (POSIX `sh`, temp dir with an `EXIT` trap, eval'd checks, `all passed` as the last line). It runs each script as `/bin/bash <script>`. It keeps the brief's Done-when name, and its header says it covers the GitLab and Bitbucket scripts, not the GitHub one.
- Brief-gate W-1 and W-2 are also pinned: agent failure is not "binary missing", and the check runs over `tart exec <clone>`.

## Tests authored

### Script test: `scripts/test-prepare-tart-runner-images.sh` (new, POSIX sh, executable)
This is a host-script test, the cheapest tier that runs the real scripts under the system bash. It runs on the CI macOS runner and needs no Rust.

**The fake `tart`.** It goes through `GRIDOPS_TART_BINARY` and models what the scripts use:
- `clone`, `run`, `stop`, `delete`, and `exec` with or without `-i`.
- VMs exist as files.
- A backgrounded `tart run` lasts until `tart stop`, plus 0.3 s, as a real VM's shutdown does. That gap is what makes a script that does not `wait` on the `run` pid fail.
- `exec` on a VM that is not running fails. `delete` of a running VM fails.
- It logs to the same file the script's output is appended to, so the log is one timeline that ordering checks can use.

The fake's switches:
- `FAKE_TART_CLONE=missing`: the clone's check exits 1.
- `FAKE_TART_AGENT=never`: the clone's `/usr/bin/true` probe never answers.
- `FAKE_TART_HANG=install|check`: that exec blocks until the test sends TERM to the script and to the exec, as Ctrl-C reaches both.

`sleep` is stubbed on the script's `PATH`, so the 90-try agent wait ends in moments.

Each case runs for both scripts. GitLab uses a custom `GRIDOPS_TART_GITLAB_RUNNER_BINARY`. Bitbucket uses a custom `GRIDOPS_TART_BITBUCKET_RUNNER_ROOT` and checks `<root>/bin/start.sh`.

| Case | Checks | Brief criterion |
|---|---|---|
| 1 good image | exits 0; the install guest script's last non-blank line is `sync`; the target is stopped after the install; its `tart run` has **ended** before `clone <target> <clone>`; the clone boots after that and two VMs never run at once; the clone's exec text has `test -x` and the configured path (GitLab: also `--version`); the clone's `run` ends before `delete <clone>`; the `Prepared <target>` line comes after the clone check; at the end only the target exists and no `run` is alive | sync last; stop + `wait` before the clone; fresh-clone check; clone stopped and deleted; success only after the check; one VM at a time |
| 2 runner missing | non-zero exit; output other than the success line names the missing path; no success line; both target and clone deleted; nothing running | missing binary: non-zero, path named, target and clone deleted |
| 3 TERM during install | the hang was reached; non-zero exit; no success line; target stopped and its `run` ended before the script exited; target kept; no clone made | interrupted run stops the target and keeps it |
| 4 TERM during clone check | the hang was reached; non-zero exit; no success line; target **not** deleted; clone deleted; nothing running | A warn 1: an interrupt during the check must not reach the delete-target branch |
| 5 clone agent never answers | non-zero exit; no success line; target kept; clone deleted; nothing running | brief-gate W-1 and A warn 1: boot or agent failure is not "binary missing" |
| 6 bad input | wrong argument count exits 64 and runs no `tart`; missing tart exits 69 | keep exit codes 64 and 69 (baseline guard) |

### Core: `crates/gridops-core/src/provisioning.rs` (unit, in `mod tests`)
- `gitlab_exit_reasons_map_each_exit_status_once`: now pins `(78, "runner_missing")` as the fourth entry. It compares slices, so a 3- or 4-length array type compiles either way.

### Core: `crates/gridops-core/tests/gitlab_runner_script.rs` (integration, the real script under bash)
- `a_runner_exiting_with_a_status_of_the_scripts_own_leaves_the_table` (new): a stand-in `run-single` exits 69, 75, 77 and 78 in turn. The script's status must differ from that status, be non-zero and be rejected by `is_reported`.
  - This is the Docker collision guard (A r1 block, brief "Collision guard").
  - 69, 75 and 77 pass today and guard F's edit of the case list.

### Tart agent: `crates/gridops-tart-agent/src/gitlab_tests.rs`
- `exit_statuses_map_to_the_exit_reason_table`: `exit_reason(78) == Some("runner_missing")`. The unmapped list drops 78 and adds 69, 70 and 79.
- `the_guest_script_exits_with_the_status_each_ending_maps_to`: new `FAKE_GITLAB_RUNNER` behaviour `exit-78`, and the case `("run-single itself exiting 78", "200", "exit-78", 70)`. This is the Tart collision guard.
- `a_guest_without_gitlab_runner_exits_runner_missing_naming_the_path` (renamed from `…_exits_78_and_says_where_it_looked`): asserts `run.status.and_then(exit_reason) == Some("runner_missing")` through the table, as the brief asks, and still checks that the path is named and `run-single` never runs.

### Tart agent: `crates/gridops-tart-agent/src/main.rs` (`mod tests`)
- `gitlab_exit_reasons_match_the_core_table`: the same four literals as core, compared as slices. This is how the brief shows the two tables are equal.
- `a_gitlab_runner_gets_its_token_on_stdin_and_lists_its_exit_reason`: the `"78"` ending now expects `exitReason: "runner_missing"` in `GET /v1/runners`.

### Manager: `crates/gridops-manager/src/main.rs` (`mod tests`)
- `a_runners_exit_reason_survives_the_managers_reparse`: also re-parses `runner_missing`. It is **green by design**: `exit_reason` is a free string, which confirms the manager needs no change beyond tests (brief, NIT-2 of the brief gate).

### Reconciler: `crates/gridops-reconciler/src/gitlab_lifecycle_tests.rs` (integration, real DB, stand-in manager, `FakeGitLab`)
- `assert_runner_missing(setup, pool_id, runner_id, outcome)` (new sibling of `assert_readable`, which is unchanged). It checks:
  - the runner row's `failure_reason` names the outcome, `gitlab-runner` and `prepare-tart-gitlab-runner-image.sh`;
  - the pool has exactly one `error` event whose `metadata.outcome` is the outcome and whose `metadata.runnerId` is the runner, and whose message names the script;
  - no `glrt-` token appears in either.
- `a_tart_runner_without_gitlab_runner_counts_once_with_a_readable_reason` (new). It runs a Tart GitLab pool seeded with `provision_failure_count=1`, through the Tart start gate. The runner exits `runner_missing`, then:
  - the count becomes **2** (raised by one, not reset), the state is `backoff`, and `provision_retry_at` is in the future;
  - the runner is out of service and deregistered from GitLab;
  - `assert_runner_missing` holds;
  - after two more passes the count is still 2, there is still one event, and there is still one `CreateRunner` (counted once, nothing started while backing off);
  - no token is stored;
  - after `end_backoff`, a second Tart start completes and the count, circuit and retry time go back to `(0, false, None)` (completed still clears);
  - finally, `exit_reason(78) == "runner_missing"` ties the typed fixture to the published table.

  It sits at the integration tier because counted-once comes from `runner_exited`'s guarded UPDATE together with the pass. A unit test of `left_service` would not see that.

Untouched, and still to keep passing: `registration_refused`, `idle_expired` and `completed` (`a_refused_registration_counts_once…`, `three_refused…`, `an_exited_gitlab_runner_is_deregistered_replaced_and_proves_the_circuit`, `an_old_completed_runner_does_not_reset…`).

## RED confirmation

Run with `CARGO_BUILD_JOBS=2 RUST_TEST_THREADS=2`. The manager's suite also needs a short `TMPDIR` (`TMPDIR=/tmp/g173`): with this session's 49-character TMPDIR its Unix-socket tests fail with `path must be shorter than SUN_LEN`, an environment limit that predates this change. `cargo fmt -- --check` is clean.

**`sh scripts/test-prepare-tart-runner-images.sh`** exits 1 with `39 check(s) failed`: 16 ok, 39 FAIL. For GitLab (Bitbucket is the same, without `--version`):
```
FAIL gitlab: the guest script's last line is sync
FAIL gitlab: a verification clone of the target is made
FAIL gitlab: the target's tart run has ended before the clone is made
FAIL gitlab: the clone boots after that, and never beside another VM
FAIL gitlab: the clone is checked for /Users/admin/gridops-test/gitlab-runner/bin/gitlab-runner
FAIL gitlab: the clone is stopped and deleted, its tart run ended first
FAIL gitlab: success is printed only after the check
FAIL gitlab: only the target is left, and nothing runs
FAIL gitlab: a missing runner exits non-zero
FAIL gitlab: and names the missing path, /Users/admin/gridops-test/gitlab-runner/bin/gitlab-runner
FAIL gitlab: and prints no success line
FAIL gitlab: and deletes the target and the clone
FAIL gitlab: and leaves nothing running
FAIL gitlab: the target is stopped and its tart run has ended
FAIL gitlab: interrupted during the check, the run reached the check
FAIL gitlab: it exits non-zero, with no success line
FAIL gitlab: the clone is deleted and nothing runs
FAIL gitlab: a clone whose agent never answers exits non-zero, with no success line
FAIL gitlab: the target is kept, the clone deleted, nothing runs
FAIL gitlab: the clone runs gitlab-runner --version
```
Each FAIL is the missing feature: no `sync`, no `wait` on the `run` pid, no clone, no check, no failure path. The ok checks are true today and guard what must not change: exit 0 on a good image, the target stopped after the install, the target kept and no clone on an install interrupt, exits 64 and 69. Two checks passed wrongly in my first draft and were tightened before this run:
- "success after the check" now requires a check exec to exist;
- "names the missing path" now ignores the current success line, which also prints the path.

**`cargo test -p gridops-core --no-fail-fast`**: lib 306 passed, 1 failed; `gitlab_runner_script` 11 passed, 1 failed.
```
provisioning::tests::gitlab_exit_reasons_map_each_exit_status_once ... FAILED
  left: [(0, "completed"), (75, "idle_expired"), (77, "registration_refused")]
 right: [(0, "completed"), (75, "idle_expired"), (77, "registration_refused"), (78, "runner_missing")]
a_runner_exiting_with_a_status_of_the_scripts_own_leaves_the_table ... FAILED
Error: run-single exiting 78: the script exited Some(78)
  (stderr: GridOps: gitlab-runner exited with status 78.)
```

**`cargo test -p gridops-tart-agent --no-fail-fast`**: 47 passed, 5 failed.
```
gitlab::tests::exit_statuses_map_to_the_exit_reason_table                     left: None      right: Some("runner_missing")
gitlab::tests::a_guest_without_gitlab_runner_exits_runner_missing_naming_the_path  left: None  right: Some("runner_missing")
gitlab::tests::the_guest_script_exits_with_the_status_each_ending_maps_to     left: Some(78)  right: Some(70)   (the exit-78 case)
tests::a_gitlab_runner_gets_its_token_on_stdin_and_lists_its_exit_reason      left: None      right: Some(String("runner_missing"))
tests::gitlab_exit_reasons_match_the_core_table                               left: [3 entries] right: [4 entries, (78, "runner_missing")]
```

**`cargo test -p gridops-manager --no-fail-fast`** (short TMPDIR): 111 passed, 0 failed. The `runner_missing` re-parse case is green by design.

**`cargo test -p gridops-reconciler --no-fail-fast`**: 114 passed, 1 failed.
```
gitlab_lifecycle_tests::a_tart_runner_without_gitlab_runner_counts_once_with_a_readable_reason ... FAILED
assertion `left == right` failed: one failure more, the one before it kept
  left: (1, false)
 right: (2, false)
```
This is the live bug from #143: the exit is treated as `Unknown`, nothing is counted, and the pool would start again.

**Proof that the RED can turn GREEN (throwaway, never staged).**
- **Production draft.** I wrote a minimal draft of each production change. Production files were backed up to the scratchpad first; `gitlab_lifecycle.rs` and `gitlab.rs` were then reverted with `git checkout`, and the draft hunks were removed from the two files that also hold test edits. `git diff` now shows test hunks only. The draft:
  - the core and Tart tables plus constant;
  - the core `@EXIT_RUNNER_MISSING@` guard;
  - the Tart `75|77|78)` guard;
  - `Exit::RunnerMissing` in a shared `left_service` arm.

  With the draft, all four crates went fully green (core all, Tart 52/52, manager 111/111, reconciler 27/27 `gitlab_lifecycle`). `cargo clippy --all-targets -- -D warnings` on the four crates was clean apart from two lints noted below.
- **Script prototype.** Under the scratchpad, I put draft copies of both prep scripts beside a copy of the driver, which ended in `all passed`. Then I ran nine mutations, and each one fails the driver:

  | Mutation | Checks that fail (per script) |
  |---|---|
  | A's `trap cleanup EXIT INT TERM` | "the target is kept" (TERM during check) |
  | `tart stop` without `wait` | 10 checks |
  | cleanup deletes the target | 4 checks |
  | no `sync` | "last line is sync" |
  | agent failure handled as missing | "target is kept, clone deleted" |
  | success printed before the check | 4 checks |
  | target not stopped before the clone boots | 2 checks |
  | clone never deleted | 6 checks |
  | missing path keeps the target | "deletes the target and the clone" |

## Ready for F
RED is valid. F may implement against these tests. Notes for F:

1. **What the script driver relies on.** Each is how the scripts behave today or what the brief names:
   - the success line still starts `Prepared <target>`;
   - the agent probe is `tart exec <vm> /usr/bin/true`;
   - the agent wait stays a counted loop with `sleep`, as today. A `SECONDS` deadline would make case 5 take minutes;
   - the clone check goes through `tart exec <clone> …`, and its arguments or stdin contain `test -x` and the configured path (GitLab: also `--version`);
   - the install's guest script ends in `sync`: stdin of the `-i` exec for GitLab, the `-lc` argument for Bitbucket.
2. **CI wiring is F's.** In `.github/workflows/tart-installer.yml`:
   - add both prep scripts and the driver (and any sourced helper) to both `paths` lists;
   - add a step that runs `sh scripts/test-prepare-tart-runner-images.sh` and passes only on a last line of `all passed` (copy the installer step);
   - update the header comment and job name.

   Also update `docs/macos-runners.md` ~261. I do not edit the workflow or the docs.
3. **The constant name is pinned in Phase 6, not now.** `gitlab_runner_constants_are_pinned` does not yet name `GITLAB_EXIT_RUNNER_MISSING`, because naming a constant that does not exist would stop core's whole lib test binary from compiling and hide the other RED. I will add `GITLAB_EXIT_RUNNER_MISSING == 78` to it in Phase 6. The table pins the value now.
4. **`assert_runner_missing` sets the reason's wording.** The runner's `failure_reason` must contain `runner_missing` (any case), `gitlab-runner` and `prepare-tart-gitlab-runner-image.sh`. The pool's one error event must carry `metadata.outcome = "runner_missing"`, `metadata.runnerId`, and a message naming the script. This matches the refusal event's shape.
5. **Two clippy lints are outside this change.** With this Mac's rustc 1.99.1, `cargo clippy --all-targets -- -D warnings` reports two lints on lines this run does not touch:
   - `clippy::unused_async_trait_impl` at `crates/gridops-manager/src/main.rs:233`;
   - `clippy::assert_is_empty` at `crates/gridops-reconciler/src/builtin_host.rs:119`.

   With those two allowed, the four crates are clippy-clean with my tests and the draft. F should report them, and not fold them into #173, unless O decides otherwise.
