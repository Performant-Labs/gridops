# Handoff-T-green: Phase 7 - Tart GitLab/Bitbucket image prep verifies its image; exit 78 is a provisioning failure (#173)

**Date:** 2026-11-21
**Branch:** claude/gitlab-173-tart-prep-164633
**Issue:** #173
**Handoff-F reviewed:** docs/handoffs/173/handoff-F.md
**Handoff-T-red:** docs/handoffs/173/handoff-T-red.md

## GREEN confirmation
Run with `CARGO_BUILD_JOBS=2 RUST_TEST_THREADS=2 TMPDIR=/tmp/g173` (short TMPDIR for the manager's Unix-socket tests, as in the RED handoff).

- `sh scripts/test-prepare-tart-runner-images.sh`: all checks ok, last line `all passed` (run twice by me, plus once more after mutation restore).
- `cargo test -p gridops-core -p gridops-tart-agent -p gridops-manager -p gridops-reconciler --no-fail-fast`: every binary `ok`, 0 failed.
  - gridops-core lib 307, `gitlab_runner_script` 12, other core integration binaries ok.
  - gridops-tart-agent 52, gridops-manager 111, gridops-reconciler 115.
- Every Phase 4 test is GREEN, including the reconciler `a_tart_runner_without_gitlab_runner_counts_once_with_a_readable_reason`. The untouched `registration_refused`, `idle_expired` and `completed` tests still pass inside the totals.

Spot-checks that the tests pin behavior (mutate production, watch the test fail, restore):
- Removed the guest script's `sync` from the GitLab prep script: driver fails `gitlab: the guest script's last line is sync`.
- Removed `wait "$target_pid"` after the target's `tart stop`: driver fails three checks (last line is sync is a side effect of the sed; the `tart run has ended before the clone` and `never beside another VM` checks fail on their own).
- Removed `@EXIT_RUNNER_MISSING@` from core's `finish 1` guard: `a_runner_exiting_with_a_status_of_the_scripts_own_leaves_the_table` fails (`the script exited Some(78)`).
- All mutations were restored (`git checkout` for the script, a saved copy for provisioning.rs); the suite is green again.

## Test repair made this phase (mine, not F's)
`crates/gridops-core/src/provisioning.rs`, test `gitlab_runner_constants_are_pinned`: it pinned only 0/75/77. It now also pins `GITLAB_EXIT_RUNNER_MISSING` by name, `(0, 75, 77, 78)`. This was the reminder in my RED handoff (note 3) and F's "Tests that look wrong" section. No production line changed. Re-ran: passes; `cargo fmt --all -- --check` clean.

## Tier 1 results
| Check | Result |
|---|---|
| `cargo fmt --all -- --check` | PASS |
| `bash -n` on both prep scripts | PASS |
| `shellcheck` on both prep scripts and the driver | PASS |
| System `/bin/bash` is GNU bash 3.2.57; the driver runs the scripts under it | PASS |
| `cargo clippy` on the 4 crates, `--all-targets -D warnings -A clippy::assert_is_empty -A clippy::unused_async_trait_impl` | PASS (Finished, no warnings) |
| `cargo clippy ... -D warnings` with no allowances | FAIL on pre-existing lints only (see Advisory) |
| Existing tests still pass | PASS |
| Server start / API smoke | N/A: no server code path changed beyond a published `exitReason` string; the Tart listing and manager re-parse tests cover it |

## Tier 2 results
- Test coverage per criterion: PASS (table below).
- Test quality (test-quality.md section 7): PASS. Each test names one behavior; the script cases sit at the host-script tier (cheapest that runs the real scripts under bash 3.2), the reconciler case at the integration tier because counted-once comes from `runner_exited`'s guarded UPDATE, the rest are unit. The manager test is green by design (confirms no code change needed) and is a single added re-parse literal, not a new test. No redundant tests found. The suite is proportionate: one driver, 1 new reconciler test, 1 new core integration test, a handful of edits to existing tests.
- Type safety: PASS. No casts or `any`; `GITLAB_EXIT_REASONS` is `[(i32, &str); 4]` in both crates.
- Error handling: PASS. Missing binary, clone agent that never answers, interrupt during install, interrupt during the check, and bad input (64, 69) are each tested for both scripts.
- Data integrity: PASS. Counted once across repeated passes, backoff raised, completed still clears (reconciler test). The target is deleted only on a confirmed missing binary; cleanup never deletes the target.
- API contract: PASS. `exitReason: "runner_missing"` is reported by the Tart listing, survives the manager's re-parse, and the two tables are pinned to the same literals.
- Security: PASS. The check passes the path as an argv element, nothing is interpolated into the guest script; the reason text carries no token (assertion in `assert_runner_missing`).
- Collision guards: PASS in both runners (Tart `75|77|78`, core `@EXIT_RUNNER_MISSING@`), each backed by a failing-on-mutation test.
- Migration safety: N/A, no schema change.
- Playwright: N/A, no UI surface.
- Evidence appendix: docs/handoffs/173/evidence.md has the 8 entries F listed; the tests rely on no further unchanged-source fact. No additions needed.
- Cross-check of F's results: every F-reported command reproduced with the same counts; the only difference is that I ran clippy without allowances too and got the same pre-existing failures (the manager's `unused_async_trait_impl` at main.rs:233 and `assert_is_empty` in `vm_slots.rs:755`, neither in this diff).
- Docs and CHANGELOG: `runner_missing` is in docs/macos-runners.md (exit table, counted-as-failure sentence, "any other exit"), docs/gitlab-runners.md and CHANGELOG.md.
- CI wiring: `.github/workflows/tart-installer.yml` lists both prep scripts and the driver in both `paths` lists and runs the driver with the `all passed` last-line rule.

## Acceptance criteria status
- Prep scripts: sync last, target stopped and `run` waited before the clone, clone checked (`test -x`, GitLab `--version`), clone stopped and deleted, success after check, missing binary non-zero with path named and target and clone deleted, interrupt keeps the target: PASS (driver, both scripts).
- CI runs the driver on macOS bash 3.2 and docs mention it: PASS (workflow and docs/macos-runners.md read; not run on a hosted runner).
- Exit 78 reconciler case (readable reason, error event, count +1 once, backoff, no reset, completed clears): PASS (`a_tart_runner_without_gitlab_runner_counts_once_with_a_readable_reason`).
- Collision guards (Tart 78 gives 70; core 78 rejected by `is_reported`): PASS.
- Exit-reason tests (core and Tart map 78, tables equal, listing reports it, manager re-parse): PASS.
- fmt, clippy, cargo tests, bash -n, shellcheck: PASS, with clippy allowances for pre-existing lints noted below.
- Docs and CHANGELOG: PASS.

## Blocking issues
None.

## Advisory notes
- `cargo clippy --all-targets -- -D warnings` fails on this Mac's toolchain on lines this change does not touch: `clippy::assert_is_empty` (`gridops-core/src/autoscaling.rs`, `autoscaling_gitlab_tests.rs`, reconciler `builtin_host.rs`, manager `vm_slots.rs:755`) and `clippy::unused_async_trait_impl` (manager `main.rs:233`). For O to route; not #173.
- No real VM was run (brief forbids it). The fake-`tart` driver covers ordering, cleanup and failure paths; the `sync` fix itself was confirmed in #143's live rerun.
- A Docker GitLab pool whose `docker_image` lacks `gitlab-runner` (exit 127) is still reported without a reason: the brief's follow-up, not this issue.
- U is N/A: no UI surface changed.
