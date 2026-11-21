# Handoff-T-green: Phase 7 - GitLab macOS runners on Tart pools, Tart agent side (#142), second run

**Date:** 2026-11-20
**Branch:** claude/gitlab-142-tart-13d6d4 (head 9a0b2eb)
**Issue:** #142
**Handoff-F reviewed:** docs/handoffs/142/handoff-F.md (no production change in this run)
**Handoff-T-red:** docs/handoffs/142/handoff-T-red.md (second run)

## GREEN confirmation

    $ cargo test -p gridops-tart-agent     (3 consecutive runs)
    test result: ok. 52 passed; 0 failed; 0 ignored   (13.7 s each)

52/52 pass, including the five tests and rows added or changed in this run's RED. F flagged no test as wrong, and none failed, so I changed no test.

**Behaviour, not implementation (spot check).** I re-applied one of the Phase 4 mutations myself on a scratch copy of the tree (outside the repository): the waiter records `code().unwrap_or(0)` for a `tart exec` ended by a signal. The suite then failed `a_gitlab_runner_gets_its_token_on_stdin_and_lists_its_exit_reason` with `exec killed: left Some(0), right None`, plus the known flake `a_runner_waiting_for_a_slot_is_listed_as_waiting`. The worktree's production code was not edited. I also read the production lines F cites: `gitlab.rs` `valid_token` (refuses `"` and `\`), the waiter's `code()` guard, and `record_exit_code` (logs a persist error, returns, and the waiter goes on to `tart stop`).

## Tier 1 results

| Check | Command | Result |
|---|---|---|
| Format | `cargo fmt --all -- --check` | PASS |
| Lint (CI pin and flags) | `cargo +1.98.1 clippy -p gridops-tart-agent --all-targets --locked -- -D warnings -A clippy::unused_async_trait_impl` | PASS, no warnings |
| Shell | `bash -n` and `shellcheck scripts/prepare-tart-gitlab-runner-image.sh` | PASS |
| Tart agent tests | see above | PASS 52/52, 3 runs |
| Workspace tests | `TMPDIR=/tmp/t142 cargo test --workspace --no-fail-fast` | PASS except 1 environmental, pre-existing item below |

The one workspace failure is `gridops-api` `release_preflight::the_release_preflight_fails_early_without_a_macos_arm64_machine`. The test assumes the host is not a macOS arm64 machine, and this dev host is one. It is unrelated to #142: this branch changes nothing under `crates/gridops-api` (`git diff 6d5e2e1 HEAD` touches only `gridops-tart-agent`). `gridops-manager` passes 105/105 with a short `TMPDIR` (a long worktree path overflows `SUN_LEN` otherwise).

Server start / API smoke: the agent's handlers (`provision_runner`, `list_runners`, `delete_runner`) run through the real router in the `main.rs` tests with a fake `tart`; there is no separate server to start for this change.

## Tier 2 results

| Check | Method | Result |
|---|---|---|
| Coverage per criterion | every Done-when item maps to a test (table below) | PASS |
| Test quality (`test-quality.md` section 7) | each new or changed test names one behaviour and fails for the right reason (Phase 4 mutations N1 to N5, plus my own N4 re-run); the added rows are distinct inputs, not duplicates (the refusal-table trim was decided "keep" with reasons in T-red); the suite is proportionate (the second run added 1 test and 4 rows) | PASS |
| Type safety | no `unsafe`, `unwrap()` or `expect(` added; clippy `-D warnings` clean | PASS |
| Error handling | 24-row invalid-request matrix returns `BadRequest` with no token; failed start returns `Internal` with no token; a failed exit-code write is logged and the VM still stops | PASS |
| Data integrity | `record_exit_code` updates in place under the records lock and never inserts; delete-vs-exit race pinned; a signal-killed exec records no code | PASS |
| API contract | `exitReason` only for exited GitLab runners with code 0/75/77, absent otherwise; request shape unchanged | PASS |
| Security | token on `tart exec -i` stdin only; `"` and `\` refused so the verify body's JSON cannot be broken out of; hostile URLs including `'` run nothing | PASS |
| Migration safety | no schema migration; new `TartRecord` fields are optional and `#[serde(default)]` | PASS |
| Playwright | not used | N/A |
| Evidence appendix | `evidence.md` already holds T's second-run entries (`persist_record`'s `.json.tmp`, the waiter's `code()` guard, the `std` doc on signal exits) and F's earlier entries; I found no further gap | PASS |

