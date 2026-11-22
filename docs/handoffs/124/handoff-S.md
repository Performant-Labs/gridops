# Handoff-S: Phase 8 - Flaky tests: tart-agent slot wait and API auth boundary (#124)

**Date:** 2026-11-22
**Branch:** claude/fix-124-flaky-tests
**Issue:** #124
**Brief (spec):** docs/handoffs/124-brief.md
**Handoff-T reviewed:** docs/handoffs/124/handoff-T-green.md (and handoff-T-red.md)
**Handoff-A reviewed:** docs/handoffs/124/handoff-A.md (plan) and handoff-A-dup.md (anti-duplication gate)
**Handoff-F reviewed:** docs/handoffs/124/handoff-F.md
**Operator-facing report:** none. This is test-fixture work with no rendered surface, so only this markdown handoff is required.

## A precondition
Confirmed: A returned PASS on the plan (0 block, 8 warn) and PASS on the anti-duplication gate (0 block, 2 warn, no action needed).

## T precondition
Confirmed: handoff-T-green reports "Blocking issues: None". Its one failing test, `release_preflight::the_release_preflight_fails_early_without_a_macos_arm64_machine`, is host-specific (see Spec compliance, last row) and fails the same way on base `9e357e0`.

## Visual-diff-tool precondition
N/A. No visual or web surface. No browser or pixel-diff gate applies.

## Visual diff results
N/A.

## What I checked, and how
- Read the brief, all A/T/F handoffs, `decisions.md` and `evidence.md`, then the full diff `9e357e0..HEAD` for `crates/`.
- Confirmed the change is test-only. Every `gridops-tart-agent/src/main.rs` hunk is after line 2389 (`#[cfg(test)] mod tests`). The other five files are `tests/` files and `tart_outage_tests.rs`. `Cargo.toml` and `Cargo.lock` have no diff. No production, schema or contract change.
- Ran my own spot-check on the loaded host (load 7.7 to 9.0), with `CARGO_BUILD_JOBS=2 RUST_TEST_THREADS=2` and CI's pinned toolchain `+1.98.1`:
  - tart-agent: `a_runner_waiting_for_a_slot_is_listed_as_waiting`, `every_tart_command_gets_a_path_starting_with_tarts_directory`, `provisioning_gives_up_after_the_maximum_wait_and_removes_the_clone`, `tart_run_never_writes_the_state_a_held_slot_is_kept_in` and `tart_commands_run_at_once_each_log_their_whole_path`: 5 passed.
  - gridops-api: `api_routes_enforce_authentication_and_admin_boundaries`, `the_fixture_moves_off_a_port_another_server_answers_on`, `a_fixtures_session_is_unknown_to_another_fixtures_api`, `a_fleet_fixtures_session_is_unknown_to_another_fleet_fixtures_api` and `a_restart_that_another_server_answers_for_fails_and_names_the_origin`: 5 passed.
  - This is a once-through confirmation only. The repeated-run stress counts are F's and T's, which I cross-read and did not repeat.
- Linux portability check. CI runs on `self-hosted-linux` (`ci.yml:39,79`), where `/bin/sh` is usually dash, but F and T ran on macOS only. I extracted the new `FAKE_TART` from `main.rs` and ran it under `/bin/dash`: 10 rounds of 8 concurrent `tart list` calls with a 38 KB PATH gave 80 `calls/` entries and 80 `paths/` entries, every PATH entry whole at 38 048 bytes, and no `.`-prefixed leftovers. The new shell (`${0%/*}`, `$((n + 1))`, a `log()` function, `printf "$@"`, `mv` rename) is POSIX and does not rely on bash.
- Verified the one disputed host claim in source: `release_preflight.rs` builds a "Linux machine" that reports the host's real OS, so it can only pass on a non-macOS host. The file is untouched by this diff.
- Swept the repo for the same defect class. The only remaining bind-then-release port picker is the one inside `api_process.rs`, which is now guarded. The other `TcpListener::bind` sites keep and serve their listener (`gitlab_runner_script.rs:578`, `gitlab/http_tests.rs:153,1351`). There is no `portpicker` or `free_port` helper anywhere.

## Spec compliance

