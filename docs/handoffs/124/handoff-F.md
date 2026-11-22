# Handoff-F: Phase 6 - Flaky tests: tart-agent slot wait and API auth boundary

**Date:** 2026-11-22
**Branch:** claude/fix-124-flaky-tests
**Issue:** #124

## What was done
This issue changes only test code. T drew the line inside the test files (handoff-T-red.md, "How the work is split"), and F replaced the bodies behind T's seams. No test body was edited.

- `crates/gridops-tart-agent/src/main.rs` (`#[cfg(test)] mod tests` only):
  - `FAKE_TART`: each command now writes each log entry to its own file. Its call goes to `calls/` and its PATH to `paths/`. The entry is written under a `.`-prefixed name and then renamed into place in one step. `dirname` is replaced by `${0%/*}`, which saves a process per command. The header comment now describes the `always-busy` slot flag and the logs.
  - `LOGS` const (`["calls", "paths"]`). `FakeTart::new` creates both directories and, after the warm-up probe, empties them, which forgets the probe's entries.
  - `FakeTart::hold_slot` / `free_slot` set and clear `always-busy` (A's reuse of the existing branch). They no longer write the counted `busy`.
  - New private `FakeTart::logged(log)` reads one log's whole entries, ordered by write time, and skips `.`-prefixed (unfinished) names. `calls()` and `logged_paths()` both read through it.
- `crates/gridops-api/tests/support/api_process.rs` (new): the shared helper (A warn 1). It has one `pub fn start` and keeps everything else private. For each port it is offered it spawns the API through a caller closure, waits for health, checks that the child is alive (`try_wait`) and probes `GET /api/v1/auth/me` with the fixture's own cookie. On a failed attempt it kills and reaps the child, then moves to the next port. It stops after at most 5 attempts, with an error that names every origin tried and why each one failed.
- `crates/gridops-api/tests/control_plane_contracts.rs` (fixture code only): `ApiFixture::start_on` now does the following:
  - it seeds a per-fixture session (`fixture-session-<simple uuid>`) once, before spawning;
  - it calls `api_process::start` with `first_port` followed by free ports (`None`);
  - it removes the database files if the start fails.
  
  `seed_identity` became an associated fn on the database path. A `session_cookie_for` fn and a `remove_database` fn were added, and the `Drop` impl uses `remove_database`. `net::TcpListener` is no longer imported.
- `crates/gridops-api/tests/fleet_http.rs` (fixture code only):
  - `Fixture::start` seeds a per-fixture session (`fleet-http-session-<simple uuid>`) once, before spawning. It then calls `api_process::start` with free ports. If the start fails, the error adds the `api.log` path.
  - `Fixture::restart` offers only its own port (`[Some(port)]`), so it never moves. On failure the error names the origin and `api.log`.
  - `start_api` now returns the `Child`, which callers wrap in `ApiProcess`.
  - `wait_until_healthy` and the `SESSION_TOKEN` const are removed, since nothing uses them any more (T: "F can delete it").
  - `Fixture`'s `cookie: String` field and its `new` / `with_manager` / `with_env` constructors are unchanged (A warn 8, #134).

## Design decisions
- **Seed before spawn, once (A warn 3).** The fixture migrates and seeds its own database (`connect_database_path` migrates), and only then starts the API. Every retry runs on that same database, so nothing is inserted twice, and the identity probe works on the first request. The other option was to seed after the first health check. That would seed while the fixture's child might still be migrating the same file, and the first health answer might come from a foreign server. The API's startup writes nothing that conflicts with these rows: `ensure_control_plane` is `ON CONFLICT DO NOTHING` on another table.
- **Identity probe: `GET /api/v1/auth/me` must answer 200 (A warn 4).** Each fixture's session is unique, so a foreign API answers 401. The helper also checks `try_wait` before and after health, so a child that exited on its bind error fails the attempt quickly. No production change was needed.
- **One helper entry point for both "start" and "restart".** `start(client, ports, health_wait, cookie, spawn)` takes an iterator of `Option<u16>`. `ApiFixture` passes `first_port` followed by `None`s, `Fixture::start` passes only `None`s, and `Fixture::restart` passes `[Some(port)]`. The obvious alternative was two pub functions, `start` and `respawn`. But `control_plane_contracts` never restarts, so `respawn` would be dead code in that binary, and clippy `-D warnings` would fail. An enum mode has the same problem with its unused variants. With one function both binaries use every pub item (A warn 1), and restart cannot move ports because it offers only one.
- **Health budgets are kept per fixture:** `ApiFixture` 3 s and fleet 5 s. That is the old 60 and 100 polls at 50 ms, and the helper still polls 50 ms at a time. No timeout was raised.
- **The fake's logs are one file per entry, renamed into place.** First I used `mktemp` plus `mv`. That raised the cost of one fake command from about 6.5 ms to about 17 ms (100-run benchmark). `provisioning_gives_up_after_the_maximum_wait_and_removes_the_clone`, which needs more than 2 `tart run` retries within 0.3 s, then failed once in a suite run on a loaded machine. The final writer names the temp entry `.<pid>-<n>` and needs no `mktemp`. Only this live process has that PID, so no other command writes those names. A reused PID steps past the names an earlier command left (`while [ -e ... ]`). The final writer costs about 9.5 ms per command. The alternative of locking appends to one shared file was rejected: a killed holder would deadlock the fake.
- **`calls` was hardened as well as `paths` (brief §1 sweep).** `calls` has the same concurrent-append shape: the gitlab `exec -i` line carries a multi-KB script. No test depends on its order, and `calls()` still returns the same lines (multi-line entries are split as before).

## Reuse / extend-vs-new
- Tart: this extends the existing `always-busy` branch of `FAKE_TART` and T's `FakeTart::clear`. No new fake branch was added.
- API: one shared helper in `tests/support/`, included with `#[path]` by both binaries, after the `openapi_schema.rs` precedent. Both fixtures call it, and no second copy exists. The fixture-specific parts stay in each fixture, as A asked: spawn env, log target, seeding, cookie. The new file is the "shared helper" the brief and A asked for.

## Architecture notes for A
Only test code changed: the `#[cfg(test)]` module in tart-agent and `crates/gridops-api/tests/`. There are no new dependencies and no production, schema or contract changes. The new module boundary is `tests/support/api_process.rs`, which has one `pub fn` used by both binaries, as A warn 1 asked. The pattern followed is `tests/support/openapi_schema.rs` plus `#[path]`.

## Deviations from spec / wireframe
- R1's comment, and T's note, assume the logs stay at `$dir/calls` and `$dir/paths`. They now sit at the same paths, but as directories. R1 only snapshots top-level regular files, so its exclusion list still applies and needs no change.
- The brief says "a bounded number of times". The bound is 5 attempts per start, and 1 for a restart.

## Tier 1 self-check (incl. tests now GREEN)
Toolchain and flags as CI (`ci.yml`): `cargo +1.98.1 clippy -p <crate> --all-targets -- -D warnings -A clippy::unused_async_trait_impl`, with `CARGO_BUILD_JOBS=2 RUST_TEST_THREADS=2`.
- `cargo fmt --check -p gridops-api -p gridops-tart-agent`: clean.
- clippy, gridops-api and gridops-tart-agent: no warnings. The default `stable` 1.99 clippy also reports `unused_async_trait_impl` lints in `src/` that are already on base and that CI allows. They are not from this change.
- `cargo test -p gridops-tart-agent`: `test result: ok. 54 passed; 0 failed` (R1 and R2 now pass).
- `cargo test -p gridops-api --no-fail-fast`:
  - unit tests: `ok. 130 passed`;
  - control_plane_contracts: `ok. 41 passed; 0 failed` (R3 and R4 now pass);
  - fleet_http: `151 passed; 1 failed`. R5 and R6 now pass. The one failure is `release_preflight::the_release_preflight_fails_early_without_a_macos_arm64_machine`. It is pre-existing and fails on base `9e357e0` on this macOS host (T documented it). It is unrelated to #124.

Stress runs. Each test binary was copied out of the shared target dir, then run with `RUST_TEST_THREADS=2` on a busy host (load average 6.7 to 12.3, other worktrees building, several loops at once). The script is `scratchpad/f124/stress.sh`, which runs `<binary> --exact <test>` N times and counts non-zero exits. Results on the final code:

| Test (binary) | Runs | Failures |
|---|---|---|
| tart `a_runner_waiting_for_a_slot_is_listed_as_waiting` (named flake) | 100 (plus 50 + 200 on an intermediate build that used the same slot fix) | 0 |
| tart `every_tart_command_gets_a_path_starting_with_tarts_directory` (sibling; base 7 of 300) | 300 (plus 50 on the intermediate build) | 0 |
| tart `provisioning_gives_up_after_the_maximum_wait_and_removes_the_clone` (timing-sensitive, after the logger change) | 100 | 0 |
| tart full suite (54 tests) | 15 | 0 |
| api `api_routes_enforce_authentication_and_admin_boundaries` (named flake) | 50 | 0 |
| api R3 + R4 (`the_fixture_moves_off...`, `a_fixtures_session_is_unknown...`) | 30 | 0 |
| api fleet R5 + R6 (`a_fleet_fixtures_session_is_unknown...`, `a_restart_that_another...`) | 20 | 0 |

Command shape, run from the scratchpad copy: `TMPDIR=<scratch> ./stress.sh ./tart-final 300 --exact tests::every_tart_command_gets_a_path_starting_with_tarts_directory`. The 4 tart loops ran at the same time as each other, at load 6.7 to 10.6. The API loops ran alongside the tart loops, at load 6.7 to 12.3. The API fixture code did not change after those API runs.

The old fixture's failure rates come from T's RED evidence and were not re-measured here:
- R1 failed 20 of 20 runs on the old fixture, and R2 20 of 20;
- the full tart suite on base had 2 failures in 25 runs, both the waiting test;
- the sibling test alone on base had 7 failures in 300 runs.

## Evidence appendix
docs/handoffs/124/evidence.md. It covers:
- the API migrates before it binds;
- `connect_database_path` migrates;
- `/api/v1/auth/me` needs `AuthUser`;
- the cookie is split on the first `.`, and an unknown hash gets 401;
- the probe's `last_seen_at` side effect.

## Tests that look wrong (for T)
None. One comment is now out of date. R1's helper comment says "the logs the fake appends to on every command", but the logs are now directories of entries. R1 itself is still correct, because it skips directories. T may update the wording at Phase 7.

## Known issues
- `fleet_http`'s `release_preflight::the_release_preflight_fails_early_without_a_macos_arm64_machine` fails on macOS hosts, on base too. It is out of scope.
- The API stress runs use the `gridops-api` binary that the tests have compiled in, through `CARGO_BIN_EXE`, and it sits in the shared target dir. Those runs finished before any other build of `gridops-api` in this session.

## Files changed
- crates/gridops-tart-agent/src/main.rs (test module only)
- crates/gridops-api/tests/support/api_process.rs (new)
- crates/gridops-api/tests/control_plane_contracts.rs (fixture code only)
- crates/gridops-api/tests/fleet_http.rs (fixture code only)
