# Handoff-S: Phase 8 - Tart GitLab/Bitbucket image prep verifies its image; exit 78 is a provisioning failure (#173)

**Date:** 2026-11-21
**Branch:** claude/gitlab-173-tart-prep-164633
**Issue:** #173
**Brief / spec:** docs/handoffs/173-brief.md (as amended in 2d6e458)
**Handoff-T reviewed:** docs/handoffs/173/handoff-T-green.md (and handoff-T-red.md)
**Handoff-A reviewed:** docs/handoffs/173/handoff-A.md (plan review, PASS), docs/handoffs/173/handoff-A-dup.md (anti-duplication gate, PASS)
**Handoff-F reviewed:** docs/handoffs/173/handoff-F.md (with evidence.md)
**Operator-facing report:** not produced. No visual surface: shell scripts, Rust crates, CI workflow and docs only.

## A precondition
Confirmed: A returned PASS on the plan (handoff-A.md, 0 blocks, 5 warns) and PASS on the anti-duplication gate (handoff-A-dup.md, 1 warn that needs no action).

## T precondition
Confirmed: handoff-T-green.md reports "Blocking issues: None". Its only advisory is the clippy toolchain drift, which I re-checked below.

## Browser-tool and visual-diff-tool preconditions
N/A. The change has no rendered output (no `src/` file changed), so there is nothing to diff at 360/768/1280 and no HTML report is owed.

## Spec / preview sanity check
No defect in the source of truth. The brief's own conventions hold: 78 is `EX_CONFIG` in `sysexits.h`, the collision-guard rationale is sound (a table status must always be the script's own verdict), and the done-when criteria are testable. No ADVISORY-HOLD.

## What I ran myself (independent of F and T)
- `/bin/bash --version`: GNU bash 3.2.57 (the driver runs both scripts under it).
- `sh scripts/test-prepare-tart-runner-images.sh`: 3 runs, each ends `all passed`, about 12 s, no flake.
- `bash -n` on both prep scripts, `shellcheck` on both prep scripts and the driver, `actionlint` on `tart-installer.yml`: clean.
- `cargo fmt --all -- --check`: clean.
- `CARGO_BUILD_JOBS=2 RUST_TEST_THREADS=2 TMPDIR=/tmp/g173 cargo test -p gridops-core -p gridops-tart-agent -p gridops-manager -p gridops-reconciler --no-fail-fast`: exit 0, 0 failed. Core lib 307, `gitlab_runner_script` 12, Tart agent 52, manager 111, reconciler 115. Counts match F and T.
- Clippy, the exact CI command on the CI-pinned toolchain: `cargo +1.98.1 clippy --workspace --all-targets --locked -- -D warnings -A clippy::unused_async_trait_impl` (from `.github/workflows/ci.yml:113`, toolchain pinned at `ci.yml:93`): exit 0, no warnings.
  - F and T saw `clippy::assert_is_empty` failures on this Mac's default toolchain (stable 1.99.1). Those are newer-clippy lints in files this diff does not touch (`autoscaling.rs`, `autoscaling_gitlab_tests.rs`, `builtin_host.rs`, `vm_slots.rs`). CI pins 1.98.1 for that reason (`ci.yml:86-91`), so they do not affect the done-when "clippy passes" criterion as CI measures it. No action for #173.
- One mutation of my own: I replaced both scripts' `trap cleanup EXIT` / `trap 'exit 130' INT` / `trap 'exit 143' TERM` with the zsh script's `trap cleanup EXIT INT TERM` in a scratchpad copy. The driver still passed. That is an equivalent mutant at the behaviour level: the trap's cleanup deletes the clone first, so the "agent still answers" probe fails and the target is kept anyway. See advisory 1. T's three spot-checks (no `sync`, no `wait` on the target's `run` pid, no `@EXIT_RUNNER_MISSING@` in core's guard) each fail a test, which I accept.

## Spec compliance

