# Decision journal: #142 GitLab macOS runners on Tart pools (Tart agent side)

## A (Phase 3, up-front plan review): 2026-11-20T15:33:21-08:00
- **Decided:** PASS, with 0 block findings and 7 warn findings (see `handoff-A.md`).
  - **Accepted as consistent with the code:**
    - the new `gitlab.rs` sibling module, which matches the manager's `vm_slots.rs`/`sandbox.rs` and the reconciler's `tart_lane.rs`;
    - the `("gitlab", _)` arm in `validate` and the `"gitlab"` arm in `provision_runner`, with `.take()` plus `SecretString::from` into the shared `start_runner` stdin channel;
    - the `#[serde(default)]` additions to `TartRecord`;
    - the conditional `exitReason` in `list_runners`;
    - `GRIDOPS_TART_GITLAB_RUNNER_BINARY`, configured like `GRIDOPS_TART_BITBUCKET_RUNNER_ROOT`;
    - `scripts/prepare-tart-gitlab-runner-image.sh`, modelled on the Bitbucket script, with the checksum step following `prepare-tart-runner-image.sh`'s `shasum -a 256 --check`;
    - the docs section next to "Bitbucket Cloud workspaces".
  - **The warnings:**
    - (1) The exit-code writer in `start_runner`'s waiter must update in place, as `start_native_runner`'s waiter does, and never insert. Otherwise a `delete_runner` that races the exiting `tart exec` brings back a deleted record.
    - (2) The new `platform` field must not default to `"github"`, because old Bitbucket records exist. Use `Option<String>`, like `root`.
    - (3) Shape the `validate` arms like GitHub's tuple arms and fall through to the CPU and memory checks. Extend the existing native-runtime message rather than adding a duplicate, unreachable runtime check. Cap the URL at 512.
    - (4) Fix `shell_single_quote` in place rather than adding a GitLab-only quoting helper. The bug is already reachable through the Bitbucket validation.
    - (5) Reuse `FAKE_TART`/`FakeTart`, `install_script` and `gitlab_request`.
    - (6) Module conventions: a `//!` header, explicit `use crate::...`, `pub(crate)`, a local `GITLAB_RUNNER_TOKEN_PREFIX` copy, and no installer or plist entry (matching Bitbucket).
    - (7) #142 is not reachable end to end until #147 lets Tart GitLab requests through the manager's `validate` and adds the D15 relay.
- **Assumed:**
  - **Agent restarts:** a waiter task does not outlive an agent restart (spawned tasks die with the process), so old records are not rewritten by a waiter after a restart.
  - **Ownership:** this issue owns `crates/gridops-tart-agent` whole, per epic #136's table as restated in the brief. So editing `shell_single_quote`, `FAKE_TART` and `install_script`'s visibility is a planned extension, not a drive-by of another agent's code.
- **Hedged:**
  - **Runtime behaviour, left to T and F.** It is not settled whether `tart exec` returns the guest status (NV-1) after an explicit `tart stop` (from `control_runner` or `delete_runner`). If it returns 0, a stopped GitLab runner could list `exitReason: "completed"`. I did not classify this as drift.
  - **Two choices left to F,** each acceptable if written down in the handoff:
    - Finding 1: record the exit code for every VM runner, or only for GitLab ones chosen by the caller.
    - Finding 4: fix the quoting helper, or refuse `'` in the URL.
  - **No Reuse map.** No `survey.md` was provided, so the brief's named analogues were the baseline.
- **Evidence:**
  - **Read:**
    - the brief, and brief-review results r1 to r3, including the uncommitted r1 diff;
    - `docs/handoffs/137/handoff-A.md` and `docs/handoffs/137/decisions.md`, for format and precedent;
    - `docs/handoffs/137-brief.md`, grepped for the R2-1 and D10/D15 ownership rows.
  - **Source:**
    - `crates/gridops-tart-agent/src/main.rs`: `AgentState`, `TartRecord`, `ProvisionRunner`, `GitlabBootstrap`, `GITLAB_EXIT_*`, `validate`, `main()` configuration, `list_runners`, `provision_runner`, `provision_native`, `start_native_runner`, `control_runner`, `delete_runner`, `start_runner`, `bitbucket_runner_script`, `shell_single_quote`, `runner_script`, the record helpers, `valid_guest_path`, and in the tests module `provision_request`, `FAKE_TART`, `install_script`, `FakeTart`, `vm_record` and the GitLab tests;
    - `crates/gridops-tart-agent/Cargo.toml`;
    - `crates/gridops-manager/src/main.rs` (module declarations, `validate`, `valid_registration_url`) and `crates/gridops-manager/src/vm_slots.rs` (module header and imports);
    - `crates/gridops-core/src/provisioning.rs`, the GitLab constants.
  - **Scripts and docs:** `scripts/prepare-tart-bitbucket-runner-image.sh` (in full) and `scripts/prepare-tart-runner-image.sh` (checksum lines); `docs/macos-runners.md` (section list and "Bitbucket Cloud workspaces").
  - **Searched:** `GRIDOPS_TART_*_RUNNER_ROOT` references across `scripts/`, `docs/`, `deploy/` and `crates/`.