## Cross-check of F's results

I re-ran F's commands: tests (52/52 on 3 runs, matches), fmt, clippy and shellcheck (clean, match). F reported no `FAKE_TART` flake in its runs; I saw none in 3 plain runs either. The flake `a_runner_waiting_for_a_slot_is_listed_as_waiting` appeared only under my deliberate mutation run.

## Acceptance criteria status

| Criterion | Backed by | Status |
|---|---|---|
| A valid GitLab request is accepted | `a_valid_gitlab_request_is_accepted` | PASS |
| Invalid requests refused, with no token in message or Debug (incl. `"` and `\` tokens) | `each_invalid_gitlab_request_is_refused_without_its_token` | PASS |
| Token on stdin only; exit code recorded before VM stop; persisted across restart | `a_gitlab_runner_gets_its_token_on_stdin_and_lists_its_exit_reason` | PASS |
| Signal-killed exec records no code and lists no `exitReason` | same test, `killed` row | PASS |
| Failed exit-code write still stops the VM | `a_gitlab_runner_whose_exit_code_cannot_be_written_is_still_stopped` | PASS |
| Exit status mapping (0/75/77), job taken then stopped early gives 0 | `exit_statuses_map_to_the_exit_reason_table`, `the_guest_script_exits_with_the_status_each_ending_maps_to` | PASS |
| `exitReason` listing matrix | `only_exited_gitlab_runners_with_a_known_code_list_an_exit_reason`, `github_and_bitbucket_runners_never_list_an_exit_reason` | PASS |
| Shell executor, `--max-builds 1`, wait timeout, token only as `CI_SERVER_TOKEN` | `the_guest_script_exits_with_the_status_each_ending_maps_to` | PASS |
| Missing image binary exits 78 and names the path | `a_guest_without_gitlab_runner_exits_78_and_says_where_it_looked` | PASS |
| Hostile URL values inert, `'` fixed expectation | `hostile_urls_reach_gitlab_runner_verbatim_and_run_nothing` | PASS |
| Default binary path agrees across code, image script, docs | `the_default_runner_binary_is_a_guest_path_the_image_script_and_docs_name` | PASS |
| Delete racing the exit does not resurrect the record | `deleting_a_gitlab_runner_as_it_exits_does_not_bring_its_record_back` | PASS |
| Pre-#142 records load; new fields persist | `records_keep_their_exit_code_and_records_from_before_142_still_load` | PASS |
| No token in a failed-start error | `a_failed_gitlab_start_never_reports_its_token` | PASS |

## Blocking issues

None.

## Advisory notes

- **Pre-existing fixture flakes (from #137, not #142):** `a_runner_waiting_for_a_slot_is_listed_as_waiting` and `every_tart_command_gets_a_path_starting_with_tarts_directory`. A separate fixture-hardening issue is worth filing (S's advisory 8).
- **Coverage hole for #143:** a real `glrt-` token on a real Tart VM was not run; the job-received path is checked only against the upstream-modelled fake.
- **NV-1 open:** what `tart exec` returns after an outside `tart stop` is a runtime question no fake settles. The script never exits 0 unless it recognised a `run-single` line, which limits the risk.
- **Reachability:** end to end only after #147.
- **Mid-run revoked token** exits 77 only after about 300 s (upstream not-healthy sleep).