| # | Brief requirement | Where | Compliant | Notes |
|---|---|---|---|---|
| 1 | Tart: the busy-until-the-test-says case must not be a counter both sides write; use a presence flag only the test changes | `FakeTart::hold_slot` / `free_slot` set and clear `always-busy` (`main.rs:3105-3113`); the fake only tests `[ -f ]` | YES | The `run` branch still carries the old counted `busy`, kept for tests that set it before provisioning. `rm` is atomic and `fs::write` creates the file atomically, so no retry can undo the free. |
| 2 | Keep the counted `busy` for `busy = 2` (no concurrent writer) | `provisioning_waits_out_a_busy_slot_and_starts_the_runner` still uses `flag("busy","2")` (`:3291`) | YES | The fake's header comment now says which form is for which case. |
| 3 | Add a small `FakeTart` helper to clear a flag | `FakeTart::clear` (`:3095`); the inline `remove_file(exec-hold)` converted to it (`:3832`) | YES | One way to clear a flag now. |
| 4 | The test's own 20 s listing deadline stays as a safety net | `a_runner_waiting_for_a_slot_is_listed_as_waiting`: only `hold_slot`/`free_slot` lines changed | YES | No timeout changed anywhere in the diff. |
| 5 | Tart sweep: every `FAKE_TART` branch and every test that writes a flag while the agent runs a fake command | See "Sweep confirmation" below | YES | F and T documented the logs; I checked the flag-writing tests myself. |
| 6 | `every_tart_command_gets_a_path_starting_with_tarts_directory`: find the cause; fix it if it is a fixture race; otherwise say what | handoff-T-red "Sweep decisions"; fix in `FAKE_TART` (`log()` writes one file per entry and renames it into place); the test reads `logged_paths()` and waits with `wait_for_stops(&tart, 2)` | YES | Cause: concurrent fake commands append multi-`write()` PATH lines to one shared file and interleave (7 of 300 on base, all spliced lines). A's lead (reading while the second stop is still writing) was a real but smaller risk (65 of 35 252 partial reads); both are closed. |
| 7 | API: after the health check, prove the server answering is the fixture's own child | `api_process::prove_own`: `try_wait` before and after health, then `GET /api/v1/auth/me` with the fixture's own cookie must be 200 | YES | A per-fixture token (`fixture-session-<uuid>` / `fleet-http-session-<uuid>`), seeded into the fixture's own DB. No production route added. |
| 8 | On failure: kill and reap our child, retry on a new port, bounded, clear error naming the cause | `start()`: `kill` + `wait`, `ATTEMPTS = 5`, `bail!` lists every origin tried and why | YES | A restart offers one fixed port and cannot move (A warn 2). |
| 9 | Seed-once on retry (A warn 3) | Both fixtures seed before the first spawn; every attempt reuses that DB | YES | R3 would hit UNIQUE errors otherwise. |
| 10 | Tests use the fixture's own cookie via an instance method | `ApiFixture::session_cookie(&self)`; the 15 callers converted; fleet keeps its `cookie: String` field | YES | Fleet constructors and field names are unchanged, as A warn 8 (#134 coexistence) required. |
| 11 | `fleet_http.rs` shares one helper | `api_process.rs`, included with `#[path]` by both binaries; old `wait_until_healthy` and `SESSION_TOKEN` removed | YES | One `pub fn`, used by both, no `dead_code`, no blanket allow (A warn 1). |
| 12 | Every restart/re-spawn path that reuses an origin gets the check | `Fixture::restart` goes through `api_process::start` with `[Some(port)]` and fails naming the origin and `api.log` | YES | `ApiFixture` never restarts. |
| 13 | `gitlab_runner_script.rs:675`: decide whether a foreign listener changes a result; fix only if it can | Now the discard port `9` (`:674-677`) | YES | A foreign listener there turns "unreachable" into an HTTP answer, so a fix was warranted. Reuses the repo's `127.0.0.1:9` convention. |
| 14 | A warn 6 addition: `tart_outage_tests.rs refused_url()` | Now `http://127.0.0.1:9/` | YES | Same decision rule, same approach as row 13. |
| 15 | A warn 5: the secret-free-logs assertion must still check what the fixture sends | `fleet_http.rs:~533-539` now parses the token out of `f.cookie` and asserts it is absent from `api.log` | YES | It would have passed vacuously on the old constant. |
| 16 | No production changes, no timeout increases, no retry-the-test logic | Diff is test code only; no timeout raised; the health budgets are the old 3 s and 5 s (60 and 100 polls of 50 ms) | YES | The 5-attempt bound is the brief's "bounded number of times", not a retry of the test. |
| 17 | Regression test: foreign server on the chosen port is detected and the fixture moves | R3 `the_fixture_moves_off_a_port_another_server_answers_on` (one binary only, A warn 7); R6 for restart | YES | R3 failed on the old seam ("the fixture kept the port another server answers on"). |
| 18 | Evidence: 50+ runs of each, load, command, and the race reproduces on the old fixture and not the new | F: waiting test 100 runs / 0 failures, API named test 50 / 0, sibling 300 / 0. T-green independently: 40 / 40 / 30, all 0. Old fixture (T-red): R1 20 of 20 fail, R2 20 of 20, sibling 7 of 300, full tart suite 2 of 25, shell lost-update repro 1 of 40, R3 fails | YES | Command shape and load average are in handoff-F. |
| 19 | fmt, clippy `-D warnings`, `cargo test -p gridops-tart-agent`, `cargo test -p gridops-api` green | handoff-T-green Tier 1: fmt clean, clippy clean on CI's toolchain and flags for 4 crates, 54 passed for tart-agent, api 130 / 41 / 151 passed | YES, with one pre-existing exception | `release_preflight::the_release_preflight_fails_early_without_a_macos_arm64_machine` fails on macOS hosts on base too and passes on Linux CI by construction (I read the test). It is out of scope. |