## T (Phase 4, author tests / RED): 2026-11-20T16:00:53-08:00
- **Decided:**
  - **RED is valid.** 13 new tests: 7 in a new `crates/gridops-tart-agent/src/gitlab_tests.rs`, 6 in `main.rs`'s tests module. The placeholder test `gitlab_runners_are_refused_by_name_until_142_lands` is removed, as the brief directs.
  - **Real RED is a compile RED.** It has 14 errors, and each names a missing field: `AgentState.gitlab_runner_binary`, `TartRecord.platform` or `TartRecord.exit_code`.
  - **Runtime RED was shown on a throwaway stub scaffold:** 11 failures, each on the missing behaviour.
  - **Two tests are not RED, by design.** The GitHub/Bitbucket no-`exitReason` test is a guard. The old-record test passes once the `#[serde(default)]` fields exist.
  - **Pinned API, the smallest the tests need:**
    - `gitlab::runner_script(runner_binary, &GitlabBootstrap) -> String`, mirroring `bitbucket_runner_script`;
    - `gitlab::exit_reason(i32) -> Option<&'static str>`;
    - `gitlab::DEFAULT_GITLAB_RUNNER_BINARY`;
    - `AgentState.gitlab_runner_binary` and `TartRecord.{platform: Option<String>, exit_code: Option<i32>}`, per A findings 2 and 6.
    - Validation is tested through `ProvisionRunner::validate`, so `gitlab::validate`'s name is F's choice.
  - **Test placement follows A finding 5.** The pure and bash tests sit beside the module, in `gitlab_tests.rs`, mounted with `#[path]` as in #137's `fake_tests.rs`, so F's `gitlab.rs` holds no test code. Tests that need the handlers stay in `main.rs` and extend `FAKE_TART`, `FakeTart` and `vm_record`.
  - **The fake `gitlab-runner` models upstream behaviour** (`run-single` exits 0 after a job and after the idle timeout; a refused token logs "forbidden" and idles), cited in `evidence.md`. A fake that exited 75 or 77 itself would have proved nothing about the script's detection.
  - **Refused-token cases depend on what the script implements.** The guest test checks whether the script text contains `runners/verify` and/or `forbidden`, and gives each implemented mechanism its own case. A verify-only script must map a token revoked mid-run to 75, the brief's accepted W-1 gap. The brief allows either design, so the test does not force one.
  - **The `'` URL case accepts both choices,** refused or carried verbatim (A finding 4 leaves it to F). Other hostile URLs go straight into `runner_script`, so the script's quoting is tested whatever validation does.
