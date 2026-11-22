# Handoff-T-green: Phase 7 - Flaky tests: tart-agent slot wait and API auth boundary (#124)

**Date:** 2026-11-22
**Branch:** claude/fix-124-flaky-tests
**Issue:** #124
**Handoff-F reviewed:** docs/handoffs/124/handoff-F.md
**Handoff-T-red:** docs/handoffs/124/handoff-T-red.md

## GREEN confirmation
All commands with `CARGO_BUILD_JOBS=2 RUST_TEST_THREADS=2`, host load average 6 to 11.
- `cargo test -p gridops-tart-agent`: `ok. 54 passed; 0 failed` (R1 and R2 included).
- `cargo test -p gridops-api --no-fail-fast`: unit 130 passed; `control_plane_contracts` 41 passed (R3, R4); `fleet_http` 151 passed, 1 failed. R5 and R6 pass. The one failure is `release_preflight::the_release_preflight_fails_early_without_a_macos_arm64_machine`, pre-existing on base `9e357e0` on this macOS host (documented in handoff-T-red), unrelated to #124.
- T's own sweep edits: `cargo test -p gridops-core --test gitlab_runner_script` 12 passed; `TMPDIR=$HOME/.cache/t124 cargo test -p gridops-manager tart_outage` 12 passed.

Spot-check that the tests pin behavior (mutations on the committed code, restored with `git checkout`):
- `hold_slot`/`free_slot` switched back to the counted `busy` file: R1 fails on its assertion (`busy` is `"998\n"`, not `"1000"`).
- The helper's `/api/v1/auth/me` check disabled: R3 (`the_fixture_moves_off_a_port_another_server_answers_on`) and R6 (`the restart accepted another server`) both fail.

Independent stress (final binaries, three loops at once, `RUST_TEST_THREADS=2`):
- tart `a_runner_waiting_for_a_slot_is_listed_as_waiting`: 40 runs, 0 failures.
- tart `every_tart_command_gets_a_path_starting_with_tarts_directory`: 40 runs, 0 failures.
- api `api_routes_enforce_authentication_and_admin_boundaries`: 30 runs, 0 failures.
These are smaller than F's 100/300/50 counts and agree with them (F: 0 failures in all). The old-fixture rates are T-red's measurements (R1 20/20, R2 20/20, sibling 7/300, waiting test 2/25 full-suite runs).

## Tier 1 results
| Check | Result |
|---|---|
| `cargo fmt --check` (api, tart-agent, core, manager) | PASS |
| `cargo +1.98.1 clippy -p <crate> --all-targets -- -D warnings -A clippy::unused_async_trait_impl` for tart-agent, api, core, manager (CI's toolchain and flags) | PASS, no warnings |
| Existing tests still pass (full tart-agent, api, core script, manager outage) | PASS, except the pre-existing macOS preflight test |
| Build / server start / API smoke | PASS: `control_plane_contracts` and `fleet_http` spawn the real `gridops-api` binary and exercise it over HTTP |

## Tier 2 results
- **Coverage per criterion:** PASS, see below.
- **Test quality:** PASS. Each new test names one behavior and fails on its feature assertion (RED evidence, and the two mutations above). R4 and R5 assert the same invariant in two binaries, but each pins a different token source (`control_plane_contracts` vs `fleet_http` pick their tokens in separate code), so neither is redundant. The shared helper has no test of its own in the support file (A warn 7); R3 and R6 cover it through the two binaries. Suite is proportionate: 6 tests for 2 flakes plus 1 sibling.
- **Order dependence:** `FakeTart::logged` sorts by mtime then content. I checked every `calls()` and `calls_to()` caller: all use counts, `contains`, or a joined `contains`, so none depends on order and an mtime tie on a coarse-clock filesystem cannot change a result. PASS.
- **Type safety:** no `any`-style casts; test-only Rust, compiles under clippy `-D warnings`. PASS.
- **Error handling:** helper's failure path kills and reaps the child, bounded at 5 attempts, and the error names each origin and cause (R6 asserts the origin appears). PASS.
- **Data integrity:** fixtures seed once before spawn; retries reuse the same database (R3 would hit UNIQUE errors otherwise). PASS.
- **Security:** test fixtures only; per-fixture session token has no `.` (cookie splits on first `.`). No production code changed (`git diff 9e357e0 --stat` shows only `tests/`, a `#[cfg(test)]` module, and `tart_outage_tests.rs`). PASS.
- **Migration safety:** no migration. N/A.
- **Playwright / e2e:** the project has no Playwright suite for this change; no UI surface. N/A.
- **Evidence appendix:** `docs/handoffs/124/evidence.md` already lists the unchanged-source facts the tests rely on (migrate before bind, `connect_database_path` migrates, `auth/me` needs `AuthUser`, the cookie split and 401 on an unknown hash, the `last_seen_at` side effect). I spot-checked `auth.rs:87` and `:101-103` against source; the excerpts are verbatim. PASS.
- **F's results cross-checked:** every count above matches F's handoff (54 / 130 / 41 / 151+1 pre-existing). No discrepancy.

## Test repair made this phase
F reported "Tests that look wrong: None" and one stale comment. I updated the comment in `tart_run_never_writes_the_state_a_held_slot_is_kept_in` (`crates/gridops-tart-agent/src/main.rs`) to say the logs are now `calls` and `paths` directories that the helper skips. Comment only; the suite is still 54/54 and `cargo fmt --check` is clean.

## Acceptance criteria status
- Both named tests pass repeatedly under load: PASS (stress above; F's 100+ and 50 runs).
- Tart race reproduces on the old fixture and not the new: PASS (T-red R1 20/20, R2 20/20, sibling 7/300; new fixture 0 failures; the mutation above re-creates the failure).
- Regression test that a foreign server on the chosen port is detected and the fixture moves: PASS (R3; R6 for restart; R4 and R5 for the unique-token invariant).
- fmt, clippy (CI toolchain), `cargo test -p gridops-tart-agent`, `cargo test -p gridops-api`: PASS apart from the one pre-existing, out-of-scope macOS preflight failure.
- Sweep: tart `calls`/`paths` logs and the `exec-hold` conversion (done), `fleet_http` fixture on the shared helper (done), `gitlab_runner_script.rs:675` and `tart_outage_tests.rs:118` moved to the discard port (done). No timeouts raised, no retry-the-test logic.

## Blocking issues
None.

## Advisory notes
- `release_preflight::the_release_preflight_fails_early_without_a_macos_arm64_machine` fails on macOS hosts (base too); worth its own issue.
- Any API stress run uses the `gridops-api` binary from the shared target dir, so another worktree building it mid-run can skew a loop; none did here.
