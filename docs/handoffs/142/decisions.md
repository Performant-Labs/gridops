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