- **Assumed:**
  - The script calls `curl` by name on `PATH`.
  - The verify call uses `--data-binary @-` with `-w '%{http_code}'` (the brief's NIT-2 mechanism). The fake also honours `-o` and `--fail`.
  - The builds directory is passed as `--builds-dir` or `RUNNER_BUILDS_DIR`, under `$HOME`.
  - If F needs another verify mechanism, T extends the fake.
- **Hedged:**
  - **Clippy baseline.** Local clippy 0.1.99 flags 3 pre-existing `assert!(x.is_empty())` lines (`assert_is_empty`) in master's tests. The Done-when clippy gate may need them fixed, depending on CI's toolchain; flagged to F.
  - **A possible pre-existing flake.** `every_tart_command_gets_a_path_starting_with_tarts_directory` failed once in about 20 full runs and could not be reproduced. To be watched in Phase 6.
  - **Not tested.** The image script's pinned version and checksum, and the docs prose. Neither is a Done-when test item; T checks them in Phase 6 Tier 1 (`bash -n`, review).
- **Evidence:**
  - **Read:** the brief in full; `handoff-A.md`; the A entry of `decisions.md`; `docs/handoffs/137/handoff-T-red.md`, for format and the compile-RED precedent.
  - **Source:** `crates/gridops-tart-agent/src/main.rs`: `AgentState`, `TartRecord`, `ProvisionRunner`, `validate`, `list_runners`, `provision_runner`, `provision_native`, `start_native_runner`, `delete_runner`, `start_runner`, the script builders, `shell_single_quote`, the record helpers, and the tests module (`FAKE_TART`, `install_script`, `FakeTart`, `vm_record`, `gitlab_request` and the GitLab tests).
  - **Upstream:** `gitlab-runner` at `e2320620bae7319498c63f7b2b05e52afe99393e`: `commands/single.go`, `network/gitlab.go`, `common/config.go`, and `log/` for the stderr output.
  - **Runs:**
    - compile RED, and the stub-scaffold runtime RED (11 failed / 40 passed);
    - a throwaway draft: 51 passed on 9 consecutive runs, `cargo fmt --check` clean, and clippy clean in the test code;
    - 15 mutations, each failing exactly the intended tests;
    - every scaffold and draft file removed afterwards: `git status` shows only the two test files plus the handoffs.

## F (Phase 5, implementation / GREEN): 2026-11-20T16:30:26-08:00
- **Decided:**
  - **GREEN:** all 51 tests in `cargo test -p gridops-tart-agent` pass. `cargo fmt --check` is clean. Clippy on CI's pinned 1.98.1 toolchain, with CI's flags, is clean. The 3 `assert_is_empty` lines T flagged belong to the newer local clippy, not CI's.
  - **Default binary:** `/Users/admin/gitlab-runner/bin/gitlab-runner`, defined once as `gitlab::DEFAULT_GITLAB_RUNNER_BINARY`. The image script and the docs use the same literal.
  - **Refused-token detection uses both mechanisms:**
    - the verify call (`printf` builtin → `curl --data-binary @-`, `-w '%{http_code}'`): 200 continues, 403 exits 77, anything else exits 69;
    - `run-single`'s output: a job received or the build limit reached exits 0, otherwise "forbidden" exits 77, otherwise the wait-timeout line exits 75, otherwise 70.
    - This leaves no W-1 time-of-check gap. If the runner itself exits 75 or 77, the script exits 70, so 0, 75 and 77 come only from the three named situations.
  - **A finding 4:** `shell_single_quote` is fixed in place (`'\''`), and `'` in the URL is accepted. `gitlab.rs` has no quoting helper.
  - **A finding 1:**
    - the exit code is recorded for every VM runner, so `start_runner` stays platform-agnostic;
    - `record_exit_code` updates through `get_mut` under `records.write()` and persists while holding the lock, before `tart stop`;
    - it never inserts.
  - **A finding 2:** `platform: Option<String>` is written verbatim, and `exit_code: Option<i32>`. Both are `#[serde(default)]`.
  - **A finding 3:** tuple arms, then a fall-through to the CPU and memory checks. The native message names GitLab. The URL is capped at 512 bytes and checked by hand.
  - **A finding 6:**
    - `//!` header, explicit `use crate::{...}`, `pub(crate)`;
    - a local `GITLAB_RUNNER_TOKEN_PREFIX`;
    - no installer or plist entry.
  - **Token validation:** printable ASCII, excluding `"` and `\`, from 6 to 1024 bytes, so it can go into the JSON body unescaped.
  - **Pinned release:** gitlab-runner **19.5.3**, with each macOS SHA-256 from GitLab's `release.sha256`. Both binaries were downloaded, and their checksums match.
- **Assumed:**
  - The guest's `/usr/bin/curl`, `tee` and `grep` are on the login shell's PATH. They are stock macOS.
  - A pre-#142 record whose VM exited before the upgrade has no code and lists no `exitReason`, which is correct.
- **Hedged:**
  - **A mid-`run-single` refusal surfaces after about 300 s.** That is upstream's `NotHealthyCheckInterval`, or the wait timeout if that is longer. The script exits 77 then, but not sooner.
  - **One path was checked only against T's fake:** the "job received" → 0 path. A real-binary check would need a full job payload.
  - **Two pre-existing full-suite flakes:** `a_runner_waiting_for_a_slot_is_listed_as_waiting` and `every_tart_command_gets_a_path_starting_with_tarts_directory`. They reproduce at 2/15 on the untouched base `229081c`, and were 2/24 on this branch. Flagged for T and left untouched, because it is test code.
  - **The open A hedge on NV-1:** whether `tart exec` returns the guest's status after an explicit `tart stop` is still not settled. If the stopped guest's exit event reports 0, a GitLab runner stopped from outside could list "completed".
- **Evidence:**
  - **Upstream, read for NV-1/2/3:**
    - `cirruslabs/tart` 2.41.1 (`Exec.swift:195-196`, `Root.swift:164-168`);
    - gitlab-runner v19.5.3 (`network/gitlab.go:451-466, 690, 701-702`; `network/client.go:636`; `commands/single.go:24, 27, 87-89, 168, 172`; `common/config.go:1417, 1421, 1456-1457`; `common/consts.go:14`).
    - The real 19.5.3 binary's `run-single --help` confirmed the flags and `$CI_SERVER_TOKEN`.
  - **Runs:**
    - the rendered guest script with the real 19.5.3 binary and real curl against a stub GitLab: verify 403 exited 77, idle exited 75, mid-run 403 exited 77, and no token appeared in any output or argv;
    - the image script with a fake `tart` (a real download plus `shasum --check`);
    - 24 full test runs, and 15 baseline runs.

## T (Phase 7, GREEN + Tier 2)

- **Decided:**
  - GREEN is accepted: `gridops-tart-agent` 51/51, three consecutive runs.
  - The two workspace failures are environmental and not F's: manager `SUN_LEN` (long worktree `TMPDIR`; 105/105 with a short one) and the api `release_preflight` test, which fails because this host is itself a macOS arm64 machine. Neither crate is in the diff.
  - No test was repaired, and no production code was changed.
- **Assumed:** CI uses short temp paths and a non-macOS-arm64 fleet fixture, so these two do not fail there.
- **Hedged:** the two flaky pre-existing fixtures F named did not reproduce in my runs, so they are left as advisory.
- **Evidence:** the commands and counts are in `handoff-T-green.md`.

## A Phase 7 (anti-duplication gate), 2026-11-20
Verdict PASS: gitlab.rs and the image script are brief-mandated; all other changes extend existing objects in place (see handoff-A-dup.md).

## F (Phase 6 rework, S REWORK), 2026-11-20

- **Decided:** S's blocking finding is test-only (`gitlab_tests.rs` self-deriving expectations plus missing rows). F makes no production-code change for it and hands it to T. F also fixed one advisory doc inaccuracy: the exit-70 description in `docs/macos-runners.md` now covers `run-single` itself exiting 75 or 77.
- **Assumed:** the workflow routes a self-reported test-only S REWORK to T (playbook #678).
- **Hedged:** S's other advisory notes are PR-body items for O: the manual check deferred to #143, the registrationToken deviation (ignored, not refused), #147 being needed for end-to-end reachability, and the crash window. They are also contract-level items for epic #136 (instance-prefixed `glrt-` tokens) or upstream behaviour (the 300 s mid-run refusal delay, gitlab-runner's shortened-token log line). None is a #142 production change.
- **Evidence:** `handoff-S.md`; `gitlab.rs:144-178` (the exit mapping the new rows pin).

## A (Phase 3, up-front plan review, second pass): 2026-11-20T17:05:02-08:00
- **Decided:**
  - **PASS: no blocks, and 8 warnings** (see `handoff-A.md`, which this pass rewrites).
  - **The brief is unchanged since the first pass** (last amended in 12421a7, before 229081c).
  - **The baseline code is unchanged** in every file #142 touches. `origin/master`'s three new commits (#146, #145, #155) don't touch `gridops-tart-agent`, the manager's `validate`, `provisioning.rs`, `scripts/` or `docs/macos-runners.md`.
  - **Warnings 1 to 7 from the first pass stand.** The implementation already on the branch (HEAD 6d5e2e1) follows all seven.
  - **New warning 8: code already exists**, although the task text says "before any code exists". T and F in this run must extend the existing `gitlab.rs`, `gitlab_tests.rs`, the `main.rs` wiring, the image script and the docs section, not write them again. A second copy would be a parallel path, and the Phase 7 gate would block it.
- **Assumed:** this is a fresh workflow run, restarted at Phase 3 after the previous run stopped on F's "implementation incomplete" (0f08494), which came after S's test-only REWORK. T's rework commit 6d5e2e1 landed after that.
- **Hedged:**
  - **NV-1 is still open.** It is unsettled whether `tart exec` returns the guest's status after an outside `tart stop` (from `control_runner` or `delete_runner`). This is a runtime question, not drift. The existing script lowers the risk: it never exits 0 unless it recognised a line from `run-single`.
  - **The image scripts use two styles.** The GitLab image script runs its guest step as a stdin heredoc, as `prepare-tart-runner-image.sh` does, not as Bitbucket's `bash -lc` string. The repository uses both, so I raised no finding.
- **Evidence:**
  - **Read:**
    - the brief in full, `git log` of the brief, and `handoff-A.md` and `decisions.md` from the first pass;
    - `handoff-S.md`, `142-diff-result-r1.md`, F's rework section and T's rework section;
    - the uncommitted diff of `142-brief-result-r1`.
  - **Source at b22c451:** `crates/gridops-tart-agent/src/main.rs`: `TartRecord`, `ProvisionRunner`, `GitlabBootstrap`, `GITLAB_EXIT_*`, `validate`, `list_runners`, `provision_runner`, the native waiter, `control_runner`, `delete_runner`, `start_runner`, `bitbucket_runner_script`, `shell_single_quote`, the record helpers and `valid_guest_path`.
  - **Source at HEAD:** `gitlab.rs` in full, the production diff of `main.rs`, the index of `gitlab_tests.rs`, the diff between the image script and Bitbucket's, and the section list of `docs/macos-runners.md`.
  - **Source on `origin/master`:** `gridops-manager/src/main.rs:487`, `gridops-core/src/gitlab/{mod,http}.rs`, and `git diff --stat HEAD...origin/master`.

## T (Phase 4, author tests / RED, second run): 2026-11-20T17:15:45-08:00
- **Decided:**
  - **The existing suite was extended in place, not re-authored** (A finding 8). The changes:
    - +2 token refusal rows (`"` and `\`, the verify body's JSON precondition);
    - +1 script row (a job received with no build-limit line → 0);
    - the hostile-URL `'` case now has a fixed expectation instead of a conditional one (a sibling of S's finding 1);
    - +1 waiter row (a `tart exec` killed by a signal records no code and lists no `exitReason`);
    - one new test (a failed exit-code write still stops the VM, A finding 1).
  - **RED is valid, proved by removal.** I made 7 production mutations, one per new or changed behaviour, plus 4 re-checks of the existing listing and waiter contracts on F's own code. Each fails exactly the intended test, with the intended assertion. HEAD with the new tests is 52/52 GREEN, so F has no production change to make.
  - **S's optional item 7 (trimming the refusal table) is not done.** The rows are distinct inputs at the behaviour level, so they are not redundant.
- **Assumed:** this second run's RED contract is the full existing suite plus these additions. The original fails-before-code RED against `b22c451` stands from the first run (handoff-T-red.md at 48c5c0e).
- **Hedged:** NV-1 (what `tart exec` returns after an outside `tart stop`) is still a runtime question that no fake can settle. The `killed` row pins only the agent's side: no status means no recorded code.
- **Evidence:**
  - `handoff-T-red.md`: the mutation tables;
  - `evidence.md`, "T (Phase 4, RED, second run)": `persist_record`'s `.json.tmp`, the waiter's `code()` guard, and the `std` doc on signal exits;
  - the test runs: `cargo test -p gridops-tart-agent` 52/52 on 4 runs, `cargo fmt --check` clean, and CI's 1.98.1 clippy line clean.

## F (Phase 5, implement against RED, second run): 2026-11-20T17:18:44-08:00
- **Decided:** I made no production change. Every behaviour T's second-run additions pin is already in the existing implementation:
  - `gitlab.rs:94-101` refuses `"` and `\`.
  - `gitlab.rs:167` matches the "received" line.
  - `main.rs:1899-1901` quotes with `'\''`.
  - `main.rs:1843` records a code only when `code()` returns one.
  - `main.rs:1861-1870` logs a persist failure and carries on, so `tart stop` still runs.

  Following A's finding 8, nothing was rewritten.
- **Assumed:** T's mutation table is a valid RED for this run, given that the code already exists (handoff-T-red.md).
- **Hedged:** none. NV-1 and the 300 s mid-run refusal delay are carried over unchanged as known issues.
- **Evidence:** `cargo test -p gridops-tart-agent` passed 52/52 on 3 consecutive runs. `cargo fmt --check` is clean, CI's 1.98.1 clippy line is clean, and `bash -n` and shellcheck pass on the image script.
