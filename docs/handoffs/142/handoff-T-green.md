# Handoff-T-green: Phase 7 - GitLab macOS runners on Tart pools, Tart agent side (#142)

**Date:** 2026-11-20
**Branch:** claude/gitlab-142-tart-13d6d4 (head e2838d7)
**Issue:** #142
**Handoff-F reviewed:** docs/handoffs/142/handoff-F.md
**Handoff-T-red:** docs/handoffs/142/handoff-T-red.md

## GREEN confirmation

    $ cargo test -p gridops-tart-agent     (3 consecutive runs)
    test result: ok. 51 passed; 0 failed; 0 ignored   (11.5 s each)

All 51 tests pass, including the 13 #142 tests (7 in `gitlab_tests.rs`, 6 in `main.rs`) and the 38 existing ones. No test of mine needed repair: F flagged none as wrong, and none failed.

**Behaviour, not implementation (spot check).** The RED run in Phase 4 already showed the suite failing against a stub scaffold (`runner_script` -> `exit 1`, `exit_reason` -> `None`, constants empty): 11 failed / 40 passed, each on the behaviour assertion. The tests run the real generated script under `/bin/bash` and drive the real handlers, so they fail if the script's exit mapping, token channel or record write is removed. I did not mutate production code (it is F's).

## Tier 1 results

| Check | Command | Result |
|---|---|---|
| Format | `cargo fmt --all -- --check` | PASS |
| Lint (CI pin and flags) | `cargo +1.98.1 clippy --workspace --all-targets --locked -- -D warnings -A clippy::unused_async_trait_impl` | PASS, no warnings |
| Shell | `bash -n` and `shellcheck scripts/prepare-tart-gitlab-runner-image.sh` | PASS; mode 755 |
| Tart agent tests | see above | PASS 51/51 |
| Workspace tests | `cargo test --workspace --no-fail-fast` | PASS except 2 environmental, pre-existing items below |

Workspace failures, none in code this branch touches (`git diff 229081c HEAD` has no change under `crates/gridops-api` or `crates/gridops-manager`):

- `gridops-manager`: 11 tests failed with `path must be shorter than SUN_LEN`. This is the long worktree path in `TMPDIR` overflowing a unix socket path. With a short `TMPDIR`, **105/105 pass**.
- `gridops-api` `fleet_http release_preflight::the_release_preflight_fails_early_without_a_macos_arm64_machine`: fails because this dev host is itself macOS arm64, so the test's "no macOS machine" premise does not hold. It is unchanged by #142 and unrelated to it.

Server start / API smoke: the agent's HTTP handlers (`provision_runner`, `list_runners`, `delete_runner`) are exercised through the real router and handlers in the `main.rs` tests with a fake `tart`; there is no separate server to start for this change.

## Tier 2 results

| Check | Method | Result |
|---|---|---|
| Coverage per criterion | mapped each brief criterion to a test (below) | PASS |
| Test quality (`test-quality.md` section 7) | read the new tests: each names one behaviour; guard test `github_and_bitbucket_runners_never_list_an_exit_reason` is green by design before #142 (declared in T-red); the multi-case tests are table-driven single-change cases, not duplicates; suite size is proportionate (13 tests for 1 new module + 3 persisted/listed fields) | PASS |
| Type safety | no `any` analogue; grep of the main.rs diff for `unwrap()`/`expect(`/`unsafe`: none added; clippy `-D warnings` clean | PASS |
| Error handling | invalid-input matrix (22 cases) returns `BadRequest` without the token in message or `Debug`; failed start returns `Internal` without the token; missing runner binary exits 78 | PASS |
| Data integrity | `record_exit_code` updates in place under the records lock, never inserts; delete-vs-exit race pinned; pre-#142 records load with `None` fields | PASS |
| API contract | `exitReason` key present only for exited GitLab runners with code 0/75/77, absent (not null) otherwise; request shape unchanged (D9/D10/D12/D15) | PASS |
| Security | token reaches the guest only on `tart exec -i` stdin; in no argv, no `tart` command line, no log or error; hostile URLs run nothing (`'` handled by the fixed quoting helper); the script's curl and the runner argv checked by the test fakes | PASS |
| Migration safety | no schema migration; `TartRecord` fields are optional and `#[serde(default)]`, so rollback to an older agent ignores them | PASS |
| Playwright | not used by this project/change | N/A |
| Evidence appendix | `docs/handoffs/142/evidence.md` already lists the unchanged-code facts the tests rely on (delete order, atomic `persist_record`, stdin channel, `stopped` -> `exited` mapping) plus the upstream citations; no gaps found | PASS |

