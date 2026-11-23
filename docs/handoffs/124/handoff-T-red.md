# Handoff-T-red: Phase 5 - Flaky tests: tart-agent slot wait and API auth boundary (#124)

**Date:** 2026-11-22
**Branch:** claude/fix-124-flaky-tests
**Brief / wireframe reviewed:** docs/handoffs/124-brief.md, docs/handoffs/124/handoff-A.md (no wireframe: test-fixture work, no UI)

## A precondition
Confirmed: A returned PASS on the plan, with 0 blocks and 8 warns (docs/handoffs/124/handoff-A.md). Warns 2, 3 and 7 are part of the plan, and the tests below pin them.

## How the work is split (read this first, F)
This issue changes only test code, so the usual "T writes tests, F writes production code" split needs a line drawn inside the test files:

- **T (this phase, done):**
  - every test body;
  - the regression tests;
  - the sweep sites that are test-body edits;
  - thin fixture *seams*. A seam keeps today's behaviour, so each RED test compiles and fails on its assertion, not on a missing item.
- **F (next):** the fixture *internals* behind those seams: `FAKE_TART`, the bodies of the `FakeTart` seams, the shared API helper and the `ApiFixture` / fleet `Fixture` start, restart and seed code. F should not need to edit any test body. If F finds one that needs changing, flag it for T under "Tests that look wrong".

Seams added with today's behaviour (F replaces the bodies, and keeps the signatures):

