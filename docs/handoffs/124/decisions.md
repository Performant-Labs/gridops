## Phase 3 Architecture (A, up-front plan review) - 2026-11-22T10:34:28-08:00
- **Decided:** PASS, with 0 block and 8 warn findings (docs/handoffs/124/handoff-A.md).
  - The Tart fix reuses `FAKE_TART`'s existing `always-busy` branch, and the counted `busy=2` case stays as it is. A `FakeTart::clear` helper replaces the one inline `fs::remove_file` at `main.rs:3646`.
  - The two API fixtures share one helper in `crates/gridops-api/tests/support/<name>.rs`, included with `#[path]`. That is the `openapi_schema.rs` precedent.
  - The warns:
    - (1) keep the helper narrow: port, health, liveness, probe and retry. Fixture-specific spawn and seeding stay in each fixture. Every `pub` item must be used by both binaries, or `dead_code` fails clippy `-D warnings`;
    - (2) `Fixture::restart` must keep its origin, so it verifies and fails clearly but never moves port;
    - (3) seed once per fixture, or use a fresh database per attempt. The seeds are fixed-id `INSERT`s;
    - (4) probe `GET /api/v1/auth/me` with a per-fixture token; no production change;
    - (5) `fleet_http.rs:476`'s `!logs.contains(SESSION_TOKEN)` would pass without checking anything, so retarget it;
    - (6) the sweep misses `gridops-manager/src/tart_outage_tests.rs:118` `refused_url()`; `http://127.0.0.1:9` is the existing "nothing listens" convention;
    - (7) put the regression test in one binary, not in the support file, and treat it as required;
    - (8) #134 adds `fleet_http/placement.rs` tests using `Fixture::new()` and `&f.cookie`, so keep those names.
- **Assumed:** with no `survey.md`, the analogues the brief names serve as the Reuse map. #134's unmerged branch shows where that work is heading.
- **Hedged:** the lead for `every_tart_command_gets_a_path_starting_with_tarts_directory` is unconfirmed. It is that `calls_to("stop")` counts a stop at its start, before that stop's `paths` line is written, while `wait_for_stops` counts at its end. A does not diagnose runtime causes; T confirms it or rules it out.
- **Evidence:**
  - Read: the brief; `docs/handoffs/142/decisions.md:95-115` and `handoff-S.md:155-175`; tart-agent `main.rs:2880-3320, :3500-3660, :3790-3844`; `gitlab_tests.rs:300-370`.
  - API tests: `control_plane_contracts.rs:1-260, :296-330, :485-600, :2180-2260`; `fleet_http.rs:1-480`; `fleet_http/recovery.rs:740-800`; `tests/support/openapi_schema.rs`.
  - API source: `main.rs:79-100, :185-205, :272-276`; `auth.rs:60-170`. Core: `db.rs:26-50`; `crypto.rs:41-110`; `tests/gitlab_runner_script.rs:440-460, :570-720`. Manager: `tart_outage_tests.rs:105-140, :250-262, :570-580`.
  - Searched every crate for `TcpListener::bind` without `tokio::net`, for `127.0.0.1:9`, and for `SESSION_TOKEN`/`session_cookie` uses.
  - Coexistence: `git worktree list`; the diffs of #134, #143, #183, #126, #130 and #152 against their merge-bases for the touched files; the #126, #130 and #152 briefs. #183 is already merged.
  - `git diff origin/master HEAD -- crates scripts src tests .github` is empty.

