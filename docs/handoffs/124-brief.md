# [Flaky tests: tart-agent slot wait and API auth boundary](https://github.com/Performant-Labs/gridops/issues/124)

Two tests flake on loaded self-hosted CI runners. Make both deterministic by removing the race, not by raising timeouts or adding retries. This is test-fixture work: no production behaviour changes, no UI, no migration.

## 1. `gridops-tart-agent` `tests::a_runner_waiting_for_a_slot_is_listed_as_waiting`

Failure (master CI, e5b39c2, 2026-11-15 9:36 AM PST / 17:36 UTC): `the runner did not start once the slot was free`, after 30 s. Also seen 2/15 locally on an untouched base (`docs/handoffs/142/decisions.md:105`, `handoff-S.md:164`).

**Root cause: a lost update on the `busy` counter in the test fixture** (`crates/gridops-tart-agent/src/main.rs`, `FAKE_TART` ~line 2904, test ~line 3250).

- The fake `tart run` does a read-modify-write of `$dir/busy`: `busy=$(cat busy)`, then `echo $((busy - 1)) > busy`.
- The test starts with `busy = 1000`, waits until the runner is listed as `created` (waiting for a slot), then frees the slot with `tart.flag("busy", "0")` (a plain `fs::write`).
- The agent retries `tart run` every 20-50 ms (`VmSlotWait { first_retry: 20 ms, max_retry: 50 ms }`). If the test's write of `0` lands between a retry's `cat` and its `echo`, the retry writes back e.g. `996`. The slot never frees: at about 50 ms per retry plus a process spawn, 996 more retries take longer than `max_wait` (30 s), so `provision_runner` fails with the no-slot error and the test reports "did not start". A loaded runner widens the window (slower shell between read and write).
- `fs::write` also truncates then writes, so a retry can read an empty file; that one is harmless today (`[ "" -gt 0 ]` is false) but is the same unsynchronised shape.

**Fix (expected shape):** the "busy until the test says otherwise" case must not be a counter that both sides write. Use a presence flag that only the test changes and the fake only reads (the existing `always-busy` file already answers `run` with the limit error; the test can set it, wait for `created`, then remove it; `rm` is atomic). Add a small `FakeTart` helper to clear a flag if needed. Keep the counted `busy` form for tests that only set it before provisioning (`busy = 2` at ~line 3162 has no concurrent writer and is fine). The test's own 20 s listing deadline stays as a safety net only; it is not the thing that flakes.

**Sweep (same class):** check every other `FAKE_TART` branch and every test in this crate that writes a flag file while the agent is running a fake command (`exec-hold`, `exec-status`, `deleted`, `exec-until-deleted`, `slow-clone`, ...) for the same read-modify-write or truncate-then-read race, and fix any you find the same way. `every_tart_command_gets_a_path_starting_with_tarts_directory` is a known sibling flake (same handoff notes, about 2/15); investigate its cause and fix it in this issue if it is a fixture race of the same kind. If it is something else, say what in the handoff.

## 2. `gridops-api` `control_plane_contracts::api_routes_enforce_authentication_and_admin_boundaries`

Failure (PR #123, b911eac): got 401, expected 403 on the member's `GET /api/v1/backups/database` with the seeded session cookie. Passed on rerun.

**Root cause: the fixture can talk to a different API process than its own.** `ApiFixture::start_with` (`crates/gridops-api/tests/control_plane_contracts.rs:53`) picks a port by binding `127.0.0.1:0`, reads the port, **drops the listener**, then spawns `gridops-api` with `GRIDOPS_API_BIND=127.0.0.1:{port}`. Between the drop and the child's bind, the port is free for anyone: another test's fixture in the same binary (tests run in parallel), another test binary, or another CI job on the same self-hosted host. When that happens:

- our child fails to bind and exits (its stdout/stderr go to `/dev/null`, so nothing shows), and
- `wait_until_healthy` gets `200` from the **other** API process, and every request goes there.

Every fixture seeds the identical session (`session-1`, same token, same `SESSION_SECRET`), so the foreign server accepts our cookie if it has already seeded, and answers 401 if it has not yet seeded (its `seed_identity` runs after its own health check). That is exactly 401-instead-of-403. `authenticate_session` (`crates/gridops-api/src/auth.rs:74`) answers 401 only for a missing/bad signature, an unknown token hash or an expired session; database errors are not 401, so a server that holds our seeded session cannot produce this. Nothing in the API deletes sessions at startup.

**Fix (expected shape):** the fixture must prove the server answering is its own child before any test request, and recover if it is not.

- After the health check, verify identity with something only this fixture's database can answer: for example a per-fixture unique session token (or secret) seeded into this fixture's DB, then a request that must authenticate with it; plus `child.try_wait()` showing our child alive.
- If the check fails (child exited, or the answer is not ours), kill/reap our child and start again on a newly chosen port, a bounded number of times, with a clear error that names the cause if it never succeeds.
- Tests then use the fixture's own cookie (an instance method) instead of the shared constant.

**Sweep (same class):** `crates/gridops-api/tests/fleet_http.rs:178` (`Fixture::start` + `start_api` + `wait_until_healthy` at :134) has the identical bind-drop-spawn shape and also seeds a shared `SESSION_TOKEN`; give it the same protection, ideally by sharing one helper rather than two copies. Every `restart`/re-spawn path in those fixtures that reuses an origin needs the same check. `crates/gridops-core/tests/gitlab_runner_script.rs:675` binds and releases a port to get one "nothing listens there"; decide whether a foreign listener there can change a result, and fix it only if it can. In-process listeners that are kept (`tokio::net::TcpListener::bind("127.0.0.1:0")` then served) are fine.

## Not in scope

- No production code changes unless the fix genuinely needs one; if it does, justify it in the handoff.
- No timeout increases and no retry-the-test logic as the fix.

## Acceptance

- Both named tests (and any sibling fixed under the sweep) pass repeatedly under load. Evidence in the handoff: run counts and failures, e.g. 50+ runs of each with `RUST_TEST_THREADS=2` and a busy machine (`CARGO_BUILD_JOBS=2`), with the command used. Show the tart race reproduces on the old fixture (a stress loop or a targeted reproduction) and not on the new one.
- A regression test or assertion where it is cheap: e.g. a fixture-level test that a foreign server on the chosen port is detected and the fixture moves to another port.
- `cargo fmt --check`, `cargo clippy --all-targets -D warnings` for the touched crates, and `cargo test -p gridops-tart-agent` and `cargo test -p gridops-api` green.

## Environment notes

- Keep `CARGO_BUILD_JOBS=2` and `RUST_TEST_THREADS=2`. The cargo target dir is shared through the untracked `.cargo/config.toml` in the main checkout; never set `CARGO_TARGET_DIR`. Prefer `-p <crate>` runs. Disk is 97% full.
- Test-only findings in `*_tests.rs` / `tests/` belong to the Tester, not the Feature Implementor.
