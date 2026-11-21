# Handoff-T-red: Phase 4 - GitLab macOS runners on Tart pools, Tart agent side (#142)

**Date:** 2026-11-20
**Branch:** claude/gitlab-142-tart-13d6d4 (head 229081c)
**Brief / wireframe reviewed:** docs/handoffs/142-brief.md (in full, including the round 1 and 2 verification notes); docs/handoffs/142/handoff-A.md; wireframe N/A (no UI surface)

## A precondition

Confirmed: A returned **PASS** on the plan (`docs/handoffs/142/handoff-A.md`, commit 229081c), with 7 warnings and no blocks. The tests carry out the warnings that bear on testing:

- **Finding 1** (the waiter updates the record in place and never inserts): `deleting_a_gitlab_runner_as_it_exits_does_not_bring_its_record_back`.
- **Finding 2** (`platform: Option<String>`, old records read as non-GitLab, `exit_code: Option<i32>`): `vm_record` sets both fields to `None`. The listing matrix has a `before-142` row, and the old-record load test covers the rest.
- **Finding 3** (tuple-shaped arms that fall through to the CPU and memory checks; the native message names GitLab; a 512-byte URL cap): the `fractional CPU` and `no memory` refusal cases, the `GitLab` check on the native message, and the `over 512 bytes` case.
- **Finding 4** (fix `shell_single_quote` or refuse `'`; F's choice): the `'` URL case accepts either choice.
- **Finding 5** (reuse the fixtures): I extended `FAKE_TART`, `FakeTart` and `vm_record` rather than copying them. `install_script` and `gitlab_request` are now `pub(super)`.

## What F must provide (the API these tests pin)

The crate is one binary, so every test sits in one compilation unit. These names and types are therefore the contract.

| Item | Pinned by | Source of the name |
|---|---|---|
| `mod gitlab;` in `main.rs`; `crates/gridops-tart-agent/src/gitlab.rs` | all `gitlab::tests` | brief Scope 1 |
| In `gitlab.rs`, mount my tests: `#[cfg(test)] #[path = "gitlab_tests.rs"] mod tests;` | all `gitlab::tests` | #137's `fake_tests.rs` precedent |
| `pub(crate) const DEFAULT_GITLAB_RUNNER_BINARY: &str` in `gitlab.rs` | default-binary test | brief W-2 ("e.g.") |
| `pub(crate) fn runner_script(runner_binary: &str, bootstrap: &GitlabBootstrap) -> String` in `gitlab.rs` | guest-script tests | mirrors `bitbucket_runner_script(root, bootstrap)` |
| `pub(crate) fn exit_reason(code: i32) -> Option<&'static str>` in `gitlab.rs` | exit-reason test | brief Scope 1 |
| `AgentState.gitlab_runner_binary: String` | `FakeTart::new` | A finding 6 |
| `TartRecord.platform: Option<String>` and `TartRecord.exit_code: Option<i32>`, both `#[serde(default)]`; written verbatim as `"gitlab"` for GitLab runners | `vm_record`, listing and record tests | A finding 2 |
| GitLab validation reached through `ProvisionRunner::validate` | validation tests | existing seam; `gitlab::validate`'s name is not pinned |
| `scripts/prepare-tart-gitlab-runner-image.sh` exists, contains the default path and `GRIDOPS_TART_GITLAB_RUNNER_BINARY`; `docs/macos-runners.md` contains the default path | `include_str!` in the default-binary test | brief W-2 |

The fakes assume three things about the script, and F must write the script to match:

- **`curl` is called by name** (looked up on `PATH`), so the test's fake is the one that runs. An absolute `/usr/bin/curl` would reach the network.
- **The verify call** (if the script makes one) sends its body on stdin (`--data-binary @-`) and reads the status from `-w/--write-out '%{http_code}'`. The fake also honours `-o/--output` and `--fail`. This is the brief's NIT-2 mechanism. A different mechanism means telling me, so I can extend the fake.
- **The builds directory** is under `$HOME`, passed as `--builds-dir` or `RUNNER_BUILDS_DIR`.

## Tests authored

### `crates/gridops-tart-agent/src/gitlab_tests.rs` (new; unit and process-level; mounted from `gitlab.rs`)

| Test | Criterion / behaviour pinned | Tier, and why |
|---|---|---|
| `a_valid_gitlab_request_is_accepted` | **Launch**: a valid GitLab request passes validation, with `https`, `http`+port and path URLs and a wait timeout of 1. A stray `registrationToken` is ignored, not refused (the brief's documented W-1 deviation). | unit: `validate` is a pure function |
| `each_invalid_gitlab_request_is_refused_without_its_token` | **Launch + Token Debug leak (B-1)**. First, a control: the valid request is accepted. Then 22 single-change cases must each return `BadRequest`: missing, empty, `glpat-`, two-line or `\r` token; no bootstrap; zero timeout; empty, scheme-less or `ftp` URL; URL with a space, newline, tab or control character, or over 512 bytes; `persistent` mode; native runtime; extra `jitConfig`, `bitbucket` or `bitbucketOauthClientSecret` (one case each); fractional CPU; zero memory. In every case, neither the message nor the error's `Debug` contains the token marker. The mode message says `ephemeral`, and the native message names `GitLab`. | unit |
| `exit_statuses_map_to_the_exit_reason_table` | **Exit mapping**: 0, 75 and 77 map to completed, idle_expired and registration_refused. -1, 1, 2, 74, 76, 78, 143 and 255 map to `None`. | unit |
| `the_default_runner_binary_is_a_guest_path_the_image_script_and_docs_name` | **W-2**: the constant passes `valid_guest_path`. The image script contains the constant and `GRIDOPS_TART_GITLAB_RUNNER_BINARY`, and the docs contain the constant. This follows the existing `the_installer_refuses_the_same_waits_as_the_agent` drift-test precedent. | unit |
| `the_guest_script_exits_with_the_status_each_ending_maps_to` | **Script exit codes (B-1, required)**. The real generated script runs under `/bin/bash -c` (3.2 on macOS, the guest's shell) with the token on stdin, a cleared environment, fakes first on `PATH` and the runner binary pointing at the fake. The exact status is asserted for: a passed job and a failed job (0), idle (75), a runner crash (its own 3), a token refused by `run-single` (77 if the script recognises the "forbidden" output, otherwise 75, which is the brief's W-1 gap), and a token refused by verify (77; only when the script implements verify). It also asserts that at least one refusal mechanism exists. In every case where the runner runs: argv starts with `run-single`; `--url`, `--executor shell`, `--max-builds 1` and `--wait-timeout 37` (not the default) are all present; the token is in no argv; the token is in the environment **only** as `CI_SERVER_TOKEN`; and the builds directory is under `$HOME`. The verify call goes to `<url>/api/v4/runners/verify` with the token on curl's stdin and not in its argv. No token appears in the output. | process: the behaviour only exists when bash runs the script. No Tart, VM or network is involved. |
| `a_guest_without_gitlab_runner_exits_78_and_says_where_it_looked` | **Script exit codes**: a missing binary exits 78, the message names the path, and the runner never runs. | process |
| `hostile_urls_reach_gitlab_runner_verbatim_and_run_nothing` | **Launch, hostile values**. Five URLs, each containing one or more of `$(...)`, backticks, `;`, `"`, `\`, `&&`, `\|\|`, `\|`, `>`, `$HOME`, `${PATH}` or `*`, go straight into `runner_script`. Each must reach `--url` verbatim, exit 0, and create no `pwned*` file. A `'` URL must be either refused (`BadRequest`, no token) or accepted and carried verbatim through bash (A finding 4, F's choice). | process |

### `crates/gridops-tart-agent/src/main.rs` (tests module, through the real handlers with `FakeTart`)

| Test | Criterion / behaviour pinned | Tier, and why |
|---|---|---|
| `a_gitlab_runner_gets_its_token_on_stdin_and_lists_its_exit_reason` | **Token delivery + exit capture + listing**, end to end through `provision_runner`, the waiter and `list_runners`. For exit statuses 77, 0 and 78: `tart exec -i`'s stdin is exactly the token plus `\n`. No `tart` command line contains the token, and the exec runs `/bin/bash -lc` with `run-single` and the configured binary. The record file **at the moment of the VM stop** already holds the exit code, which is the "written before stop" invariant. After a reload (an agent restart) the record keeps `exit_code` and `platform == "gitlab"`. The listing shows `exited` with exactly `exitReason` registration_refused or completed, and **no key** for 78. The listing never contains the token. | integration: the record, stdin and argv are only observable through the handlers. A fake `tart` stands in for Tart. |
| `only_exited_gitlab_runners_with_a_known_code_list_an_exit_reason` | **Exit mapping, listing matrix**: the listing has the key (with the right value) only for exited GitLab runners with code 0, 75 or 77. There is no key (as opposed to `null`) for: GitLab code 1, GitLab with no code, a **running** GitLab runner with code 0, `github`, `bitbucket`, or a record with no platform. | integration: needs `list_runners` with a Tart listing |
| `github_and_bitbucket_runners_never_list_an_exit_reason` | **Exit mapping**: "the values the implementation actually writes" for GitHub and Bitbucket. Real GitHub and Bitbucket provisioning, exec exit 0, then VM stopped: no `exitReason`. | integration. **Guard, green by design before #142** (see below). |
| `records_keep_their_exit_code_and_records_from_before_142_still_load` | **Exit mapping, persistence**: a record file in the pre-#142 shape loads with `platform`/`exit_code` `None`, and a record persisted with code 75 and `gitlab` reloads with both. | integration: real record files |
| `deleting_a_gitlab_runner_as_it_exits_does_not_bring_its_record_back` | **A finding 1**: the fake's exec ends only after `tart delete`, so the exit code arrives after the delete removed the record. Afterwards no record exists in memory or on disk. | integration: needs the real waiter racing the real `delete_runner` |
| `a_failed_gitlab_start_never_reports_its_token` | **B-1, "no error returned by the GitLab arm"**: a GitLab start whose `tart run` fails returns `Internal`. Neither its `{:?}` (what `tracing::error!` logs) nor its `{:#}` contains the token, and the clone is released. | integration |

**Changed fixtures (test code only):**

- `FAKE_TART`'s `exec -i` branch:
  - saves stdin to `stdin`;
  - if `exec-until-deleted` exists, it waits until the fake `delete` has run;
  - it exits with the status in `exec-status`, or 0 if that file is absent.
- The readiness probe `tart exec <vm> /usr/bin/true` has no `-i`, so the probe is unaffected.
- `stop` appends the record files to `records-at-stop`, then logs `stops-done`. `delete` touches `deleted`.
- `FakeTart` sets `gitlab_runner_binary` to the non-default `/opt/gridops-test/bin/gitlab-runner`.
- `vm_record` gains `platform: None` and `exit_code: None`.
- **Removed:** `gitlab_runners_are_refused_by_name_until_142_lands`. The brief says the placeholder and its test are replaced by tests of the real validation.
- **Kept unchanged:** `gitlab_exit_reasons_match_the_core_table` and `a_gitlab_request_carries_its_bootstrap_and_token`.

**How the fakes model `gitlab-runner`.** I read upstream `gitlab-runner` at commit `e2320620bae7319498c63f7b2b05e52afe99393e`; `evidence.md` has the excerpts.

- **Exit status.** `run-single` exits 0 after its one build, whatever the job's result, and also exits 0 after the wait timeout.
- **What it prints.** After a build it prints "This runner has processed its build limit, so now exiting". After the timeout it prints "This runner has not received a job in N seconds, so now exiting".
- **A refused token.** It logs `Checking for jobs... forbidden`, then waits out the timeout and exits 0.
- **Configuration.** It logs to stderr. It takes the token from `CI_SERVER_TOKEN` and the builds directory from `--builds-dir` or `RUNNER_BUILDS_DIR`.

This settles the part of **NV-2** about the token's environment variable and the exit behaviour. F still owns citing the evidence for NV-1 and NV-3 in its handoff.

## RED confirmation

### 1. The real RED: compile errors that name only missing contract items

`cargo test -p gridops-tart-agent --no-run` on this branch, with only my test changes:

    error[E0560]: struct `AgentState` has no field named `gitlab_runner_binary`
    error[E0560]: struct `TartRecord` has no field named `exit_code`
    error[E0560]: struct `TartRecord` has no field named `platform`
    error[E0609]: no field `exit_code` on type `&TartRecord`   (x3)
    error[E0609]: no field `exit_code` on type `TartRecord`    (x3)
    error[E0609]: no field `platform` on type `&TartRecord`    (x3)
    error[E0609]: no field `platform` on type `TartRecord`     (x2)
    error: could not compile `gridops-tart-agent` (bin "gridops-tart-agent" test) due to 14 previous errors

Every error is one of the three new fields. None is a typo, a bad import or setup. `gitlab_tests.rs` compiles only once F creates `gitlab.rs` and mounts it. Until then the crate's existing tests cannot run either, which is inherent to RED over new types in a single binary (same as #137).

### 2. Runtime RED: each test fails on the missing behaviour

To prove the tests fail for the right reason, and not just fail to compile, I applied a **throwaway stub scaffold**. It added the three fields and `mod gitlab;`, plus a `gitlab.rs` with these stubs:

- `DEFAULT_GITLAB_RUNNER_BINARY = ""`;
- `runner_script` → `"exit 1"`;
- `exit_reason` → `None`;
- the test mount;

and an empty image script. Then I ran `cargo test -p gridops-tart-agent`: **11 failed / 40 passed**. Every existing test still passes.

    a_valid_gitlab_request_is_accepted: ... Err(BadRequest("GitLab runners are not supported yet: Tart is #142."))
    each_invalid_gitlab_request_is_refused_without_its_token: the valid request was refused: Err(BadRequest("GitLab runners are not supported yet: Tart is #142."))
    exit_statuses_map_to_the_exit_reason_table: left: None  right: Some("completed")
    the_default_runner_binary_is_a_guest_path_the_image_script_and_docs_name: panicked at gitlab_tests.rs (valid_guest_path(DEFAULT_GITLAB_RUNNER_BINARY))
    the_guest_script_exits_with_the_status_each_ending_maps_to: the script has no way to detect a refused token
    a_guest_without_gitlab_runner_exits_78_and_says_where_it_looked: left: Some(1)  right: Some(78)
    hostile_urls_reach_gitlab_runner_verbatim_and_run_nothing: https://gitlab.example.com/$(touch${IFS}pwned-substitution): left: Some(1)  right: Some(0)
    a_gitlab_runner_gets_its_token_on_stdin_and_lists_its_exit_reason: expected the runner to start, got Err(BadRequest("GitLab runners are not supported yet: Tart is #142."))
    deleting_a_gitlab_runner_as_it_exits_does_not_bring_its_record_back: expected the runner to start, got Err(BadRequest("GitLab runners are not supported yet: Tart is #142."))
    a_failed_gitlab_start_never_reports_its_token: expected an ordinary start failure, got Err(BadRequest("GitLab runners are not supported yet: Tart is #142."))
    only_exited_gitlab_runners_with_a_known_code_list_an_exit_reason: gitlab-completed: left: None  right: Some(String("completed"))

Two tests pass on the stub, for documented reasons:

- `github_and_bitbucket_runners_never_list_an_exit_reason` is a **guard, green by design**. No `exitReason` exists before #142. It is not vacuous: removing the platform check from the listing makes it fail (mutation m2 below).
- `records_keep_their_exit_code_and_records_from_before_142_still_load` passes as soon as the two `#[serde(default)]` fields exist, so its RED is the compile RED. It fails if F drops `#[serde(default)]`, because the old record then fails to load.

### 3. The suite can go GREEN, and it catches wrong implementations

I wrote a **throwaway draft implementation**. Its script verifies the token with `curl` and recognises run-single's "forbidden" output. It also includes the in-place waiter, the listing filter and the `'\''` quoting fix. Against it, `cargo test -p gridops-tart-agent` gave **51 passed** on 9 consecutive runs. `cargo fmt --check` was clean, and `cargo clippy --all-targets -- -D warnings -A clippy::unused_async_trait_impl` reported nothing in my test code. A **verify-only** script variant also passes, through the W-1 branch.

Each mutation of the draft fails exactly the intended tests:

| Mutation | Fails |
|---|---|
| m1: the waiter persists and `insert`s the record unconditionally | the delete-race test |
| m2: no platform check in the listing | the matrix and the GitHub/Bitbucket guard |
| m3: no `exited` check in the listing | the matrix |
| m4: `--token "$secret"` on argv | the script and hostile-URL tests |
| m5: the secret `export`ed | the script and hostile-URL tests (the token is in another environment variable) |
| m6: idle not mapped to 75 | the script test |
| m7: verify 403 ignored | the script test |
| m8: the old `shell_single_quote` | the hostile-URL test (the `'` case) |
| m9: URL interpolated in double quotes, unquoted | the hostile-URL test |
| m10: token `Debug`-formatted into a `BadRequest` | the refusal test |
| m11: no check for extra `bitbucket`/secret | the refusal test |
| m12: `return gitlab::validate(..)` skipping the CPU/memory checks | the refusal test |
| m13: `--wait-timeout 600` hard-coded | the script and hostile-URL tests |
| m14: a runner crash forced to 75 | the script test |
| m15: the VM stopped before the exit code is written | the token/exit end-to-end test |

All scaffold and draft files were removed afterwards:

- `git status` shows only `M crates/gridops-tart-agent/src/main.rs` and `?? crates/gridops-tart-agent/src/gitlab_tests.rs`.
- Every `main.rs` hunk is inside `mod tests`.
- `docs/macos-runners.md` is byte-identical to HEAD.

## Ready for F

**RED is valid. F may implement against these tests.** The files to stage are:

- `crates/gridops-tart-agent/src/main.rs` (tests-module changes only);
- `crates/gridops-tart-agent/src/gitlab_tests.rs`;
- `docs/handoffs/142/handoff-T-red.md`, `docs/handoffs/142/evidence.md` and `docs/handoffs/142/decisions.md`.

**Notes for F:**

1. **Lint baseline.** Local clippy 0.1.99 (2026-10-13) adds `assert_is_empty`, which flags 3 **pre-existing** `assert!(x.is_empty())` lines in master's tests module (`waits`, `slot_waiters`). CI also passes `-A clippy::unused_async_trait_impl` for an unrelated lint. These are not from #142. Whether the Done-when clippy gate needs them fixed depends on CI's toolchain; check it before Phase 6.
2. **An existing test flaked once.** `every_tart_command_gets_a_path_starting_with_tarts_directory` failed once in about 20 full-suite runs during my checks, at its PATH assertion. It passed in isolation and in the 9 later full runs, so I could not reproduce it. It may come from concurrent appends to the fake's `paths` file. I'll watch for it in Phase 6.
