# Handoff-T-followup: #124 follow-up round on PR #205 (two CI flakes)

**Date:** 2026-11-22
**Branch:** claude/fix-124-flaky-tests (worktree `.claude/worktrees/gridops-124`)
**Issue:** #124, PR #205
**Kind:** test-only. Every change is under `crates/gridops-api/tests/`, plus this handoff and `evidence.md`. Nothing is committed or pushed.

## A. `control_plane_contracts` `success_rate_*`: "API fixture did not become healthy"

**Cause (orchestrator's diagnosis, confirmed):** each attempt waited a fixed `health_wait`: 3 s for `control_plane_contracts`, and 5 s for `fleet_http`, both for start and for `Fixture::restart`. When the wait ran out, the helper killed a child that was slow but would have become healthy, then tried another port that was just as slow.

**Fix (`tests/support/api_process.rs`):**
- The `health_wait` parameter is gone. `prove_own` now polls for as long as the child is alive and not yet healthy. A child that cannot bind exits, `try_wait` sees that, and the helper moves to another port at once.
- One private constant, `HANG_GUARD = 60 s`, wraps each whole attempt (health and identity probe) in `tokio::time::timeout`. It is documented as a guard against a hung API, or a port that accepts connections but never answers. It is not the readiness signal. It also covers the case the old code could not: a request that hangs.
- The identity proof is unchanged: the child is alive, and `/api/v1/auth/me` answers 200 with the fixture's own session.
- Callers: `fleet_http.rs` drops `HEALTH_WAIT` from `start` and `restart`, and `control_plane_contracts.rs` drops `Duration::from_secs(3)`.

**New regression test:** `a_fixture_waits_for_its_own_api_however_slow_it_is_to_bind` (control_plane_contracts). The fixture starts the API through `/bin/sh -c 'sleep 4; exec "$0"'`. `exec` keeps the same PID, so this is a live child that binds after 4 s, longer than the old 3 s. The new constructor is `ApiFixture::start_slow_to_bind`, and `start_on` gained an `idle_seconds` argument.
- RED against the old behaviour (`HANG_GUARD` temporarily set to 3 s, which reproduces the old give-up-and-move): `Error: the API fixture got no API of its own after 5 attempts: http://127.0.0.1:63041: no healthy API of the fixture's own within 3s; ...` (5 ports, 15.3 s). Reverted afterwards.
- GREEN on the final code. The existing foreign-server tests still pass unchanged: `the_fixture_moves_off_a_port_another_server_answers_on`, `a_fixtures_session_is_unknown_to_another_fixtures_api`, `a_restart_that_another_server_answers_for_fails_and_names_the_origin` and `a_fleet_fixtures_session_is_unknown_to_another_fleet_fixtures_api`.

## B. `recovery::an_api_restart_leaves_a_running_runner_alone`: "timed out waiting for the journaled inspect answer"

**Cause (confirmed in source, see evidence.md):** `commands.rs:145/161` commits `state='delivered'` before the poll reply leaves the API. A restart in that window loses the reply, and the reconnect's `settle_unreceived` then cancels the inspect.

**Fix (`tests/fleet_http/recovery.rs`):** the test keeps the `delivered` wait and then waits for proof that the agent got the command before it restarts the API. The proof has two parts:
- `runtime.operations` contains `"inspect"`;
- the proxy recorded a `/commands/result` post that was answered 503 and whose body contains the inspect's command id. `issue_command`'s `CommandId` is now kept for this check.

The agent journals the result before it posts it (`dispatch.rs:120`), so this proves the answer is journaled. No timeout was raised.

**Sweep (recovery.rs and every other fleet_http file):**
- `an_administrator_repairs_a_placement_the_agent_cannot_confirm`: **fixed**. Before, it issued a start with a 7 s deadline and slept a fixed 7.5 s. A start still pending at its deadline is `cancelled`, not `uncertain` (`commands.rs:778-780`), so the test failed if the agent had not yet received the start. Now it records `issued` before `issue_start`, waits until `runtime.operations` contains `start_runner`, and then calls `sleep_until(issued + 7.5 s)`.
- `host_recovers_when_a_started_runner_result_is_lost_past_its_deadline`: already correct. It waits for `start_runner` in operations and then sleeps 7 s, which ends after `issue + 7 s`. No change.
- `host_recovers_when_its_agent_gets_a_start_too_late_to_run`: waits for `delivered` and then sleeps 7 s, but it never kills or restarts anything, so the in-flight reply still reaches the agent. The assertion (`uncertain`) depends only on the API-side `delivered` state, and both possible agent paths end in `agent_reported`/`deadline`. No change.
- `host_recovers_when_a_delivered_command_never_reaches_it` (`swallow_command`): it waits on the final state and has no fixed sleep. No change.
- `restart_mid_prepare` waits for `prepare_environment` in operations before the abort, so it is correct. `start_stranded` and `a_machine_back_with_an_undelivered_start...` abort before issuing the command. No change.
- `pool_images.rs` `f.restart` ×2: no agent and no in-flight command. No change. `busy_database.rs` 7.5 s lock hold: it already asserts that a heartbeat failed while the lock was held, so it proves its premise. No change.
- Not the same class, advisory only: `agent.rs:215` (sleeps 700 ms, then expects a newer heartbeat) and `agent.rs:287` (sleeps 300 ms, then expects the log buffer empty). Both assume periodic agent work happens within a fixed sleep. They have not been seen to flake and are outside this brief. They could become follow-up candidates if they ever do.

## Evidence

Every run used `RUST_TEST_THREADS=2`. The test binaries were copied to the scratchpad so that rebuilds could not swap them mid-run. The stress script runs `<bin> [--exact <test>]` N times and counts non-zero exits. Load was produced by 4 to 6 loops running at once on top of the host's usual background load. Load averages are from `uptime`, sampled every 30 to 60 s.

**B before/after, 1 ms-poll variant.** This was a temporary edit, now reverted. The `delivered` wait (and, in the new variant, the proof wait) polled every 1 ms instead of every 100 ms. Both variants were in the same binary.

| Variant | Runs | Failures | Load (1-min) |
|---|---|---|---|
| old wait (`delivered` only) | 60 (2 loops × 30) | **4**, all `Error: timed out waiting for the journaled inspect answer` | 8.0–11.2 |
| new wait (`delivered` + ran + refused result post) | 120 (4 loops × 30) | **0** | 8.0–12.3 |

**Stress on the final code, 2 rounds, all loops of a round running at once:**

| Test | Runs | Failures | Load (1-min) |
|---|---|---|---|
| `recovery::an_api_restart_leaves_a_running_runner_alone` | 80 (4 loops × 20) | 0 | 8.4–11.9 |
| `recovery::an_administrator_repairs_a_placement_the_agent_cannot_confirm` | 40 (2 × 20) | 0 | 8.4–11.9 |
| whole `control_plane_contracts` binary (42 tests, including all 5 `success_rate_*`, `api_routes_enforce_authentication_and_admin_boundaries` and the new slow-bind test) | 30 (2 × 15) | 0 | 8.4–11.9 |

**Tier 1:**
- `cargo fmt --check -p gridops-api`: clean.
- `cargo +1.98.1 clippy -p gridops-api --all-targets -- -D warnings -A clippy::unused_async_trait_impl` (CI's toolchain and flags): clean. Plain `cargo clippy -p gridops-api --all-targets -- -D warnings` on the default stable 1.99 reports 5 `unused_async_trait_impl` errors. All of them are in `src/` (the `gridops-api` bin and the `gridops-host-agent` lib), and none is in `tests/`. They are already on base and CI allows them (same as handoff-F).
- `CARGO_BUILD_JOBS=2 RUST_TEST_THREADS=2 cargo test -p gridops-api --no-fail-fast`: unit tests 130/130; control_plane_contracts 42/42 (41 plus the new test); fleet_http 151 passed, 1 failed. The failure is `release_preflight::the_release_preflight_fails_early_without_a_macos_arm64_machine`, which already fails on base on macOS hosts (T-red and T-green record this). It is unrelated to this change.

## Files changed
- `crates/gridops-api/tests/support/api_process.rs`: removed `health_wait`, wait while the child is alive, added `HANG_GUARD`, updated docs.
- `crates/gridops-api/tests/control_plane_contracts.rs`: changed the caller, added `start_slow_to_bind` and the `idle_seconds` argument to `start_on`, added the new regression test.
- `crates/gridops-api/tests/fleet_http.rs`: removed `HEALTH_WAIT`, and updated the `start` and `restart` callers.
- `crates/gridops-api/tests/fleet_http/recovery.rs`: added the restart test's ran-and-journaled wait, and changed the admin-repair test to wait for the start to run and then `sleep_until` issue + 7.5 s.
- `docs/handoffs/124/evidence.md`: 3 new T entries (delivered is committed before the reply, the agent journals before it posts, a pending command past its deadline is cancelled while a delivered one becomes uncertain).

## Blocking issues
None.

## Could not fix
Nothing in scope. The macOS-only `release_preflight` failure was already there before this change and is out of scope.
