# Handoff-F: Phase 5 - Tart GitLab/Bitbucket image prep verifies its image; exit 78 is a provisioning failure (#173)

**Date:** 2026-11-21
**Branch:** claude/gitlab-173-tart-prep-164633
**Issue:** #173

## What was done
- `scripts/prepare-tart-gitlab-runner-image.sh` changes:
  - The guest script now ends in `sync`.
  - The target's `tart run` pid is recorded, and the target is stopped with `tart stop` and then `wait`.
  - The script then clones the target to `<target>-gridops-check-<pid>`, boots the clone, waits for its guest agent, and checks it with `tart exec <clone> /bin/sh -c 'test -x "$1" && "$1" --version' sh <runner_binary>`.
  - One `cleanup` handles both VMs. `Prepared …` is printed only after the check.
  - A failed check deletes the target and exits 1, naming the path.
  - Arguments, environment variables, the 64/69 exits and guest paths are unchanged.
- `scripts/prepare-tart-bitbucket-runner-image.sh`: the same shape. `sync` is the last line of the `-lc` guest script, and the clone check is `test -x <runner_root>/bin/start.sh`. A header comment was added.
- `crates/gridops-core/src/provisioning.rs`:
  - `GITLAB_EXIT_RUNNER_MISSING = 78` (`EX_CONFIG`), documented per A warn 5: only the Tart guest script exits with it.
  - Table entry `(78, "runner_missing")`, with the table now `[_; 4]`.
  - `@EXIT_RUNNER_MISSING@` added to the Docker script's `finish 1` guard and substituted in `gitlab_runner_script()`.
  - The script's doc now states the collision-guard invariant.
- `crates/gridops-tart-agent/src/main.rs`: the same constant and table entry in the crate's copy.
- `crates/gridops-tart-agent/src/gitlab.rs`: the `runner_script` guard is now `75|77|78) … exit 70`, and its doc names `runner_missing`/`EX_CONFIG` and the guard.
- `crates/gridops-manager/src/main.rs`: doc only. `ManagedRunner::exit_reason` lists `runner_missing`, with no code change; the manager relays the free string.
- `crates/gridops-reconciler/src/gitlab_lifecycle.rs`:
  - `Exit::RunnerMissing`, with entries in `from_exit_reason` and `name()`.
  - `runner_missing_reason()`.
  - `left_service` reshaped so that both failing exits share one failure path, with a title and reason per exit.
  - The module's Outcomes doc updated. `from_inspect` is unchanged.
- `.github/workflows/tart-installer.yml`:
  - Both prep scripts and the driver added to both `paths` lists.
  - A new step runs `sh scripts/test-prepare-tart-runner-images.sh` and passes only on a last line of `all passed`.
  - The runner check now requires `/bin/bash`.
  - The header comment and job name were updated.
- `docs/macos-runners.md`:
  - The CI paragraph (~261) mentions the prep scripts' test.
  - The Bitbucket and GitLab image sections describe the fresh-clone check and the failure and interrupt behaviour.
  - The exit table gains `78 | runner_missing`, plus a sentence that both failing reasons count as provisioning failures.
  - The "any other exit" sentence now says 75, 77 or 78.
- `docs/gitlab-runners.md`: a `runner_missing` bullet in the runner lifecycle list; the macOS section mentions the fresh-clone check.
- `CHANGELOG.md`: a new `### Fixed` subsection under Unreleased with two #173 bullets, separate from #171/#172.

## Design decisions
- **Each script stays self-contained, with no sourced helper.** This is A warn 4 and brief W3.
  - The repo has no sourcing precedent.
  - The `include_str!` test reads the GitLab entry script for its defaults.
  - The duplication is about 40 lines of identical `cleanup`/`stop_vm`/`wait_for_agent`, which a reviewer can diff between the two files. A helper would add a fourth file to CI `paths`, a `BASH_SOURCE` lookup and a non-executable file, to save that.
- **Traps: `trap cleanup EXIT` plus `trap 'exit 130' INT` and `trap 'exit 143' TERM`.** This follows A warn 1.
  - I did not copy the zsh `trap cleanup EXIT INT TERM` line: a returning handler would resume bash into the delete-target branch.
  - Why add exiting INT/TERM traps instead of relying on bash's default death:
    - If `tart exec` catches SIGINT and exits normally (not killed by the signal), bash's wait-and-cooperative-exit would let the script continue into `if ! check` and delete the target.
    - An exiting trap ends the run after the foreground command returns, however it returned.
  - Verified by hand: SIGINT to the whole process group during the clone check exited 130, kept the target, deleted the clone and left nothing running.