### Sweep confirmation (row 5), which F and T did not spell out branch by branch
I read every `FAKE_TART` branch and every `flag()` call site:
- `exec-status`, `slow-clone`, `broken`, `busy = 2`, `ps-output`, `ps-broken` and the first `list.json` write are all written before the agent starts a fake command. There is no concurrent writer, so truncate-then-read cannot occur.
- `list.json` at `:3804`, `:3901` and `:3947` is written after `wait_for_stops`, or before any `list` runs.
- `exec-hold`, `exec-until-deleted` and `deleted` are presence-only (`[ -f ]`, `touch`, `rm`), which is atomic.
- `stops-done` is a short single-write append read by count.
- `records-at-stop` is a `cat >>` of small files and is read only after `stops-done`.
- The remaining shapes are `pid` (`echo $$ >` then `cat` in `stop`) and `home/records` (`cat` in `stop`). No test result depends on them, so I am not asking for a change.

## Quality audit

| Area | Result | Notes |
|------|--------|-------|
| API consistency | N/A | No endpoint added or changed. The identity probe reuses the existing `GET /api/v1/auth/me`. |
| Error handling | PASS | A failed attempt kills and reaps its child. The final error names every origin tried and why. `ApiFixture` removes its database files on a failed start. R6 pins the restart error naming the origin. |
| UI/UX match to spec | N/A | No UI. |
| Accessibility | N/A | No UI. |
| Architecture gate | PASS | A: PASS on the plan, PASS on anti-duplication. I found nothing that conflicts with either handoff. |
| Code organization | PASS | One narrow shared helper after the `openapi_schema.rs` precedent, one `pub fn`. The fixture-specific spawn, seed and cookie code stays in each fixture. `FAKE_TART`'s header comment is updated and explains the why. No dead code, no TODO, no `println!`. |
| Security | PASS | Tokens are per-fixture test values with no `.` (the cookie splits on the first `.`). No secret is logged. No production auth change. |
| Performance | PASS, with an advisory | The fake's cost per command rose from about 6.5 ms to about 9.5 ms (two `mv` execs where there was one `dirname`). See advisory 1. |
| Visual regression | N/A | No visual surface. |
| Naming consistency | PASS | Names match the brief's vocabulary (`hold_slot`, `free_slot`, `clear`, `api_process`). New test names state one behaviour each. |
| Test quality (`testing/test-quality.md` §7) | PASS | See below. |

