# Handoff-S: Phase 8 - GitLab macOS runners on Tart pools, Tart agent side (#142)

**Date:** 2026-11-20
**Branch:** claude/gitlab-142-tart-13d6d4 (head f6f4610)
**Issue:** #142
**Handoff-T reviewed:** docs/handoffs/142/handoff-T-green.md (and handoff-T-red.md)
**Handoff-A reviewed:** docs/handoffs/142/handoff-A.md (plan: PASS) and docs/handoffs/142/handoff-A-dup.md (anti-duplication: PASS)
**Handoff-F reviewed:** docs/handoffs/142/handoff-F.md
**Spec:** docs/handoffs/142-brief.md (issue text, plus the #137 deviations D9, D10, D12, D15, and the review round 1-3 amendments; where they conflict, the amendments win, as the brief states)
**Operator-facing report:** not applicable (no visual surface)

## A precondition
Confirmed. A returned PASS on the plan (seven warnings, no blocks) and PASS on the anti-duplication gate (no duplication; extension is clean).

## T precondition
Confirmed. T-green reports "Blocking issues: None". Its two workspace failures are environmental and in code this branch does not touch (11 `gridops-manager` `SUN_LEN` socket-path tests from the long worktree path; one macOS-host `release_preflight` test).

## Visual-diff-tool precondition
Not applicable. #142 is a Rust crate change, a shell script and a docs section. It has no rendered surface, so the browser, Playwright and ImageMagick preconditions, the pixel diffs and the HTML report do not apply.

## Preview / spec sanity check
Done on the brief before auditing. No source-of-truth defect that justifies ADVISORY-HOLD. One upstream fact worth telling O, which is not F's to fix (see Advisory notes, item 6): gitlab-runner v19.5.3 accepts an instance-prefixed token (`<prefix>-glrt-...`), and the contract's `glrt-` prefix check, which the brief mandates and #137's core constant defines, would refuse one.

## Visual diff results
Not applicable.

## Independent verification I ran (not only T's and F's reports)

| Check | Result |
|---|---|
| `cargo test -p gridops-tart-agent` | 51 passed, 0 failed (12.0 s). Includes the 13 #142 tests and `gitlab_exit_reasons_match_the_core_table`, which is untouched by the diff |
| `cargo fmt --all -- --check` | clean |
| `cargo +1.98.1 clippy --workspace --all-targets --locked -- -D warnings -A clippy::unused_async_trait_impl` (the exact CI line, `.github/workflows/ci.yml:113`) | clean |
| `cargo clippy --workspace --all-targets -- -D warnings` (the brief's literal Done-when line, on this host's newer default toolchain) | fails, only on code this branch did not write: `main.rs:445` (`unused_async_trait_impl`, which CI allows), `main.rs:2867`, `:2889`, `:3324` (`assert!(x.is_empty())` in existing tests) and `gridops-host-agent`. CI pins 1.98.1 for this reason, and passes. No #142 line is flagged |
| `bash -n` and `shellcheck scripts/prepare-tart-gitlab-runner-image.sh` | clean; mode 755 |
| Scope: `git diff b22c451 HEAD -- . ':!docs/handoffs'` | exactly 5 files: `gitlab.rs`, `gitlab_tests.rs`, `main.rs` (all `gridops-tart-agent`), `docs/macos-runners.md`, `scripts/prepare-tart-gitlab-runner-image.sh`. Nothing in `gridops-manager`, `gridops-host-agent`, `gridops-core` or any shared contract type (b22c451, the #137 merge, is an ancestor of HEAD) |
| NV-4: the two pinned SHA-256 values | fetched GitLab's published `https://gitlab-runner-downloads.s3.amazonaws.com/v19.5.3/release.sha256`. `darwin-arm64` is `43c7ddb8...b7a10` and `darwin-amd64` is `5d2b4514...d659c`, both equal to the script's values |
| NV-1: `tart exec` exits with the guest's status | fetched `cirruslabs/tart` at `543453404...` and read `Exec.swift:195-196` (`throw ExecCustomExitCodeError(exitCode: exit.code)`) and `Root.swift:164-168` (`Foundation.exit(execCustomExitCodeError.exitCode)`). Confirmed |
| NV-2: `run-single` flags, token env, log lines, exit 0 on both idle and job | fetched `gitlab-org/gitlab-runner` `v19.5.3` `commands/single.go`: the build-limit line (`:168`), the wait-timeout line (`:172`), `NotHealthyCheckInterval` sleep (`:87-89`), `Execute` ending normally after a job error, `--wait-timeout` and `--max-builds` (`:24`, `:27`). Confirmed. A runner's job error does not change the exit status, so a failed job still exits 0 |
| The generated script under the guest's `/bin/bash` 3.2.57, with my own fake `curl` and `gitlab-runner` (a scratch copy of the script text, not F's test) | verify 403 -> 77; verify 500 -> 69; curl failing (000) -> 69; `run-single` exiting 75 -> 70; exiting 77 -> 70; exiting 78 -> 78; exiting 0 with none of the recognised lines -> 70; idle line -> 75. A token containing `$`, a backtick, `'`, `%`, `!`, `&`, `*`, `;`, `\|`, `(`, `)`, `{` and `}` arrived byte-for-byte in the verify request body on stdin and in `CI_SERVER_TOKEN`, and appeared in no argv and no script output. F's behavioural claims are accurate |
| Mutation checks on a scratch copy of the tree (the repo file was restored and compared byte-for-byte) | see "Test-quality audit". The tests catch a real regression (control: `--max-builds 2` fails the script test) but not five of the script's branches |

## Spec compliance

| Spec requirement | Compliant | Notes |
|---|---|---|
| `ProvisionRunner::validate` `("gitlab", _)` placeholder and its test replaced; `#[allow(dead_code)]` removed | YES | tuple arms `("gitlab", "ephemeral") => gitlab::validate(self)?` then `("gitlab", _)` -> "GitLab Tart runners must be ephemeral."; the CPU and memory checks still run after. `gitlab_runners_are_refused_by_name_until_142_lands` is removed, and no `allow(dead_code, reason = "... #142 ...")` remains |
| Validation: ephemeral and vm only; `gitlab` present; `http`/`https` URL, single line, no whitespace or control characters; wait timeout > 0; token present, non-empty, `glrt-`, single line; extra `jitConfig`/`bitbucket`/`bitbucketOauthClientSecret` refused; refusals are `BadRequest` and never contain the token | YES | native runtime refused earlier with a message that now names GitLab; 512-byte URL cap added per A finding 3; token also limited to printable ASCII without `"` and `\`. 22 refusal cases, each asserted free of the token in message and `Debug` |
| Guest script built with `shell_single_quote` for every interpolated value; binary check -> 78; token read from stdin with `IFS= read -r`; `run-single --url --executor shell --max-builds 1 --wait-timeout <s>`; builds dir under `$HOME`; token only as `CI_SERVER_TOKEN`, never argv | YES | confirmed by reading the script, by T's tests and by my own bash 3.2 run |
| Exit 0 / 75 / 77 only for completed / idle / refused; any other failure keeps its own non-table status | YES | and F added statuses 69 and 70 for two cases the brief did not name (see Scope check). I confirmed each by running the script. These branches are not pinned by a test (finding 1) |
| 77 detection: a mechanism "whose behaviour you can show from documented or source behaviour", stated in the handoff; W-1 time-of-check gap stated | YES | F implements both mechanisms (verify, then `run-single`'s "forbidden" line), cites upstream for each, and states the 300 s latency of the second. Verified against upstream by me |
| Exit-reason lookup via `GITLAB_EXIT_REASONS` | YES | `gitlab::exit_reason`; an exit status outside the table gives none |
| Runner binary path configurable and validated; one named default constant used by the agent, the image script and the docs; a test pins it | YES | `GRIDOPS_TART_GITLAB_RUNNER_BINARY`, `anyhow::ensure!(valid_guest_path(..))` in `main()`; `DEFAULT_GITLAB_RUNNER_BINARY = /Users/admin/gitlab-runner/bin/gitlab-runner`; named in the handoff |
| `provision_runner` `"gitlab"` arm: `.take()` then `SecretString::from`, token on `tart exec -i` stdin | YES | same shape as the GitHub and Bitbucket arms. No `tracing` call, `context(..)` or `BadRequest` formats the request or the token (I grepped every `tracing::` site and `gitlab_runner_token` use) |
| Exit status capture: recorded in the record file, `#[serde(default)]`, written before the VM stop, survives a restart; crash window stated | YES | `record_exit_code` updates in place under the records write lock and never inserts (A finding 1), persists, then the waiter runs `tart stop`. The crash window is in a code comment. F's handoff only points at that comment instead of stating the behaviour (see Advisory notes, item 4) |
| Record which platform a record is, so only GitLab entries carry `exitReason` | YES | `platform: Option<String>`, `#[serde(default)]`, written verbatim; an old record reads as non-GitLab (A finding 2) |
| `list_runners` adds `exitReason` only for an exited GitLab record with a known code | YES | key absent, not null, in every other case; the 9-row matrix test covers it |
| Image script: pinned release, per-architecture SHA-256, HTTPS with the same curl flags, checksum before install, `chmod`, `--version`, git check without installing, no token, same shape as the Bitbucket script | YES | SHA values verified against GitLab's published file. F ran it against a fake `tart` and reports the download, `shasum --check` and install succeeding. I did not repeat that run |
| No existing shell-lint coverage to extend (NV-5) | YES | none exists, none added |
| Docs: GitLab section next to Bitbucket; image script, pinned version, ephemeral one-job VMs, idle timeout = the pool's wait timeout (600 default), the shell executor ignores `image:`, which exits carry `exitReason`, and that any other exit (78 named) carries none | YES | also names 69 and 70. Tag wording stays within existing code ("the VM adds none") |
| Pinning test `gitlab_exit_reasons_match_the_core_table` unchanged and passing | YES | not in the diff; passes |
| Contract deviation for `registrationToken` stated for the epic table | YES | in F's handoff and pinned by `a_valid_gitlab_request_is_accepted`. It must also go in the PR body (Advisory notes, item 1) |
| Manual check with a real token on a real VM: "not run, deferred to #143", said exactly | YES | F's Known issues say so. Must go in the PR body |
| No `tart` run by the tests | YES | all tests use `FakeTart` or pure functions |

## Quality audit

| Area | Result | Notes |
|------|--------|-------|
| API consistency | PASS | `GET /v1/runners` gains one optional key under the contract's conditions; the request contract is unchanged |
| Error handling | PASS | every refusal is `BadRequest` with a readable message; a failed start is `Internal` and neither `{:?}` nor `{:#}` of it holds the token (`a_failed_gitlab_start_never_reports_its_token`); a persist failure in the waiter is logged with the runner id and error only and does not stop the VM stop |
| UI/UX match to spec | N/A | no UI |
| Accessibility | N/A | no UI |
| Architecture gate | PASS | A: PASS (plan) and PASS (anti-duplication). Nothing in this audit conflicts with either |
| Code organization | PASS | one new module with a `//!` header and explicit imports, as A asked; no second quoting helper; `GitlabBootstrap` and the exit constants stay in `main.rs` where #137 pinned them |
| Security | PASS | token only on `tart exec -i` stdin, in no host or guest argv, never exported, never in the host environment; the verify body is written by the `printf` builtin into a pipe; URL single-quoted through the fixed `shell_single_quote`; hostile-URL cases run nothing. Two depth-of-exposure notes in Advisory notes, item 5 |
| Performance | PASS | the waiter holds the records lock across one small atomic file write (per A finding 1's direction); one verify call per runner |
| Visual regression | N/A | no visual surface |
| Naming consistency | PASS | `GITLAB_RUNNER_TOKEN_PREFIX` copy beside the exit-code copies; `exitReason`, `completed`/`idle_expired`/`registration_refused` as the contract names them |
| Test quality (`testing/test-quality.md` section 7) | FAIL | see below: the script's exit-status test adapts its expectations to the script's own text, and five script branches can be deleted with all 51 tests green |

## Test-quality audit (section 7 rubric)

Per test, the other 12 #142 tests meet all four criteria. The listing matrix, the persistence test, the delete race, the failed-start test, the validation tables and the exit-lookup test each name one behaviour, fail for it, sit at the cheapest tier that can show it, and assert effects. The delete-race test cannot go red from timing: the fake's 0.3 s grace only decides whether the race is exercised, and either order ends with no record.

**`the_guest_script_exits_with_the_status_each_ending_maps_to` (`gitlab_tests.rs:491-544`) fails criterion 2 ("fails in isolation, for the right reason").** It reads the generated script's text to decide what to expect:

    let verifies = probe.contains("runners/verify");
    let reads_refusal = probe.to_ascii_lowercase().contains("forbidden");
    ...  if reads_refusal { 77 } else { 75 }     (line 517)
    ...  if verifies { cases.push(.. 403 -> 77) } (line 520)
    ...  if verifies { assert_verify_call(..) }   (line 539)

T wrote this before F, to be mechanism-agnostic. Both mechanisms now exist, so the expectation must be fixed, not derived from the code under test. As written, deleting a mechanism also deletes its test case, and the test stays green.

Measured on a scratch copy of the tree (the repo's `gitlab.rs` was never edited and was compared byte-for-byte afterwards). Control: changing `--max-builds 1` to `2` fails the script test, so the harness does catch real regressions.

| Mutation of `runner_script` | Result |
|---|---|
| A. Delete the whole verify call and its `case` (no 403 -> 77, no non-answer -> 69) | all 7 `gitlab::tests` pass |
| B. Delete the `run-single` "forbidden" -> 77 branch | script test passes (`reads_refusal` flips and the expected status becomes 75) |
| C. Delete the `75\|77` -> 70 collision guard (a `run-single` that itself exits 75 would list as `idle_expired`) | all 51 tests pass |
| D. Verify answering neither 200 nor 403 no longer exits 69 | script test passes |
| E. A `run-single` that exits 0 with none of the recognised lines (a signal stop) reports 0 instead of 70, so it lists as `completed` | script test passes |

For B, D and E I ran the script test alone. The only other tests that run the script (`hostile_urls_...`, `a_guest_without_...`) use verify 200 and a passing job, so they cannot reach those branches. The `main.rs` tests use a fake `tart` and never run the script.

This matters for the issue's point. The exit status is the entire contract with #139. A future edit that loses 77 detection, or lets a crashed or killed `run-single` read as `completed` or `idle_expired`, would ship silently. The brief's own words are "the script does not force any other failure into the table", and branches C and E exist for exactly that. Under section 3, each branch F added earns a case, and C, D and E have none.

## Scope check
F delivered the phase scope and nothing outside it. One addition beyond the brief's minimum, which F declares and I accept: statuses 69 (the verify call got neither 200 nor 403) and 70 (a `run-single` that stopped without taking a job or reaching its timeout, or that itself exited 75 or 77). Both are outside the table, as "any other failure keeps its own non-table status" requires, and both guard the table's invariant. They must be tested (finding 1).

## Verdict

REWORK, test-only. Production code, the image script and the docs are correct and need no change. All changes are in `crates/gridops-tart-agent/src/gitlab_tests.rs`.

1. **Pin the script's exit-status branches in `the_guest_script_exits_with_the_status_each_ending_maps_to`** (T's repair):
   - Delete the script-text probing (`probe`, `verifies`, `reads_refusal`, lines 493-502) and the three conditionals at lines 517, 520-527 and 539-541. The expectations become fixed:
     - refused by `run-single` after a passing verify: exactly 77;
     - refused by the verify call (403): exactly 77, and `runner-argv` must be empty (`run-single` never ran);
     - `assert_verify_call` always runs when verify is involved.
   - Add the three branches F added, as rows in the same table-driven test, each a different branch:
     - the verify call answers neither 200 nor 403 (the fake's `verify-status` set to 500): exactly 69, and `run-single` never ran;
     - `run-single` itself exits 75: exactly 70 (77 may be a second row, at T's discretion, as the guard is one `case`);
     - `run-single` exits 0 and prints none of the recognised lines: exactly 70.
   - `FAKE_GITLAB_RUNNER` needs two new behaviours for those rows (an exit of 75, and a silent exit of 0). The fake `curl` already honours any `verify-status`.
2. **Show the repair works.** Re-run mutations A to E from the table above on a scratch copy and report that each now fails this test. Keep the `--max-builds` control. Report the result in T's handoff.

## Advisory notes

1. **PR body must state** (all four are in F's handoff; the PR body needs them too):
   - the manual check with a real `glrt-` token on a real Tart VM is **not run, deferred to #143**, because no GitLab test project or token exists;
   - the contract deviation for the epic #136 table: the Tart agent has no `registration_token` field and no `deny_unknown_fields`, so a stray `registrationToken` on a GitLab request is ignored, not refused (#139 and #147 should not expect a refusal);
   - reachability (A finding 7): #142 works end to end only after #147 lets `("gitlab", "ephemeral")` with the Tart provider through the manager's `validate` and adds D15's `exit_reason` relay; #143's manual check should wait for both;
   - the crash window: if the agent dies after `tart exec` returns and before the record write completes, no code is recorded and that runner lists as `exited` with no `exitReason`.
2. **Mid-run revoked token.** A token refused while `run-single` is already polling exits 77 only after gitlab-runner's 300 s not-healthy sleep (or after the wait timeout, if longer). Upstream behaviour, confirmed in source by me. A token that is bad at the verify call exits 77 at once.
3. **"Job received" -> 0 is checked only against the upstream-modelled fake.** F ran the real 19.5.3 binary for 75 and 77, not for a real job. #143's manual check covers it.
4. **Docs and handoff nits, not blocking.** The brief asked for the crash window "in a code comment and the handoff"; the handoff only says the comment exists. `docs/macos-runners.md` describes 70 only as "stopped without taking a job or reaching its wait timeout"; it also covers a `run-single` that itself exits 75 or 77 (the diff review's W-1 asked for this clause).
5. **Secret depth of exposure.** `gitlab-runner` prints a shortened token in its own log (`Starting runner for <url> with token <first 8 characters>`; `helpers.ShortenToken`, v19.5.3), and this script `tee`s that output into the runner's log file. Upstream's own masking, and the same as any shell-executor runner. Separately, the token stays in an unexported shell variable for the whole `run-single` duration and is unset afterwards; it is not in `gitlab-runner`'s environment other than as `CI_SERVER_TOKEN`, as T's test asserts.
6. **Instance-prefixed tokens (contract level, not #142's).** v19.5.3's `helpers/shorten_token.go` strips an optional alphanumeric instance prefix before `glrt-`. A GitLab instance that configures one would issue `<prefix>-glrt-...` tokens, which the `glrt-` prefix check (core's `GITLAB_RUNNER_TOKEN_PREFIX`, copied here as the brief requires) refuses. Worth a line on epic #136 for #139 and #143.
7. **Test hygiene, optional.** The 22-case refusal table has same-predicate duplicates (a `\r` token and a two-line token; a space, a tab, a newline and a control character in a URL each take the one whitespace-or-control check). They sit in one test and name the case on failure, so I am not gating on them. T may trim.
8. **Pre-existing flakes (not #142).** `a_runner_waiting_for_a_slot_is_listed_as_waiting` (a lost-update race in `FAKE_TART`'s `run` branch) and `every_tart_command_gets_a_path_starting_with_tarts_directory` fail about 2 runs in 15 on the untouched base too (F's measurement). Every run of mine was green. Worth a separate fixture-hardening issue.
9. **Scratch files.** I left copies and probes under `/tmp/s142` (a mutated tree copy, fetched upstream sources, probe scripts). They are outside the repository and safe to delete; the system removal check stopped me from deleting them myself.