| Brief requirement | Compliant | Evidence |
|---|---|---|
| GitLab script: `sync` after the install, in the guest script | YES | `prepare-tart-gitlab-runner-image.sh`: `sync` is the guest heredoc's last line after `--version`. Driver check "the guest script's last line is sync" |
| Bitbucket script: same `sync` | YES | `sync` is the last line of the `-lc` guest script, after `test -x .../bin/start.sh` |
| GitHub script already syncs; leave it | YES | `prepare-tart-runner-image.sh:~91` has `sync`; `git diff 2d6e458 HEAD --stat` for it is empty |
| Stop the target with `tart stop`, then `wait` its recorded `tart run` pid; no `tart list`/jq polling | YES | `target_pid=$!`, `tart stop`, `wait "$target_pid" || true`. Driver: "the target's tart run has ended before the clone is made" |
| Fresh-clone check: clone to a temp name, boot headless, wait for the agent, check path exists, executable, `--version` runs (GitLab) / `test -x .../bin/start.sh` (Bitbucket) | YES | `${target}-gridops-check-$$`, `run --no-graphics`, shared `wait_for_agent`, `tart exec <clone> /bin/sh -c 'test -x "$1" && "$1" --version' sh <path>` (GitLab); `test -x "$1"` (Bitbucket). Path passed as argv, nothing interpolated into the guest script |
| One VM at a time; target stopped before the clone boots | YES | Driver: "the clone boots after that, and never beside another VM" (the fake logs `two VMs running`) |
| Clone stopped and deleted on success, failure and interrupt; one cleanup for both VMs | YES | One `cleanup` per script (stop, wait, delete the clone). Driver cases 1, 2, 4, 5 |
| Success printed only after the check | YES | `Prepared ...` is the script's last line; driver "success is printed only after the check" |
| Failed check: non-zero, names the missing path, deletes the target (and the clone), no success line | YES | `A fresh clone of ... has no working gitlab-runner at <path>. Deleting <target>.`, `exit 1`. Driver case 2 |
| Interrupted run stops the target and keeps it; interrupt during the check must not delete it | YES | `cleanup` never deletes the target; exiting INT/TERM traps. Driver cases 3 and 4 (A warn 1 covered both) |
| A clone whose guest agent never answers is not "binary missing" | YES | Probe `tart exec <clone> /usr/bin/true` after a failed check; only an answering guest leads to deleting the target. Driver case 5 (brief-gate W-1, A warn 1) |
| Arguments, env vars, exit 64 and 69, guest paths unchanged | YES | Removed lines in the two scripts are only the old inline agent wait, the old one-line cleanup and the old `clone`/`run` lines they replace. Driver case 6 pins 64 and 69 |
| Shared helper or self-contained; shape named in handoff-F | YES | Self-contained (A warn 4's default). F's handoff names the choice and the reasons. The `include_str!` test reading the GitLab script's defaults still holds |
| `GITLAB_EXIT_RUNNER_MISSING = 78`, reason `runner_missing`, in core and in the Tart agent's copy, same names and values | YES | `provisioning.rs` and `gridops-tart-agent/src/main.rs`, both tables `[_; 4]`. Documented as `EX_CONFIG`; core's doc says only the Tart script exits with it and that core's own script moves `run-single`'s 78 out of the table (A warn 5) |
| Tables shown equal by the same literals in each crate's tests, no new dev-dependency | YES | `gitlab_exit_reasons_map_each_exit_status_once` (core) and `gitlab_exit_reasons_match_the_core_table` (Tart); no `Cargo.toml` change |
| Collision guard, Tart `runner_script`: `75|77|78) ... exit 70` | YES | `gitlab.rs:163`; test row `("run-single itself exiting 78", "200", "exit-78", 70)` |
| Collision guard, core `GITLAB_RUNNER_SCRIPT`: new placeholder in the `finish 1` list, substituted | YES | `@EXIT_RUNNER_MISSING@` in the case list and in `gitlab_runner_script()`; test `a_runner_exiting_with_a_status_of_the_scripts_own_leaves_the_table` (T showed it fails without the placeholder) |
| Tart listing reports `exitReason` for 78; manager relays it unchanged, no manager code change | YES | `GITLAB_EXIT_REASONS` entry drives the listing; manager diff is a doc comment and a test literal only |
| Reconciler: new `Exit` variant, one shared `left_service` arm, title and reason per exit, counting inside `left_service`, `assert_readable` not loosened, sibling assertion, does not clear failures | YES | `Exit::RunnerMissing`; one failure path that writes `failure_reason`, records the `error` event with `outcome: exit.name()` and calls `count_failure`; `Completed | IdleExpired` clear and `Unknown` does nothing, as before. `assert_runner_missing` sits after `assert_readable`. `from_inspect` untouched |
| Reason visible on the pool as `registration_refused`'s is | YES | Same runner-row `failure_reason` and same pool `error` event paths; evidence.md shows `runner-row.tsx:128` renders `failureReason`. No UI change, as the brief expected |
| Fleet path unchanged; Docker exit 127 out of scope and reported | YES | No `DiagnosticCode`, no `from_inspect` change; F lists the follow-up under Known issues |
| Coexistence with #171/#172 | YES | `gitlab_lifecycle.rs`: only imports, `Exit`, reason helper, `left_service` and the Outcomes doc. No `Setup` helper edited, no #171 helper name reused, the new test sits after `a_refused_registration_counts_once_...`. #171's files untouched. Separate CHANGELOG bullets |
| Driver: one script, fake `tart` via `GRIDOPS_TART_BINARY` recording each call, told the clone has no binary, last line `all passed` | YES | `scripts/test-prepare-tart-runner-images.sh`, POSIX `sh`, runs each script under `/bin/bash` |
| Driver runs in `tart-installer.yml` on macOS with bash 3.2; both prep scripts and the driver in both `paths` lists; mentioned in `docs/macos-runners.md` ~261; not in `gitlab_tests.rs` | YES | Both `paths` lists, new step with the `all passed` last-line rule, `[ -x /bin/bash ]` runner check, header comment and job name updated. `actionlint` clean. Not run on a hosted runner (nothing can be, before a push) |
| Reconciler test: readable `failure_reason`, error event, `provisionFailureCount` +1 counted once across passes, backoff, no reset, later `completed` clears | YES | `a_tart_runner_without_gitlab_runner_counts_once_with_a_readable_reason`. T showed its RED failure was `(1, false)` vs `(2, false)`, the live bug |
| Exit-reason tests (core and Tart map 78; Tart no longer lists 78 as unmapped; listing and manager re-parse) | YES | `exit_statuses_map_to_the_exit_reason_table` (unmapped list now 69, 70, 79, ...), `a_gitlab_runner_gets_its_token_on_stdin_and_lists_its_exit_reason`, `a_runners_exit_reason_survives_the_managers_reparse` |
| fmt, clippy, cargo test for the 4 crates, `bash -n`, `shellcheck` | YES | All run by me, see above; clippy on CI's pinned toolchain |
| Docs and CHANGELOG | YES | `docs/macos-runners.md`: GitLab and Bitbucket sections describe the fresh-clone check; exit table gains 78; the "75 or 77" sentence is now "75, 77 or 78". `docs/gitlab-runners.md`: `runner_missing` bullet and the fresh-clone mention. `CHANGELOG.md`: two #173 bullets. A search for stale "0/75/77", "75 or 77", "exactly three" phrasing finds none outside the handoffs |

Scope: delivered exactly the brief. Over-delivery: none (no UI, no schema, no new dependency, no fleet change). Under-delivery: none. The one outstanding item the brief itself defers, a Docker GitLab pool whose own image lacks `gitlab-runner` (exit 127), is correctly left as a reported follow-up.

## Quality audit
| Area | Result | Notes |
|------|--------|-------|
| API consistency | PASS | `exitReason: "runner_missing"` is one more value of the existing free-string field in `GET /v1/runners`; the manager relays it unchanged (tested) |
| Error handling | PASS | Missing binary, unanswered clone agent, interrupt during install, interrupt during the check, and bad input are each handled and tested for both scripts. A `tart exec` transport failure is not read as a broken image |
| UI/UX match to spec | N/A | No UI change; the existing runner row and pool events carry the reason |
| Accessibility | N/A | No UI |
| Architecture gate | PASS | A: PASS (plan) and PASS (anti-duplication). Nothing in this audit conflicts with either |
| Code organization | PASS | One `Exit` variant and one shared failure path rather than a copied arm; scripts keep the repo's self-contained shape. The roughly 40 duplicated lines between the two bash scripts are the accepted default (A-dup warn, no action) |
| Security | PASS | The check passes the path as an argv element, so nothing is interpolated into a guest script (relevant for Bitbucket's free-form runner root). No token reaches the new reason, event or logs; `assert_runner_missing` asserts that. The scripts still take no token |
| Performance | PASS | The check adds one clone and boot per prep run (the cost the brief asks for). The reconciler change adds no query |
| Visual regression | N/A | No visual surface |
| Naming consistency | PASS | `runner_missing` / `GITLAB_EXIT_RUNNER_MISSING` / `Exit::RunnerMissing` / `runner_missing_reason()` are used the same way in both crates, the docs and the tests; `EX_CONFIG` documented like the 69 doc |
| Test quality (`testing/test-quality.md` section 7) | PASS | See below |

### Test-quality audit
- **Per test.** Each Rust test names one behavior and fails for the right reason when the production change is removed (T's RED run shows the exact assertion for each). The script-driver cases sit at the host-script tier, the cheapest that runs the real scripts under bash 3.2. The reconciler case is at the integration tier because "counted once" comes from `runner_exited`'s guarded UPDATE plus the pass. The manager test is a single added re-parse literal, green by design, and does not pretend otherwise. The tests assert outcomes (VMs that exist, what runs, order of events in one log, exit status, rows and events), not the scripts' internals.
- **Per suite.** Proportionate: one driver whose six cases map one-to-one to the brief's done-when, one new reconciler test, one new core integration test, one new Tart `FAKE_GITLAB_RUNNER` behaviour and a few edited rows/literals in existing tests. No fan-out re-proves one branch.
- **Smells.** No assertion-free, tautological or unreachable-outcome test found. Two non-blocking observations, both in the advisory notes.
- **No "delete or merge this test" findings.**

## Verdict

PASS. Every done-when criterion is met and I reproduced the checks myself: the driver on bash 3.2.57, the four crates' tests, fmt, shellcheck, actionlint and the exact CI clippy command on the CI-pinned toolchain. The code matches the brief, including the collision guards, the table copies, the single `left_service` failure path and the coexistence limits. Ready for O to commit.

## Advisory notes
1. **The driver cannot tell the two defenses against the bash-trap pitfall apart.** In a scratchpad copy of both scripts, replacing the exiting `INT`/`TERM` traps with `trap cleanup EXIT INT TERM` still gives `all passed` (T-red's earlier mutation did fail, against a prototype that had no probe). The behavior the brief and A warn 1 care about, an interrupt during the clone check keeps the target, holds either way: the trap's cleanup deletes the clone, the "agent still answers" probe then fails, and the script exits 1 without deleting the target. I do not call this a finding. The test asserts behavior, and F's exiting traps plus the probe are two independent guards. If a future change drops the probe, case 4 starts to bite. No action.
2. **The reconciler test bundles four groups of assertions** (count and backoff, counted once over repeat passes, a later `completed` clears, the table name) in one integration test. The brief's done-when lists them together and an existing test already covers the `completed` clear, so splitting is optional and not worth a rework cycle.
3. **Real-VM run not done.** The brief forbids real VMs in tests and any deploy, so the `sync` fix is covered by the fake `tart` driver (ordering, cleanup, failure paths) and by #143's live rerun, not by a fresh live run. The first live prep run after merge is the real confirmation.
4. **Tooling note for O (not #173).** This Mac's default toolchain (stable 1.99.1) fails `clippy -D warnings` on five `assert_is_empty` sites and one `unused_async_trait_impl` that CI's pinned 1.98.1 does not. If the default toolchain is meant to match CI, route that separately.
5. **Merge-time heads-up.** #171 and #172 also add `### Fixed`-style bullets to `CHANGELOG.md` and edit `docs/gitlab-runners.md`. Expect a trivial textual conflict there, as the brief anticipated; resolve by keeping each issue's bullets.