- **`cleanup` never deletes the target.**
  - It stops whichever VM has a recorded pid, then `wait`s on that pid.
  - It deletes the clone only when `check_image` is set, which happens after `tart clone` succeeds. So a name clash with an existing VM never deletes someone else's VM.
  - The target is deleted only in the explicit failed-check branch.
- **A failed check is confirmed before the target is deleted.** If the check exec fails, the script probes the clone's agent once more (`tart exec <clone> /usr/bin/true`).
  - Only if the agent still answers is the failure taken as "runner missing", and the target deleted.
  - Otherwise the target is kept and the script exits 1.
  - This follows A warn 1's "only on a positive 'binary missing' answer from a clone whose guest agent answered". A `tart exec` transport error is not read as a broken image.
- **The check runs `/bin/sh -c '…' sh <path>`, with the path as an argv element.** Nothing is interpolated into the guest script. This is safe for Bitbucket's free-form runner root, and it is the same argv channel the GitLab install already uses (`bash -s -- …`).
- **`left_service` uses one failure path.** The `match` returns early for `Completed | IdleExpired`, which clear failures, and for `Unknown`, which does nothing. It yields `(title, reason)` for `RegistrationRefused` (connection-aware, unchanged text) and for `RunnerMissing`. A single shared block then writes `failure_reason`, records the `error` event with `outcome: exit.name()`, and calls `count_failure`. Nothing is copied, and the counting stays inside `left_service` (A r1 W5).
- **The `runner_missing` reason text:** "The pool's Tart image has no gitlab-runner (runner_missing): prepare the image with scripts/prepare-tart-gitlab-runner-image.sh and point the pool at it." It names the outcome, the problem and the fix, and it is used as both the runner's `failure_reason` and the event message, as the refusal's is. The event title is "GitLab runner missing from the Tart image".
- **Literals stay in the Tart guest script.** It keeps `exit 78` and `75|77|78)`, like the existing `75`/`77`/`69` literals. The table equality is pinned by T's literal tests in both crates.

## Reuse / extend-vs-new
- Extended in place, per the brief's analogues (no `survey.md`; A used the brief's list as the Reuse map):
  - `GITLAB_EXIT_REASONS` and its Tart copy;
  - both collision guards;
  - `Exit` and the `RegistrationRefused` arm of `left_service`, now shared;
  - both bash prep scripts, using `prepare-tart-runner-image.sh`'s `sync` placement and `vm_pid` stop-then-`wait` shape.
- The only new objects:
  - the `Exit::RunnerMissing` variant and `runner_missing_reason()`, which the brief asked for;
  - three small functions inside each script (`stop_vm`, `cleanup`, `wait_for_agent`). `cleanup` replaces the old one-line cleanup, and `wait_for_agent` replaces the inline agent loop so it can serve both VMs.
- No parallel path.

## Architecture notes for A
- **Public contract:** a new published `exitReason` value, `runner_missing`, in `GET /v1/runners` (Tart agent, relayed by the manager), and a new public const `gridops_core::provisioning::GITLAB_EXIT_RUNNER_MISSING`. `GITLAB_EXIT_REASONS`' type changed from `[(i32, &str); 3]` to `[(i32, &str); 4]`. Every user iterates it, so nothing outside the touched crates breaks.
- **Unchanged:** no schema or migration change, no new dependency, no change in dependency direction, and the fleet path is untouched (`from_inspect`, `DiagnosticCode`).
- **#171 coexistence (A warn 2):** in `gitlab_lifecycle.rs` I touched only the imports, `Exit`, its `from_exit_reason`/`name`, `runner_missing_reason` (new, beside `refusal_reason`), `left_service` and the Outcomes doc. I did not touch `deregister_runner`, `fail_unstarted` or the sweep. I edited no tests.
- **CI:** one more step in `tart-installer.yml`'s existing job, on the same runner, at no extra cost.

