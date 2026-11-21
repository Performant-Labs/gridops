## Phase 3 Architecture (A, up-front plan review) - 2026-11-21T08:48:27-08:00
- **Decided:** BLOCK, with 1 block and 7 warn findings (docs/handoffs/173/handoff-A.md).
  - The block is about the runner scripts. Both keep `run-single`'s own exit status out of `GITLAB_EXIT_REASONS`:
    - Tart `runner_script` turns 75 or 77 into 70 (`gitlab.rs:160-163`);
    - core `GITLAB_RUNNER_SCRIPT` turns 75, 77 or 69 into 1 (`provisioning.rs:198`).
  - The brief adds 78 to both tables but extends neither guard. So:
    - a `run-single` that itself exits 78 would list as `runner_missing` on Tart;
    - a Docker GitLab container exiting 78 would too, because the manager maps Docker exit codes through core's table (`gridops-manager/src/main.rs:1626-1639`).
  - The warns:
    - (2) the script test is a `scripts/test-*.sh` driver run in `tart-installer.yml` on real macOS bash 3.2, not in `gitlab_tests.rs`;
    - (3) no sourced-helper precedent exists, so whichever shape is chosen follows set rules;
    - (4) reuse the GitHub script's `sync` placement and its `vm_pid` stop-then-`wait`, with one `cleanup`;
    - (5) one `left_service` arm for both failing exits, and counting stays inside `left_service`;
    - (6) the fleet condition is already resolved: no Tart fleet runtime; do not map `ImageUnavailable`; a Docker follow-up for custom images without `gitlab-runner`;
    - (7) shared files with #171 and #172: `gitlab_lifecycle.rs`, `docs/gitlab-runners.md`, `CHANGELOG.md`;
    - (8) the `EX_CONFIG` doc, literal pins in place of a cross-crate dev-dependency, and the full doc/test sweep.