### Test-quality audit (§7), per new test
| Test | Names one behaviour | Fails alone, for the right reason | Tier / non-duplicative | Behavioural |
|---|---|---|---|---|
| R1 `tart_run_never_writes_the_state_a_held_slot_is_kept_in` (`main.rs:3433`) | Yes: a retried `tart run` never writes the state that holds the slot | Yes. T's mutation back to the counted `busy` file fails it (`"998\n"` against `"1000"`), and it failed 20 of 20 on the old fixture. It is the deterministic form of a race, so it does not depend on timing. | Unit, cheapest tier. The named flake test is probabilistic, so this is its only deterministic guard. | Yes: asserts the files a test set are untouched by `run`. It does not assert the fake's script text. |
| R2 `tart_commands_run_at_once_each_log_their_whole_path` (`:3250`) | Yes: concurrent fake commands each log a whole entry | Yes. 20 of 20 on the old fixture, and it cannot fail on a per-entry atomic log. It is deterministic in effect: 8 concurrent writers, 25 rounds and a 34 KB PATH make the interleave show every time. | Unit. It is the pinned cause of the sibling flake, which only fails about 2 to 3% of the time, so it does not duplicate it: one asserts the logging contract, the other asserts the agent's PATH. | Yes |
| R3 `the_fixture_moves_off_a_port_another_server_answers_on` | Yes | Yes. It failed on the old seam. T's mutation (probe off) also fails it. | Integration, needs the real binary; the only test that runs the retry path, which A warn 7 required. | Yes: the observable result is a different origin that answers 200 for the fixture's own session. |
| R4 `a_fixtures_session_is_unknown_to_another_fixtures_api` | Yes: another fixture's API rejects this fixture's session | Yes (200 against 401 on the old shared token) | Integration with two real processes. A cheaper tier cannot see a token in a running server's database. | Yes |
| R5 `a_fleet_fixtures_session_is_unknown_to_another_fleet_fixtures_api` | Yes (same invariant, other binary) | Yes | Integration | Yes |
| R6 `a_restart_that_another_server_answers_for_fails_and_names_the_origin` | Yes | Yes. T's probe-off mutation fails it ("the restart accepted another server"). | Integration. Without it the restart check never runs. | Yes |

- **Proportionality:** 6 new tests for 2 named flakes, 1 sibling flake and 1 restart path. R4 and R5 pin the same invariant in two binaries, but each binary picks its token in its own code, so deleting either would leave one fixture's token unguarded. I am not asking for a merge.
- **§6 smells:** none. There are no assertion-free or tautological tests, no mock-shaped tests and no snapshots. The assertions are on observable results (statuses, origins, entry counts, file contents).
- **No "delete or merge" findings.**
- **Changed existing tests:** the `exec-hold` conversion and the `session_cookie()` caller edits are mechanical. The sibling test now waits on the existing `wait_for_stops`, which is a stronger seam than polling `calls_to("stop")`.

## Scope check
Delivered exactly the brief's scope plus the two sweep items A added:
- **Named flakes:** both fixed at root cause (a presence flag, and proof of server identity with retry).
- **Sibling flake:** cause found and documented, fixture fixed.
- **Sweep:** `calls` hardened with the same per-entry mechanism; `fleet_http` on the shared helper; `gitlab_runner_script.rs` and `tart_outage_tests.rs` moved to the discard port.
- **No over-delivery:** nothing outside the touched fixtures. No production, dependency, config or contract change. #134's `cookie`, `new`, `with_manager` and `with_env` names are intact.
- **No under-delivery:** every Acceptance bullet is met, and the regression test the brief called "where cheap" is delivered and treated as required.

## Verdict

PASS. All acceptance criteria are met, the work is spec-compliant and the quality is acceptable. Ready for O to commit.

## Advisory notes
Non-blocking.
1. **Timing-sensitive neighbour.** `provisioning_gives_up_after_the_maximum_wait_and_removes_the_clone` has a 0.3 s window and asserts more than 2 `tart run` retries. The new logger adds about 3 ms to each fake command. F measured 100 of 100 passes under load 6.7 to 12.3 and got that figure back down from 17 ms to 9.5 ms by dropping `mktemp`. On a much slower runner this test has less headroom than before. If it ever flakes on `self-hosted-linux`, widen its wait or relax the `> 2` bound in a new issue. That would be a test-body decision, and the brief forbids it as the fix for #124.
2. **Linux has not run this change.** F and T ran on macOS. I checked the fake under dash (see above). The PR's CI run on `self-hosted-linux` is the first real run of the full suite there. The one macOS-only failure should pass there.
3. **Pre-existing, out of scope:** `release_preflight::the_release_preflight_fails_early_without_a_macos_arm64_machine` fails on macOS hosts on base too. T already suggested its own issue.
4. **Small cleanups, optional:**
   - R1's `["tart", "ps", "calls", "paths"]` exclusion is now partly redundant, because `calls` and `paths` are directories and are skipped by the `is_file()` check. It is harmless and the comment says so.
   - The `bail!` in `api_process::start` reads "after 1 attempts" for a restart.
   - `fleet_http`'s `Fixture::start` leaves its temp directory behind if the start fails. This is pre-existing, since the old health failure did the same.
5. **The API stress loops use the `gridops-api` binary from the shared target dir.** Another worktree building it mid-run could skew a loop. T and F both noted this, and neither run was affected.