## Cross-check of F's results

I re-ran the commands F reported: tests (51/51 green, matches), fmt, clippy and shellcheck (clean, match). F reported 2 flaky pre-existing tests; I did not reproduce either in 3 full runs plus the workspace run.

## Acceptance criteria status

| Criterion | Backed by | Status |
|---|---|---|
| A valid GitLab request is accepted (https/http+port/path, stray `registrationToken` ignored) | `a_valid_gitlab_request_is_accepted` | PASS |
| Invalid requests refused, with no token in message or Debug | `each_invalid_gitlab_request_is_refused_without_its_token` | PASS |
| Token delivered on stdin only; exit code recorded before VM stop; persisted across restart | `a_gitlab_runner_gets_its_token_on_stdin_and_lists_its_exit_reason` | PASS |
| Exit status mapping (0/75/77 to reasons, others none) | `exit_statuses_map_to_the_exit_reason_table`, `the_guest_script_exits_with_the_status_each_ending_maps_to` | PASS |
| `exitReason` listing matrix (D15) | `only_exited_gitlab_runners_with_a_known_code_list_an_exit_reason`, `github_and_bitbucket_runners_never_list_an_exit_reason` | PASS |
| Shell executor, `--max-builds 1`, wait timeout, builds dir under `$HOME`, token only as `CI_SERVER_TOKEN` | `the_guest_script_exits_with_the_status_each_ending_maps_to` | PASS |
| Missing image binary exits 78 and names the path | `a_guest_without_gitlab_runner_exits_78_and_says_where_it_looked` | PASS |
| Hostile URL values inert | `hostile_urls_reach_gitlab_runner_verbatim_and_run_nothing` | PASS |
| Default binary path agrees across code, image script and docs (W-2) | `the_default_runner_binary_is_a_guest_path_the_image_script_and_docs_name` | PASS |
| Delete racing the exit does not resurrect the record (A finding 1) | `deleting_a_gitlab_runner_as_it_exits_does_not_bring_its_record_back` | PASS |
| Pre-#142 records load; new fields persist | `records_keep_their_exit_code_and_records_from_before_142_still_load` | PASS |
| No token in a failed-start error | `a_failed_gitlab_start_never_reports_its_token` | PASS |

## Blocking issues

None.

## Advisory notes

- **Test-fixture flakes (pre-existing, from #137, not #142):** `a_runner_waiting_for_a_slot_is_listed_as_waiting` (a lost-update race in the `FAKE_TART` `run` branch) and `every_tart_command_gets_a_path_starting_with_tarts_directory`. F measured about 2/15 failures on the untouched base. Worth a separate fixture-hardening issue; not a #142 blocker.
- **Coverage hole for #143:** the "job received" -> 0 path was checked only against the upstream-modelled fake, and a real `glrt-` token on a real Tart VM was not run (F's Known issues). A real-VM manual check is deferred to #143, as F states.
- **Reachability:** per F, the feature is not reachable end to end until #147 (manager `validate` and the `exit_reason` relay).
- **Mid-run revoked token:** exits 77 only after about 300 s (gitlab-runner's not-healthy sleep). Upstream behaviour, documented by F.
- **Open NV-1 hedge:** whether `tart exec` returns the guest's status after an outside `tart stop` is unsettled; a runner stopped from outside could list "completed".