- **Assumed:** with no `survey.md`, the analogues the brief names (the `RegistrationRefused` arm, the two tables, and `prepare-tart-runner-image.sh`'s `sync`) serve as the Reuse map. "The trap cleans up both VMs" is read as: stop and keep the target on interruption, and delete it only when verification fails.
- **Hedged:** gitlab-runner probably never exits 78 itself. The block rests on the repo's own pinned and documented invariant, which already covers the equally unlikely 75 and 77, not on observed behaviour.
- **Evidence:** I read:
  - the brief and the brief gate result;
  - all three prep scripts and `scripts/test-install-tart-agent.sh`, `test-release-preflight.sh`, `ci.yml` and `tart-installer.yml`;
  - tart agent `gitlab.rs`, `gitlab_tests.rs:180-600` and `main.rs` (the table copy, the listing, the 78 exits, the parity and listing tests);
  - core `provisioning.rs` (the table, the Docker script and its guard, the pins) and `tests/gitlab_runner_script.rs`;
  - manager `main.rs` (`ManagedRunner`, `gitlab_exit_reason`) and `gitlab_runner_tests.rs`;
  - reconciler `gitlab_lifecycle.rs` in full, `main.rs:186-240` and `:895-930`, and `gitlab_lifecycle_tests.rs` (`assert_readable`, the refusal tests);
  - host-agent `runtime.rs` and `docker.rs` (the exit diagnostics and the inspect path), and `DiagnosticCode`;
  - `docs/macos-runners.md` and `docs/gitlab-runners.md`, the exit sections;
  - the #171 and #172 briefs on their branches, whose diffs touch only `docs/handoffs`.

  `git diff origin/master HEAD -- crates scripts src tests .github` is empty.

## Phase 3 Architecture (A, up-front plan review, re-review r2) - 2026-11-21T09:00:23-08:00
- **Decided:** PASS, with 0 block and 5 warn findings (docs/handoffs/173/handoff-A.md, which replaces the r1 BLOCK handoff from 7bcd0b2).
  - The r1 block is resolved. The amended brief (2d6e458) extends both collision guards to 78: Tart `runner_script` `75|77|78) … exit 70`, and core `GITLAB_RUNNER_SCRIPT` gets a placeholder in the `finish 1` list. Done-when pins both, using hooks that already exist (`FAKE_GITLAB_RUNNER` `exit-NN`, `Scenario::verified(_, 78)` with `is_reported`).
  - All seven r1 warns are adopted.
  - The new warns:
    - (1) do not copy the zsh `trap cleanup EXIT INT TERM` into the bash scripts. A returning INT trap resumes bash, so an interrupt during the clone check would run the delete-target branch and exit 0. Keep `trap cleanup EXIT`. Delete the target only in the explicit check-failed branch, never in `cleanup`;
    - (2) #171 also has uncommitted edits to `gitlab_lifecycle_tests.rs` (helpers `refusals`, `RefusalEvent`, `assert_refusal` and others; tests at ~:787). Place #173's assertion and test elsewhere and avoid those names;
    - (3) the driver name `test-prepare-tart-runner-images.sh` reads as the GitHub script's test. Follow the `test-install-tart-agent.sh` shape and run the scripts as `/bin/bash`;
    - (4) the two shared-code bullets set opposite defaults. Prefer self-contained scripts; the `include_str!` test pins the GitLab defaults in the entry script;
    - (5) core's doc for the new constant should say only the Tart guest script exits 78.
- **Assumed:** the working-tree `173-brief-result-r1.md` is the brief gate's result on the amended brief (PASS). #171's uncommitted worktree edits show where that branch is heading, though they may still change.
- **Hedged:** finding 1 is a pitfall the brief invites, not an error in the brief. The brief's contract (keep the target on interrupt) is correct and testable, so the finding is a warn, not a block.
- **Evidence:**
  - the brief diff 5e986d8..2d6e458 and the r1 handoff and decisions;
  - all three prep scripts, `tart-installer.yml` and `test-install-tart-agent.sh`;
  - Tart `gitlab.rs:95-185`, `gitlab_tests.rs:190-218` and `:515-597`, and `main.rs:300-321`, `:765-787` and `:1875-1915`;
  - core `provisioning.rs:1-80` and `:120-210`, and `tests/gitlab_runner_script.rs`;
  - reconciler `gitlab_lifecycle.rs:1-40`, `:180-375` and `:580-640`, and the `gitlab_lifecycle_tests.rs` layout;
  - the #171 and #172 branches and worktrees (status, and the test-file hunk list);
  - the actions/runner-images macOS 14/15 arm64 readmes (Bash 3.2.57 only);
  - a scratchpad reproduction of INT-trap resume under `/bin/bash` 3.2.57 and `/bin/zsh`.

  `git diff origin/master HEAD -- crates scripts src tests .github` is empty.

## Phase 4 Tests RED (T) - 2026-11-21T09:29:48-08:00
- **Decided:** RED is valid. 1 new script driver and 11 Rust tests were added or changed across 4 crates (docs/handoffs/173/handoff-T-red.md).
  - `scripts/test-prepare-tart-runner-images.sh`: POSIX `sh`, both prep scripts run as `/bin/bash`, a fake `tart` that keeps one timeline log. 39 of its 55 checks fail today.
  - Core and Tart: the table pins `(78, "runner_missing")` (slice compare, so both array lengths compile). Collision-guard tests: core with a stand-in runner exiting 69/75/77/78; Tart with a new `exit-78` behaviour that must give 70.
  - Tart: the missing-binary guest status goes through `exit_reason`, and the listing reports `runner_missing`. The manager re-parse case is green by design.
  - Reconciler: `assert_runner_missing`, beside an unchanged `assert_readable`, and `a_tart_runner_without_gitlab_runner_counts_once_with_a_readable_reason`. Today it fails with (1, false) != (2, false), which is the live #143 bug.
- **Assumed:**
  - The driver keeps the brief's Done-when name (A warn 3 allowed either), with a header that says what it covers.
  - The success line stays `Prepared <target>…`, and the agent wait stays a counted `sleep` loop.
  - CI wiring, the docs and the CHANGELOG are F's, not test files.
  - The typed `"runner_missing"` fixture in the reconciler test (checked against the table at the end) is a fair exception to that file's "fixtures from the contract" rule: it lets the pass, not the lookup, be what fails at RED.
- **Hedged:**
  - Pinning `GITLAB_EXIT_RUNNER_MISSING` by name in `gitlab_runner_constants_are_pinned` waits for Phase 6. Naming a constant that does not exist would break core's whole lib test binary.
  - The interrupt cases send TERM, not INT, because a backgrounded job ignores SIGINT in a non-interactive `sh`. A returning trap on TERM resumes bash the same way, and the mutation run showed the A-warn-1 trap is caught.
- **Evidence:**
  - A throwaway production draft turned every Rust RED green: core all, Tart 52/52, manager 111/111, reconciler `gitlab_lifecycle` 27/27. It was then reverted; `git diff` shows test hunks only.
  - Scratchpad prototypes of both scripts gave `all passed`, and all 9 mutations failed the driver.
  - Full `--no-fail-fast` runs on the reverted tree: core 1+1 RED, Tart 5 RED, manager 0, reconciler 1 RED. No collateral failures.
  - `cargo fmt --check` and `shellcheck` on the driver are clean. Clippy with the draft was clean apart from two lints on untouched lines (rustc 1.99.1): `unused_async_trait_impl` at manager `main.rs:233` and `assert_is_empty` at reconciler `builtin_host.rs:119`.
  - The manager's tests need a short TMPDIR in this session (`SUN_LEN`).

## Phase 5 Implementation (F) - 2026-11-21T09:43:11-08:00
- **Decided:** all of T's RED is now GREEN across 11 production files (docs/handoffs/173/handoff-F.md).
  - Both bash prep scripts:
    - `sync` is the guest script's last line;
    - the target is stopped with `tart stop` and its `tart run` pid is `wait`ed on;
    - the image is checked in a fresh clone (`<target>-gridops-check-<pid>`) over `tart exec`;
    - one `cleanup` stops either VM and deletes only the clone;
    - the target is deleted only when the check fails and the clone's agent still answers.
  - The scripts stay self-contained, with no sourced helper (A warn 4).
  - Traps: `trap cleanup EXIT`, plus INT/TERM traps that `exit 130/143` (A warn 1).
  - `GITLAB_EXIT_RUNNER_MISSING = 78` / `"runner_missing"` is in both tables. Both collision guards are extended to 78.
  - `Exit::RunnerMissing` shares one failure path in `left_service` with `RegistrationRefused`.
  - CI wiring, the docs and two CHANGELOG bullets are done.
- **Assumed:** a `tart exec` that catches SIGINT and exits normally would let bash continue, so the INT/TERM traps must `exit`; a returning trap or no trap is not enough. A failed check whose clone agent no longer answers is not a missing runner.
- **Hedged:**
  - `cargo clippy --all-targets -D warnings` fails on `clippy::assert_is_empty` on 5 untouched lines: 4 in `gridops-core` (`autoscaling.rs`, `autoscaling_gitlab_tests.rs`) and 1 in reconciler `builtin_host.rs`. With that lint and `unused_async_trait_impl` allowed, it is clean. I left these out of #173 for O.
  - No real VM was run; the brief forbids it.
- **Evidence:**
  - `cargo fmt --check` is clean.
  - Tests: core lib 307/307, core `gitlab_runner_script` 12/12, Tart 52/52, manager 111/111, reconciler 115/115.
  - The script driver gave `all passed` (55 checks) 3 times.
  - `bash -n`, `shellcheck` and `actionlint` are clean.
  - A manual SIGINT to the process group during the clone check exited 130, kept the target, deleted the clone and left nothing running.
  - Evidence entries: docs/handoffs/173/evidence.md.

## T (Phase 7, GREEN + Tier 2)
- **Decided:** the suite is GREEN and Tier 2 is clean, so the verdict is PASS. I repaired my own test `gitlab_runner_constants_are_pinned` to pin `GITLAB_EXIT_RUNNER_MISSING == 78` by name; no production code touched.
- **Assumed:** the pre-existing clippy lints (`assert_is_empty` x6, `unused_async_trait_impl` x1) are outside #173, since none of those lines are in the diff.
- **Hedged:** the CI job was not run on a hosted macOS runner; I ran the driver under this Mac's /bin/bash 3.2.57. No real VM run.
- **Evidence:** I reproduced F's cargo, driver, shellcheck and clippy results. Mutations (drop `sync`, drop the `wait` on the target pid, drop the core guard placeholder) each made the matching test fail; all restored. Details: docs/handoffs/173/handoff-T-green.md.

## A (Phase 7, anti-duplication gate)
- **Decided:** PASS. F extended the exit-reason tables, both collision guards, `Exit` and `left_service` (one shared failure path), and the two prep scripts in place. No parallel path.
- **Assumed:** the brief's own analogue list stands in for the Reuse map (there is no survey.md).
- **Hedged:** one warn only. About 40 lines of cleanup and agent-wait code are duplicated between the two prep scripts, which the brief and handoff-F justify (self-contained is the repo's dominant shape).
- **Evidence:** diff 2d6e458..42ac8e5 read in full for the reconciler, core, Tart agent, manager and both prep scripts; details in docs/handoffs/173/handoff-A-dup.md.