| Seam | File | Today's body (the defect) | What F makes it do |
|---|---|---|---|
| `FakeTart::hold_slot()` / `free_slot()` | `crates/gridops-tart-agent/src/main.rs` (FakeTart impl) | write the counted `busy` file, `1000` then `0`; `tart run` rewrites it | hold through a flag that only the test changes (A: the existing `always-busy` branch); free with `clear` |
| `FakeTart::clear(name)` | same | already final: `fs::remove_file`. Used by the `exec-hold` test (A's request to convert `main.rs:3646`) | nothing; `free_slot` can use it |
| `FakeTart::logged_paths()` | same | parses the shared `$dir/paths` file that every fake command appends to | read whatever race-free log F gives `FAKE_TART` (see "Sibling flake" below) |
| `ApiFixture::start_trying_port_first(port)` -> `start_on(Option<u16>, env)` | `crates/gridops-api/tests/control_plane_contracts.rs` | uses the port as given; no identity check, no retry | the shared helper (A warn 1) tries `port` first, detects a foreign server, and moves |
| `ApiFixture.session_token` field + `session_cookie(&self)` | same | the field holds the old shared literal; all 15 callers now use the instance method | a per-fixture unique token (it must contain no `.`: `auth.rs` splits the cookie on the first `.`) |

Fleet `Fixture` keeps `cookie: String`, `new`, `with_manager` and `with_env` unchanged (A warn 8, #134). Only the value behind `cookie` changes.

## Tests authored

| # | Test | File | Pins | Tier, and why |
|---|---|---|---|---|
| R1 | `tart_run_never_writes_the_state_a_held_slot_is_kept_in` | tart-agent `main.rs` tests | Brief §1 root cause. While `hold_slot` holds the slot, two `tart run` calls answer with the limit error and leave every test-set file byte-identical. After `free_slot`, the next `run` gets past the slot check (with `broken` set it fails on "disk image is corrupted" instead of starting a VM). | Unit (crate test module). It runs the fake directly, with no agent. This is the cheapest deterministic form of the lost update: the counter changes on every retry, so a test that asserts nothing writes it fails every time. A timing race would not. |
| R2 | `tart_commands_run_at_once_each_log_their_whole_path` | same | The sibling flake's cause (below). 25 rounds of 8 concurrent fake `tart list` calls with a ~34 KB PATH. Every logged entry must be exactly `("list", PATH)`, one per command. | Unit. Direct fake calls. A long PATH plus 8 writers makes the interleave show up every time on the old fixture (20/20 measured). |
| R3 | `the_fixture_moves_off_a_port_another_server_answers_on` | `control_plane_contracts.rs` | Brief §2 fix, plus A warns 3 and 7: the required regression test. An in-process server holds the port, answers 200 on `/api/health` and 401 elsewhere. `ApiFixture::start_trying_port_first(taken)` must end on a different port, and its own `GET /api/v1/auth/me` with its own cookie must return 200. This also covers "seed once": re-seeding on retry would hit UNIQUE errors and fail the test. | Integration. It needs the real `gridops-api` binary that the fixture spawns. It lives in one binary only (A warn 7). |
| R4 | `a_fixtures_session_is_unknown_to_another_fixtures_api` | same | Brief §2 identity proof. Another fixture's API (the likeliest process to steal a port) answers 401 to this fixture's cookie, and its own cookie gets 200. If tokens are shared, the identity probe cannot tell a foreign fixture from the fixture's own API. | Integration (two real API processes). No cheaper tier sees the token in a real server's database. |
| R5 | `a_fleet_fixtures_session_is_unknown_to_another_fleet_fixtures_api` | `fleet_http.rs` | The same invariant for the sweep's second fixture. The fleet token is chosen by fleet-specific code, which the shared helper does not cover. | Integration. |
| R6 | `a_restart_that_another_server_answers_for_fails_and_names_the_origin` | `fleet_http.rs` | A warn 2. The test kills the fleet API, a foreign server takes its origin, then `f.restart(ZERO)` runs. The restart must return `Err` and the error must name the origin. It must not move ports or carry on against the foreign server. | Integration. Without this test the restart check never runs. |

Existing tests changed (test bodies, mine; they compile and pass now):
- `a_runner_waiting_for_a_slot_is_listed_as_waiting`: `flag("busy","1000")`/`flag("busy","0")` becomes `hold_slot()`/`free_slot()`. Its 20 s listing deadline is unchanged.
- `every_tart_command_gets_a_path_starting_with_tarts_directory`: reads entries through `logged_paths()`. It now waits with the existing `wait_for_stops(&tart, 2)`, which is written at the end of `stop`, instead of polling `calls_to("stop")`, which is written at the start. This is A's lead, applied as a cheap guard (see below).
- `a_gitlab_runner_whose_exit_code_cannot_be_written_is_still_stopped`: inline `fs::remove_file(exec-hold)` becomes `tart.clear("exec-hold")` (A).
- `enrollment_is_one_time_with_metadata_only_retries_and_secret_free_logs` (`fleet_http.rs:~526`): `!logs.contains(SESSION_TOKEN)` now checks the token parsed from `f.cookie`. It keeps checking the token the fixture actually sends after F makes it per-fixture (A warn 5). Once F stops using the `SESSION_TOKEN` const, F can delete it (dead_code).
- 15 `ApiFixture::session_cookie()` / `Self::session_cookie()` callers became `fixture.session_cookie()` / `self.session_cookie()`. None is in #134's `:660-700` range.

## Sweep decisions (brief §1, §2; A warn 6), all test-body edits done here
- **`every_tart_command_gets_a_path_starting_with_tarts_directory`: confirmed the same class (a fixture race), but not A's exact mechanism.**
  - On the untouched base binary, 300 runs of this test alone under load had 7 failures. Base binary copied before any edit; two loops of 150 in parallel plus a full-suite loop for load.
  - All 7 were mixed `paths` lines. 6 were `run lost "/var/run/com.apple.security.cryptexd/..."` or `exec lost ...`; the logged line shows a `run\t<PATH>` entry spliced into the middle of an `exec` entry. 1 was `no PATH logged`.
  - Cause: the fake runs concurrently. `tart run` stays up in the background while the agent polls with `tart exec`. Each command appends `printf '%s\t%s\n' "$1" "$PATH"` to the one shared `$dir/paths`. With this host's 2043-byte PATH, the shell writes that line in more than one `write()`, so the pieces of two concurrent commands interleave.
  - A's lead (reading `paths` while the second stop is still writing it) is a real, smaller risk. A scratch reader saw a partial last line in 65 of 35 252 reads. The switch to `wait_for_stops` guards it, but it did not cause the observed failures.
  - The fix F owns: each fake command must log its PATH entry atomically and separately. For example, write each entry to its own temp file and `mv` it into a log directory. `logged_paths()` then reads that directory, and `FakeTart::new`'s warm-up cleanup forgets the probe's entry.
- **`crates/gridops-core/tests/gitlab_runner_script.rs:675`: fixed.** A foreign listener there turns "unreachable" into an HTTP answer, which changes the script's exit and the scenario's result. It now uses the discard port `9`, the repo's existing convention. That port is privileged, so no unprivileged test process can bind it. All 12 tests pass.
- **`crates/gridops-manager/src/tart_outage_tests.rs:118` `refused_url()`: fixed the same way** (`http://127.0.0.1:9/`), because a foreign listener turns "refused" into an answer for its 3 callers. All 12 `tart_outage` tests pass (with a short `TMPDIR`, see below).

## RED confirmation
Lint: `cargo +1.98.1 clippy -p <crate> --all-targets -- -D warnings -A clippy::unused_async_trait_impl` reports nothing for gridops-tart-agent, gridops-api, gridops-manager and gridops-core. That is CI's pinned toolchain and its flags (`.github/workflows/ci.yml:93,113`). `cargo fmt` was applied to all four.
- The default `stable` clippy (1.99) also reports 4 pre-existing lints in tart-agent: `assert_is_empty` and `unused_async_trait_impl`. They are on base too and not ours.

`CARGO_BUILD_JOBS=2 RUST_TEST_THREADS=2 cargo test -p gridops-tart-agent --no-fail-fast`:
```
test tests::tart_run_never_writes_the_state_a_held_slot_is_kept_in ... FAILED
test tests::tart_commands_run_at_once_each_log_their_whole_path ... FAILED
---- tests::tart_run_never_writes_the_state_a_held_slot_is_kept_in stdout ----
assertion `left == right` failed: a retried `tart run` changed the state the test holds the slot with
  left: {"broken": "", "busy": "998\n"}
 right: {"broken": "", "busy": "1000"}
---- tests::tart_commands_run_at_once_each_log_their_whole_path stdout ----
Error: no PATH logged (… bytes): "st/0068/xxxxxxxx…"      (an entry split by another command's write)
test result: FAILED. 52 passed; 2 failed
```
Both fail for the right reason: the counter written back in one, interleaved entries in the other. Reliability on the old fixture: R1 failed 20 of 20 runs and R2 failed 20 of 20.

`CARGO_BUILD_JOBS=2 RUST_TEST_THREADS=2 cargo test -p gridops-api --no-fail-fast`:
```
unittests: test result: ok. 130 passed; 0 failed
control_plane_contracts:
thread 'a_fixtures_session_is_unknown_to_another_fixtures_api' panicked at control_plane_contracts.rs:292:5:
assertion `left == right` failed: another fixture's API accepted this fixture's session
  left: 200
 right: 401
thread 'the_fixture_moves_off_a_port_another_server_answers_on' panicked at control_plane_contracts.rs:256:5:
assertion `left != right` failed: the fixture kept the port another server answers on
  left: "http://127.0.0.1:52610"
 right: "http://127.0.0.1:52610"
test result: FAILED. 39 passed; 2 failed
fleet_http:
test a_restart_that_another_server_answers_for_fails_and_names_the_origin ... FAILED
Error: the restart accepted another server on http://127.0.0.1:52658
thread 'a_fleet_fixtures_session_is_unknown_to_another_fleet_fixtures_api' panicked at fleet_http.rs:316:5:
assertion `left == right` failed: another fixture's API accepted this fixture's session
  left: 200
 right: 401
test release_preflight::the_release_preflight_fails_early_without_a_macos_arm64_machine ... FAILED   (pre-existing, see below)
test result: FAILED. 149 passed; 3 failed
```
Each new test fails on its feature assertion. None fails on setup, a missing item or a compile error. Every other test in both crates passes, except one fleet_http test that also fails on the untouched base on this host:
- `release_preflight::the_release_preflight_fails_early_without_a_macos_arm64_machine` fails on base `9e357e0` with my changes stashed. The output is `PASS  macos-machine: macOS ARM64 machine ready ... fleet-host`.
- Its "Linux machine" reports this Mac's own OS, so the test only works on a non-macOS host. It is unrelated to #124 and not in scope, so I did not change it.

Environment notes for F and T-green:
- gridops-manager's `tart_outage` tests need a short `TMPDIR`. The default macOS `TMPDIR` is longer than `SUN_LEN` allows for their Unix socket, and that fails 8 tests on base too. I used `TMPDIR=$HOME/.cache/t124`.

Base flake evidence, untouched base `9e357e0`, `RUST_TEST_THREADS=2`, load average about 6.7:
- full tart-agent suite: 25 runs, 2 failures, both `a_runner_waiting_for_a_slot_is_listed_as_waiting` ("the runner did not start once the slot was free");
- the sibling test alone: 300 runs, 7 failures (above);
- a shell reproduction of the `run` branch's read-modify-write against a test-style `printf 0 > busy` showed the lost update (`busy=29` after the slot was freed) in 1 of 40 trials.

Scratch artefacts are under the session scratchpad (`race/`, `partial/`, `base-full/`, `sib-a/`, `sib-b/`, `red-*.log`), outside the repo.

## Ready for F
RED is valid. All 6 new tests fail on their feature assertions, and everything else is green apart from the one pre-existing host-specific test above. F may implement against these tests. Constraints for F, from A and from these tests:
- **R1**: `tart run` must not write any top-level regular file in the fake's directory while the slot is held. R1 snapshots every top-level file except `tart`, `ps`, `calls` and `paths`. If F moves the PATH log, keep it at `$dir/paths` (a directory is fine) or say so, so T can update R1's exclusion.
- **R2 and the sibling**: change `FAKE_TART`'s logging and `logged_paths()`, not the tests.
- **R3**: seed once per fixture (A warn 3). Probe `GET /api/v1/auth/me` with the fixture's own cookie (A warn 4). The helper goes in `tests/support/<name>.rs`, included with `#[path]` by both binaries, and every `pub` item must be used by both (A warn 1). Do not put a test in the support file (A warn 7).
- **R6**: `restart` keeps its origin and never moves ports. On failure it returns an error that names the origin (A warn 2).
- Update the `FAKE_TART` header comment (`main.rs` about line 2898) to match the new slot flag.