## Phase 5 Tests RED (T) - 2026-11-22
- **Decided:** RED valid; 6 new tests, each failing on its feature assertion (docs/handoffs/124/handoff-T-red.md).
  - Tart: `tart_run_never_writes_the_state_a_held_slot_is_kept_in` (the lost update, deterministic: the counter changes on every retry) and `tart_commands_run_at_once_each_log_their_whole_path` (the sibling's cause). Both failed 20 of 20 runs on the old fixture.
  - API: `the_fixture_moves_off_a_port_another_server_answers_on` (required by A warn 7, one binary), `a_fixtures_session_is_unknown_to_another_fixtures_api`, `a_fleet_fixtures_session_is_unknown_to_another_fleet_fixtures_api`, `a_restart_that_another_server_answers_for_fails_and_names_the_origin` (A warn 2).
  - **Split inside test-only work:** T owns every test body. T added thin seams that keep today's behaviour (`FakeTart::hold_slot/free_slot/logged_paths/clear`, `ApiFixture::start_trying_port_first`/`start_on`, `ApiFixture.session_token` + `session_cookie(&self)`), so the tests compile and fail on assertions. F owns the fixture internals behind them and should not need to touch a test body.
  - Sibling `every_tart_command_gets_a_path_starting_with_tarts_directory`: **confirmed a fixture race, but not A's mechanism.** Concurrent fake commands (background `tart run` plus polling `tart exec`) append multi-`write()` PATH lines to one shared `paths` file and interleave. 7 of 300 runs on base failed this way. The test now waits with `wait_for_stops(&tart, 2)` (A's lead, a smaller real risk) and reads through `logged_paths()`.
  - Sweep: `gitlab_runner_script.rs:675` and `tart_outage_tests.rs:118` `refused_url()` can change a result under a foreign listener, so both now use the discard port 9 (the repo convention). The fleet log assertion now checks the token parsed from `f.cookie` (A warn 5). 15 `session_cookie` callers now use the instance method. #134's `cookie`/`new`/`with_*` names are untouched.
- **Assumed:**
  - Port 9 refuses on CI hosts as it does here (curl exit 7). The repo already relies on it at 5 sites.
  - CI's clippy is 1.98.1 with `-A clippy::unused_async_trait_impl` (`ci.yml:93,113`). Lint was judged with that toolchain.
- **Hedged:**
  - R2 is probabilistic by nature (an interleave needs overlap). With 8 concurrent writers × 25 rounds × a 34 KB PATH it failed 20/20 on the old fixture, and it cannot fail on a per-entry atomic log.
  - `release_preflight::the_release_preflight_fails_early_without_a_macos_arm64_machine` fails on base on this macOS host (the fake machine reports the real OS). It is pre-existing, out of scope and untouched.
  - The manager `tart_outage` tests need a short `TMPDIR` on macOS (`SUN_LEN`). That is pre-existing.
- **Evidence:**
  - Base `9e357e0`, `RUST_TEST_THREADS=2`, load about 6.7: the full tart suite had 2 of 25 waiting-test failures; the sibling alone had 7 of 300; a shell lost-update repro hit 1 of 40; partial-tail reads were 65 of 35 252.
  - RED runs: `cargo test -p gridops-tart-agent` gave 52 passed and 2 failed (R1, R2). `cargo test -p gridops-api` gave unit tests 130 passed; control_plane_contracts 39 passed and 2 failed (R3, R4); fleet_http 149 passed and 3 failed (R5, R6, plus the pre-existing preflight test).
  - Read: brief; handoff-A; `main.rs:2880-3320, 3520-3560, 3630-3655, 1830-1860`; `control_plane_contracts.rs:1-260, 300-600, 2195-2230`; `fleet_http.rs:1-260, 455-480`; `auth.rs:60-150`; `tart_outage_tests.rs:105-130`; `gitlab_runner_script.rs:660-720`; `ci.yml`.

## Phase 6 Implementation (F) - 2026-11-22
- **Decided:**
  - Tart: `hold_slot`/`free_slot` set and clear `always-busy` (the existing branch), so `tart run` only reads the held-slot state. `FAKE_TART` logs each call and each PATH as one file per entry in `calls/` and `paths/`, written under a `.<pid>-<n>` name and renamed into place. `FakeTart::logged` reads whole entries by write time. `calls` was hardened too: it has the same concurrent-append shape (brief §1 sweep).
  - API: one shared helper, `tests/support/api_process.rs`, with a single `pub fn start(client, ports, health_wait, cookie, spawn)`. It checks health, then that the child is alive, then probes `GET /api/v1/auth/me` with the fixture's own cookie, and moves to the next offered port, at most 5 attempts. `ApiFixture` offers `first_port` and then free ports; fleet `start` offers free ports; fleet `restart` offers only its own port, so it never moves and fails naming the origin.
  - Each fixture seeds a per-fixture session (simple UUID, no `.`) once, before it spawns. Health budgets are unchanged (3 s and 5 s, 50 ms polls).
