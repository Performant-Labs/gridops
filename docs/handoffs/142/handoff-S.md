# Handoff-S: Phase 8 - GitLab macOS runners on Tart pools, Tart agent side (#142), second audit

**Date:** 2026-11-20
**Branch:** claude/gitlab-142-tart-13d6d4 (head 1c45f91)
**Issue:** #142
**Handoff-T reviewed:** docs/handoffs/142/handoff-T-green.md (and handoff-T-red.md)
**Handoff-A reviewed:** docs/handoffs/142/handoff-A.md (plan, second pass: PASS) and docs/handoffs/142/handoff-A-dup.md (anti-duplication: PASS)
**Handoff-F reviewed:** docs/handoffs/142/handoff-F.md (no production change in this run)
**Spec:** docs/handoffs/142-brief.md (the issue text, the #137 deviations D9, D10, D12, D15, and the review round 1-3 amendments; where they differ, the amendments win, as the brief says)
**Previous S verdict (this file at 8f6c066):** REWORK, test-only. Closed by 6d5e2e1 and re-checked below.
**Operator-facing report:** not applicable (no visual surface)

## A precondition
Confirmed. A returned PASS on the plan (second pass, eight warnings, no blocks) and PASS on the anti-duplication gate (no duplication, extension is clean).

## T precondition
Confirmed. T-green reports "Blocking issues: None". Its two workspace failures are environmental and in code this branch does not touch (the `gridops-manager` `SUN_LEN` socket-path tests from the long worktree path, and one macOS-arm64-host `release_preflight` test in `gridops-api`).

## Visual-diff-tool precondition
Not applicable. #142 is a Rust crate change, a shell script and a docs section. It has no rendered surface, so the browser, Playwright and ImageMagick preconditions, the pixel diffs and the HTML report do not apply.

## Preview / spec sanity check
Done on the brief before auditing. No source-of-truth defect that justifies ADVISORY-HOLD. Two upstream facts for O, neither F's to fix (Advisory notes 5 and 6): gitlab-runner accepts an instance-prefixed token that the contract's `glrt-` check would refuse, and the "never in a log" sentence in the docs is a little stronger than gitlab-runner's own log allows.

## Visual diff results
Not applicable.

## What changed since my previous audit
- Production code (`gitlab.rs`, `main.rs` outside `mod tests`, the image script) is identical to what I audited at f6f4610. `git diff 6d5e2e1 HEAD` touches only the `mod tests` block of `main.rs` (every hunk is at line 2923 or later; `mod tests` starts at 2386) and `gitlab_tests.rs`.
- The one docs change is the `70` clause I asked for (the `run-single` that itself exits 75 or 77).
- 6d5e2e1 delivered my test-only REWORK, and T-red then added 1 test and 4 rows (52 tests now, 51 before).

## Independent verification I ran (not only T's and F's reports)

| Check | Result |
|---|---|
| `cargo test -p gridops-tart-agent` | 52 passed, 0 failed (13.9 s). Includes `gitlab_exit_reasons_match_the_core_table`, which is untouched by the diff |
| `cargo fmt --all -- --check` | clean |
| `cargo +1.98.1 clippy --workspace --all-targets --locked -- -D warnings -A clippy::unused_async_trait_impl` (the CI line) | clean, whole workspace |
| `cargo clippy -p gridops-tart-agent --all-targets -- -D warnings` (the brief's literal line, on this host's newer default toolchain) | fails on four lines this branch did not write: `main.rs:445` (`unused_async_trait_impl`, which CI allows), and `:2867`, `:2889`, `:3328` (`assert!(x.is_empty())`). `git blame` puts them on commits from 2026-11-02 and 2026-11-03, long before #142. No #142 line is flagged. CI pins 1.98.1 for this reason |
| `bash -n` and `shellcheck scripts/prepare-tart-gitlab-runner-image.sh` | clean; mode 755 |
| Scope: `git diff b22c451 HEAD -- . ':!docs/handoffs'` | exactly 5 files: `gitlab.rs`, `gitlab_tests.rs`, `main.rs` (all in `gridops-tart-agent`), `docs/macos-runners.md`, `scripts/prepare-tart-gitlab-runner-image.sh`. Nothing in `gridops-manager`, `gridops-host-agent`, `gridops-core` or any shared contract type. Working tree clean; I edited no repo file |
| The two pinned SHA-256 values | fetched `https://gitlab-runner-downloads.s3.amazonaws.com/v19.5.3/release.sha256` again: `darwin-arm64` is `43c7ddb8...b7a10` and `darwin-amd64` is `5d2b4514...d659c`, both equal to the script's literals |
| Upstream gitlab-runner v19.5.3 log lines the script keys on | fetched `commands/single.go` and `network/gitlab.go` from the tag. `single.go:167-168` "This runner has processed its build limit, so now exiting"; `:171-172` "This runner has not received a job in N seconds, so now exiting"; `:24` and `:27` the `--wait-timeout` and `--max-builds` flags; `gitlab.go:690` "Checking for jobs... received" and `:702` "Checking for jobs... forbidden". All four strings in `runner_script` match |
| NV-1 `tart exec` exits with the guest's status | fetched `Sources/tart/Commands/Exec.swift` at `543453404...`: `case .exit(let exit): throw ExecCustomExitCodeError(exitCode: exit.code)` (a copy is in the scratchpad). A VM stopped from outside makes the response stream error, which is an ordinary error and not a custom exit code; the residual is in Advisory note 3 |
| Mutation checks on a scratch copy of the tree (the repo's files were never edited; each mutated file was restored and compared byte-for-byte) | 27 mutations, listed below. All 27 fail at least one test for the behaviour they remove |

### Mutation results (scratch copy; each row is one change to `gitlab.rs` or `main.rs`)

| Mutation | Test that goes red |
|---|---|
| A1: a verify answer of 403 no longer exits 77 | `the_guest_script_exits_with_the_status_each_ending_maps_to` |
| A2: a verify answer that is neither 200 nor 403 no longer exits 69 | the same |
| B: delete the `run-single` "forbidden" to 77 branch | the same |
| C: delete the `75\|77` to 70 collision guard | the same |
| E: a silent `run-single` exit of 0 reports 0 instead of 70 | the same |
| F: delete the "Checking for jobs... received" match | the same |
| I: idle exits 0 instead of 75 | the same |
| J: another `run-single` failure forced to 0 | the same |
| K: a missing binary exits 1 instead of 78 | `a_guest_without_gitlab_runner_exits_78_and_says_where_it_looked` |
| G (control): `--max-builds 2` | the script test and `hostile_urls_...` |
| L: token added to `run-single` argv | the script test and `hostile_urls_...` |
| M: token added to `curl` argv | the script test |
| N: the token variable exported | the script test and `hostile_urls_...` |
| O: wait timeout ignored (600) | the script test and `hostile_urls_...` |
| P: URL not single-quoted | `hostile_urls_reach_gitlab_runner_verbatim_and_run_nothing` |
| Q: `--executor docker` | the script test and `hostile_urls_...` |
| R: `exit_reason` returns none for 75 | `exit_statuses_map_to_the_exit_reason_table`, `only_exited_gitlab_runners_with_a_known_code_...` |
| S: token validation drops the `glrt-` prefix check | `each_invalid_gitlab_request_is_refused_without_its_token` |
| T: validation ignores an extra `jitConfig` | the same |
| U: a zero wait timeout accepted | the same |
| V: listing without the `platform == gitlab` check | `only_exited_gitlab_runners_...`, `github_and_bitbucket_runners_never_list_an_exit_reason` |
| W: listing without the `exited` check | `only_exited_gitlab_runners_with_a_known_code_...` |
| X: the waiter records a signal-killed exec as 0 | `a_gitlab_runner_gets_its_token_on_stdin_and_lists_its_exit_reason` |
| Y: the VM stopped before the exit code is written | the same |
| Z: the token passed on `tart exec`'s argv | the same |
| AA: `platform` never persisted | the same |
| AB: the waiter re-inserts a snapshot of the record (resurrects a deleted runner) | `deleting_a_gitlab_runner_as_it_exits_does_not_bring_its_record_back` |

The previous audit's blocking finding (mutations A to E surviving all 51 tests) is closed: each now fails the script test. Mutation S also tripped `every_tart_command_gets_a_path_starting_with_tarts_directory`, one of the two pre-existing `FAKE_TART` flakes (not #142's); the intended test failed with it.

## Spec compliance

| Spec requirement | Compliant | Notes |
|---|---|---|
| `validate`'s `("gitlab", _)` placeholder and its test replaced; the `#[allow(dead_code, reason = "... #142 ...")]` attributes removed | YES | tuple arms `("gitlab", "ephemeral") => gitlab::validate(self)?` then `("gitlab", _)` -> "GitLab Tart runners must be ephemeral."; the CPU and memory checks still run after the `match`. `gitlab_runners_are_refused_by_name_until_142_lands` is gone and no `allow(... #142 ...)` remains |
| Validation: ephemeral and vm only; `gitlab` present; `http`/`https` URL on one line with no whitespace or control characters; wait timeout > 0; token present, non-empty, `glrt-`, single line; an extra `jitConfig`, `bitbucket` or `bitbucketOauthClientSecret` refused (one case each); every refusal `BadRequest`, never containing the token | YES | native runtime is refused earlier, with a message that now names GitLab. Two additions beyond the brief: a 512-byte URL cap (A finding 3) and tokens limited to printable ASCII without `"` and `\` (the precondition of the verify call's JSON body; the diff review's W-3 accepted it). 24 refusal cases, each asserted free of the token in message and `Debug` |
| Token Debug-leak test (B-1) | YES | the 24-case table asserts the token marker is in neither the `BadRequest` message nor `{result:?}`; `a_failed_gitlab_start_never_reports_its_token` covers the `Internal` error with `{:?}` and `{:#}`. I grepped every `tracing::` site and every `gitlab_runner_token` use in production code: the token is only `.take()`n into `SecretString` in the `"gitlab"` arm of `provision_runner` (`main.rs:907-908`), and no log, `context(..)` or `BadRequest` formats the request |
| Guest script: every interpolated value through `shell_single_quote`; binary check -> 78; token read with `IFS= read -r`; `run-single --url --executor shell --max-builds 1 --wait-timeout <s>`; builds directory under `$HOME`; token only as `CI_SERVER_TOKEN`, never argv | YES | read in `gitlab.rs:128-183` and run under bash 3.2-compatible `/bin/bash` by the tests; mutations K, L, M, N, O, P, Q prove each property is pinned |
| 77 detection mechanism named and shown from upstream; the W-1 time-of-check gap stated | YES | two mechanisms, both with a test case: the verify call (`POST <url>/api/v4/runners/verify`, body written by the `printf` builtin into a pipe to `curl --data-binary @-`, `%{http_code}` branched on) and `run-single`'s own "forbidden" line. The upstream source for both is in `evidence.md` and I re-fetched the log lines (table above). The gap and its 300 s latency are in F's Known issues and Advisory note 2 |
| `0` completed, `75` idle, `77` refused; any other failure keeps its own non-table status | YES | and two more statuses outside the table (69: verify answered neither 200 nor 403; 70: `run-single` itself exited 75 or 77, or exited 0 with none of the recognised lines). Both guard the table's invariant ("the script does not force any other failure into the table"), are tested (mutations A2, C, E), and are in the docs |
| Exit-reason lookup via `GITLAB_EXIT_REASONS` | YES | `gitlab::exit_reason`; a status outside the table gives none |
| Runner binary path configurable and validated; one named default constant used by the agent, the image script and the docs; a test pins it | YES | `GRIDOPS_TART_GITLAB_RUNNER_BINARY`, `anyhow::ensure!(valid_guest_path(..))` in `main()`; `DEFAULT_GITLAB_RUNNER_BINARY = /Users/admin/gitlab-runner/bin/gitlab-runner`, named in F's handoff; `the_default_runner_binary_is_a_guest_path_the_image_script_and_docs_name` reads the script and the docs |
| `provision_runner` `"gitlab"` arm: `.take()` then `SecretString`, token on `tart exec -i` stdin only | YES | same shape as the GitHub and Bitbucket arms; mutation Z (token on argv) fails the stdin test |
| Exit status capture: persisted in the record, `#[serde(default)]`, written before the VM stop, survives a restart; crash window stated | YES | `record_exit_code` updates in place under the records write lock, only if the entry still exists, and persists before the waiter runs `tart stop` (mutations Y and AB). A signal-killed `tart exec` records nothing (mutation X). The crash window is in a code comment; see Advisory note 4 for the handoff wording |
| Record which platform a record is, so only GitLab entries carry `exitReason` | YES | `platform: Option<String>`, `#[serde(default)]`, written verbatim; an old record reads as non-GitLab (A finding 2); a test loads a pre-#142 record |
| `list_runners` adds `exitReason` only for an exited GitLab record with a known code | YES | the key is absent, not null, in every other case; a 9-row matrix covers running, other exit, no code, GitHub, Bitbucket and a pre-#142 record (mutations V and W) |
| Image script: pinned release, per-architecture SHA-256, HTTPS and the same curl flags, checksum before install, `chmod`, `--version`, git check without installing, no token | YES | SHA values verified against GitLab's published file; `shasum -a 256 --check` runs before `mv` and `chmod 755`; the guest step is a `bash -s` heredoc as in `prepare-tart-runner-image.sh`. F ran it against a fake `tart`; I did not repeat that run, and `bash -n` and `shellcheck` are clean |
| No existing shell-lint coverage to extend (NV-5) | YES | none exists, none added |
| Docs: GitLab section next to Bitbucket; the image script, the pinned version, ephemeral one-job VMs, idle timeout = the pool's wait timeout (600 default), the shell executor ignores `image:`, which exits carry `exitReason`, and that any other exit (78 named) carries none | YES | also names 69 and 70. Tag wording stays within existing code ("the VM adds none") |
| Pinning test `gitlab_exit_reasons_match_the_core_table` unchanged and passing | YES | not in the diff; passes |
| Contract deviation for `registrationToken` stated for the epic table | YES | in F's earlier handoff and pinned by `a_valid_gitlab_request_is_accepted`. It must also go in the PR body (Advisory note 1) |
| Manual check with a real `glrt-` token on a real Tart VM: "not run, deferred to #143", said exactly | YES | in F's Known issues; it must go in the PR body (Advisory note 1) |
| No `tart` run by the tests | YES | all tests use `FakeTart`, fake `curl` and `gitlab-runner` scripts, or pure functions |

## Quality audit

| Area | Result | Notes |
|------|--------|-------|
| API consistency | PASS | `GET /v1/runners` gains one optional key under the contract's conditions; the request contract is unchanged |
| Error handling | PASS | every refusal is `BadRequest` with a readable message; a failed start is `Internal` with no token in `{:?}` or `{:#}`; a failed exit-code write is logged with the runner id and error only and the VM is still stopped (`a_gitlab_runner_whose_exit_code_cannot_be_written_is_still_stopped`, which forces the failure with a directory at `<id>.json.tmp` so it holds as root) |
| UI/UX match to spec | N/A | no UI |
| Accessibility | N/A | no UI |
| Architecture gate | PASS | A: PASS (plan) and PASS (anti-duplication). Nothing in this audit conflicts with either |
| Code organization | PASS | one new module with a `//!` header and explicit imports; no second quoting helper (`shell_single_quote` is fixed in place); `GitlabBootstrap` and the exit constants stay in `main.rs` where #137 pinned them; the test file is mounted with `#[path]` from the module |
| Security | PASS | token only on `tart exec -i` stdin, in no host or guest argv, never exported, never in the host environment; the verify body is written by the `printf` builtin into a pipe; URL single-quoted through the fixed helper, and every hostile URL (including one with `'`) runs nothing under bash. Depth-of-exposure notes in Advisory notes 5 and 6 |
| Performance | PASS | the waiter holds the records lock across one small atomic file write; one verify call per runner |
| Visual regression | N/A | no visual surface |
| Naming consistency | PASS | `GITLAB_RUNNER_TOKEN_PREFIX` copy beside the exit-code copies; `exitReason` and `completed`/`idle_expired`/`registration_refused` as the contract names them |
| Test quality (`testing/test-quality.md` section 7) | PASS | see below |

## Test-quality audit (section 7 rubric)

**Per test.** I read all 13 #142 tests in `gitlab_tests.rs` and `main.rs`. Each names one behaviour, fails for that behaviour in isolation, sits at the cheapest tier that can show it, and asserts effects rather than implementation:

- validation and exit lookup are pure unit tests;
- the guest script runs under real `bash` with fake executables, the only tier that can show an exit status;
- the waiter and the record store need the handlers and `FakeTart`.

The previous blocker is fixed. `the_guest_script_exits_with_the_status_each_ending_maps_to` no longer reads the script text to choose its expectation: its table is fixed (11 rows, including 69, 70 twice and 77 by both mechanisms), the verify call is always asserted, and `runner-argv` must be empty when verify did not answer 200. The mutation table is the proof: A1, A2, B, C, E and F each go red, and the previous round's five surviving mutations are among them.

`hostile_urls_...` no longer accepts "refused or quoted": the `'` URL is a fixed row that must reach `--url` verbatim, and mutation P and T-red's `shell_single_quote` reversions turn it red.

**Per suite.** 13 new tests plus 4 rows in the 24-row refusal table, for a feature that adds a validation arm, a script, an exit-code store, a listing key and an image script. That is proportionate. The refusal table keeps same-predicate rows (a `\r` token and a two-line token; a space, a tab, a newline and a control character in a URL). T decided "keep" with a reason (an implementation checking only `\n` or only `' '` would pass a trimmed table), and each row names its case on failure. I agree and do not gate on it.

**Smells.** None found.

- No assertion-free or tautological test: the guard assertion in the persist-failure test (the record on disk has no exit code) stops it passing vacuously.
- The delete-race test cannot go red from timing: the fake's 0.3 s grace only decides whether the race is exercised, and either order ends with no record.
- `github_and_bitbucket_runners_never_list_an_exit_reason` is labelled "green by design before #142" and sits in the suite as the regression guard for the platform field; it fails under mutation V, so it can fail.
- The `FAKE_TART` extension (`exec-hold`, `exec-until-deleted`, `exec-status`, `killed`) extends the shared fake in place instead of adding a second one.

No "delete or merge this test" finding.

## Scope check
F delivered the phase scope and nothing outside it: the five files above, no edits to the manager, host agent, core or any shared contract type. One addition beyond the brief's minimum, which F declares and I accept: statuses 69 and 70, both outside the table, both tested, both documented.

## Verdict

PASS. All acceptance criteria are met, the implementation matches the brief (with its review amendments), the previous REWORK is closed with measured proof, and the quality is acceptable. Ready for O to commit.

## Advisory notes

1. **The PR body must state four things** (all are in F's handoff; the PR body needs them too):
   - the manual check with a real `glrt-` token on a real Tart VM is **not run, deferred to #143**, because no GitLab test project or token exists;
   - the contract deviation for the epic #136 table: the Tart agent has no `registration_token` field and no `deny_unknown_fields`, so a stray `registrationToken` on a GitLab request is ignored, not refused (#139 and #147 should not expect a refusal);
   - reachability (A finding 7): #142 works end to end only after #147 lets `("gitlab", "ephemeral")` with the Tart provider through the manager's `validate` and adds D15's `exit_reason` relay. #143's manual check should wait for both;
   - the crash window: if the agent dies after `tart exec` returns and before the record write completes, no code is recorded and that runner lists as `exited` with no `exitReason`. Statuses 69 and 70 never produce an `exitReason`, and an `exitCode` is now recorded for every VM runner (GitHub and Bitbucket too) but only listed for GitLab, so #147 and #139 should not read it as a contract field.
2. **A token revoked while `run-single` is already polling** exits 77 only after gitlab-runner's 300 s not-healthy sleep (`single.go:87-89`), or after the wait timeout if that is longer. A token that is bad at the verify call exits 77 at once. Upstream behaviour, stated in F's handoff.
3. **NV-1's residual.** `tart exec` returning the guest's status is confirmed. What it returns after a stop from outside (`control_runner`'s `stop`, or the VM dying) is not settled by any fake. From `Exec.swift`, a broken response stream is an ordinary error (exit 1, outside the table), but a stream that ended cleanly with no exit event would return 0 and list `completed`. The script never exits 0 unless it recognised a `run-single` line, which does not help if the script itself was killed. #143's manual check should include "stop a running GitLab runner from the pool and read its listing".
4. **Handoff wording.** The brief asked for the crash window "in a code comment and the handoff". The comment exists (`main.rs`, `start_runner`'s waiter); F's second-run handoff only refers to the carried-over notes. Putting the sentence in the PR body (note 1) closes it.
5. **Secret depth of exposure.** `gitlab-runner` prints a shortened token in its own log (`Starting runner for <url> with token <first 8 characters>`), and the script `tee`s that output into the runner's log file, which the agent also surfaces. Upstream's own masking, as for any shell-executor runner. `docs/macos-runners.md` says the token "never appears on a command line or in a log", which is true of everything the agent and the script write, but a reader would not guess this. Not worth a rework; the sentence could say "beyond the shortened form gitlab-runner logs".
6. **Instance-prefixed tokens (contract level, not #142's).** v19.5.3's token shortening strips an optional alphanumeric instance prefix before `glrt-`. A GitLab instance that configures one would issue `<prefix>-glrt-...` tokens, which the `glrt-` check (core's `GITLAB_RUNNER_TOKEN_PREFIX`, copied here as the brief requires) refuses. Worth one line on epic #136 for #139 and #143.
7. **Pre-existing flakes (not #142).** `a_runner_waiting_for_a_slot_is_listed_as_waiting` (a lost-update race in `FAKE_TART`'s `run` branch) and `every_tart_command_gets_a_path_starting_with_tarts_directory` each failed about 2 runs in 15 on the untouched base, by F's measurement. One of them (`every_tart_command_...`) appeared once, under mutation S; neither appeared in my unmutated runs. Worth a separate fixture-hardening issue.
8. **Not covered by any test:** a GitLab URL with a trailing slash through the script (the script strips it for the verify URL, `${gitlab_url%/}`, and passes it unchanged to `--url`). Validation accepts one and the script handles it; the only evidence is reading the line. Low risk.
9. **Scratch files.** Mutation copies, the fetched upstream sources and the harness are under the session scratchpad (`.../scratchpad/mut`, `mut-target`, `up`, `harness`), outside the repository. The previous audit's `/tmp/s142` is still there if no one removed it.