## Deviations from spec / wireframe
None. Two notes:
- The brief expected no UI change and I made none. The `runner_missing` reason reaches the same runner-row `failureReason` and pool-event surfaces as `registration_refused` (see evidence.md).
- The job name in `tart-installer.yml` changed to "Tart installer and image preparation tests (macOS)". The workflow `name:` and concurrency group are unchanged, and nothing requires this check today (the workflow's own comment says so).

## Tier 1 self-check (incl. tests now GREEN)
Run with `CARGO_BUILD_JOBS=2 RUST_TEST_THREADS=2`. Manager and reconciler were run with `TMPDIR=/tmp/g173`: the manager's Unix-socket tests hit `SUN_LEN` with this session's long TMPDIR, as T noted.

```
cargo fmt --all -- --check                                   -> clean
cargo test -p gridops-core -p gridops-tart-agent --no-fail-fast
  gridops-core lib                 test result: ok. 307 passed; 0 failed
  gridops-core gitlab_runner_script test result: ok. 12 passed; 0 failed
  (every other gridops-core integration binary: ok, 0 failed)
  gridops-tart-agent               test result: ok. 52 passed; 0 failed
cargo test -p gridops-tart-agent -p gridops-manager -p gridops-reconciler --no-fail-fast
  gridops-manager                  test result: ok. 111 passed; 0 failed
  gridops-reconciler               test result: ok. 115 passed; 0 failed
  gridops-tart-agent               test result: ok. 52 passed; 0 failed   (re-run after the docs edit: 52 passed)
sh scripts/test-prepare-tart-runner-images.sh                -> 55 ok, last line "all passed" (3 runs, all passed)
bash -n + shellcheck on both prep scripts and the driver     -> clean (/bin/bash is GNU bash 3.2.57)
actionlint .github/workflows/tart-installer.yml              -> clean
cargo clippy -p gridops-core -p gridops-tart-agent -p gridops-manager -p gridops-reconciler --all-targets -- -D warnings
  -> fails only on clippy::assert_is_empty in files this run does not touch:
     crates/gridops-core/src/autoscaling.rs:1810, :1896
     crates/gridops-core/src/autoscaling_gitlab_tests.rs:288, :384
     crates/gridops-reconciler/src/builtin_host.rs:119
  with -A clippy::assert_is_empty -A clippy::unused_async_trait_impl -> Finished, no warnings
```
T's RED tests are now GREEN:
- core `gitlab_exit_reasons_map_each_exit_status_once` and `a_runner_exiting_with_a_status_of_the_scripts_own_leaves_the_table`;
- the Tart agent's five;
- reconciler `a_tart_runner_without_gitlab_runner_counts_once_with_a_readable_reason`;
- the 39 failing script checks.

The `registration_refused`, `idle_expired` and `completed` tests still pass, within the 115 and 52 totals above.

I also checked the interrupt by hand, outside the driver, since it sends TERM rather than INT. I used T's fake `tart` and sent SIGINT to the script's process group while the clone check hung. Result: `exit 130`, VMs left: the target only, nothing running, the log shows `stop tgt`, `stop <clone>`, `delete <clone>`, and no `delete tgt`.

## Evidence appendix
docs/handoffs/173/evidence.md (8 entries):
- counted once comes from `runner_exited`'s guarded UPDATE;
- `record_provision_failure` raises the count by one;
- the reconciler path from `exitReason` to `left_service`;
- the Tart listing uses the table;
- the manager maps Docker exit codes through core's table;
- no Tart fleet runtime;
- the `include_str!` pin on the GitLab script;
- the runner row shows `failureReason`.

## Tests that look wrong (for T)
None. A reminder from T's own handoff (note 3): `gitlab_runner_constants_are_pinned` should now pin `GITLAB_EXIT_RUNNER_MISSING == 78` by name. The constant exists now, so that edit compiles.

## Known issues
- **Two clippy lints predate this change.** `cargo clippy --all-targets -- -D warnings` fails on this Mac's toolchain on lines this change does not touch:
  - `clippy::assert_is_empty`, 5 sites, listed above. T reported only the reconciler one; the 4 in `gridops-core` are the same lint.
  - `clippy::unused_async_trait_impl` at `crates/gridops-manager/src/main.rs:233`, reported by T.

  I left them out of #173. They are for O to route.
- **Out of scope, as the brief says:** a Docker GitLab pool whose `docker_image` lacks `gitlab-runner` exits 127 and is still reported without a reason. This is the brief's follow-up and needs a separate issue.
- **Not run:** a real-VM run of the two prep scripts. The brief forbids real VMs in tests, and no deploy is allowed. The fake `tart` covers the ordering, cleanup and failure paths. The fix itself (`sync` before stop) was confirmed in #143's live rerun.

## Files changed
- .github/workflows/tart-installer.yml
- CHANGELOG.md
- crates/gridops-core/src/provisioning.rs
- crates/gridops-manager/src/main.rs
- crates/gridops-reconciler/src/gitlab_lifecycle.rs
- crates/gridops-tart-agent/src/gitlab.rs
- crates/gridops-tart-agent/src/main.rs
- docs/gitlab-runners.md
- docs/macos-runners.md
- scripts/prepare-tart-bitbucket-runner-image.sh
- scripts/prepare-tart-gitlab-runner-image.sh