- **Assumed:** the API's startup writes nothing that conflicts with rows seeded before it starts (`ensure_control_plane` is `ON CONFLICT DO NOTHING` on `fleet_control_plane`), and `connect_database_path` migrates (evidence.md).
- **Hedged:**
  - A first `mktemp`-based logger made each fake command about 2.6x slower (17 ms against 6.5 ms) and failed the 0.3 s `provisioning_gives_up...` test once under load. The final PID-named writer costs about 9.5 ms. Its stress results are in handoff-F.
  - API stress runs use the `CARGO_BIN_EXE_gridops-api` path in the shared target dir.
- **Evidence:**
  - handoff-F.md Tier 1 section: fmt, clippy 1.98.1 with CI flags, `cargo test -p gridops-tart-agent` 54/54, `cargo test -p gridops-api` with all new tests green and only the pre-existing macOS `release_preflight` failure;
  - the stress counts;
  - docs/handoffs/124/evidence.md.

## Phase 7 Verify GREEN (T) - 2026-11-22
- **Decided:** PASS. Suite GREEN; fmt and CI-pinned clippy clean; no blocking Tier 2 issue. Repaired only a stale comment in R1 (test-owned); no production code touched.
- **Assumed:** the pre-existing macOS `release_preflight` failure is out of scope, as T-red established on base `9e357e0`.
- **Hedged:** my stress counts (40/40/30) are smaller than F's; they agree with F's 0-failure results and the old-fixture rates are T-red's measurements. `FakeTart::logged` orders by mtime, which can tie on a coarse-clock filesystem, but no caller depends on order (checked every `calls()`/`calls_to()` use).
- **Evidence:** handoff-T-green.md; mutation checks (counter-based slot hold fails R1; disabled identity probe fails R3 and R6).

## Phase 7 Anti-duplication gate (A) - 2026-11-22
- **Decided:** PASS. F extended `always-busy` / `FakeTart::flag` and put the API guard in one shared `tests/support/api_process.rs`; the old fleet health loop and `SESSION_TOKEN` are gone, so no parallel path. No production changes.
- **Assumed:** no survey.md exists, so the brief and handoff-A.md served as the Reuse map.
- **Hedged:** two warns (heavier per-entry fake logger, justified by stress evidence; `session_cookie` vs `session_cookie_for` could share a body).
- **Evidence:** diff 9e357e0..8233328 read in full for the helper, fleet_http, gitlab_runner_script, tart_outage_tests; handoff-A-dup.md.

## Follow-up round on PR #205 (T) - 2026-11-22
- **Decided:** fixed two CI flakes with test-only changes. (A) The API fixture helper waits while its own child is alive and not healthy, under one 60 s `HANG_GUARD` (a hang guard, not the readiness signal); per-caller health waits removed; new slow-bind regression test. (B) The API-restart test restarts only after the agent ran the inspect and posted its (refused) answer; the admin-repair sibling waits for the start to run, then sleeps until issue + 7.5 s.
- **Assumed:** the agent journals before posting (`dispatch.rs:120`), so a refused result post carrying the command id proves the answer survives a restart.
- **Hedged:** `agent.rs:215` and `:287` sleep a fixed time and expect periodic agent work. That is not the same class and they have not flaked, so I recorded them as advisory only.
- **Evidence:** handoff-T-followup.md. 1 ms-poll repro: old 4/60 fail, new 0/120. Stress: restart 80/80, admin repair 40/40, control_plane_contracts binary 30/30, at load 8.4–12.3. fmt and CI clippy clean. Full `cargo test -p gridops-api` green except the pre-existing macOS preflight test.
