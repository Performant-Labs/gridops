# Handoff-T-green: Phase 7 - SQLITE_BUSY reconnect storm (#179)

**Date:** 2026-11-21
**Branch:** claude/lock-storm-179-17e061
**Issue:** #179
**Handoff-F reviewed:** docs/handoffs/179/handoff-F.md
**Handoff-T-red:** docs/handoffs/179/handoff-T-red.md

## GREEN confirmation
Run with `CARGO_BUILD_JOBS=2 RUST_TEST_THREADS=2`.

`cargo test --no-fail-fast -p gridops-core -p gridops-api -p gridops-host-agent -p gridops-reconciler`:
888 passed, 1 failed (F reported 887 + 1; the extra pass is the `LAST_FAILURE` plan test T added this phase).
The one failure is `fleet_http release_preflight::the_release_preflight_fails_early_without_a_macos_arm64_machine`.
Re-run alone it fails with `PASS  macos-machine: macOS ARM64 machine ready ...` on this Mac. `probe::host_os` is
`cfg!(target_os = "macos")` (`crates/gridops-host-agent/src/probe.rs:16`), so the fixture's "Linux" machine enrolls
as macOS here. The diff touches neither probe nor the preflight. It is environment-dependent, not a regression,
and passes on CI's Linux runners.

All #179 tests pass: both `query_plan_tests` (plus the new third), `log_filter_tests` in the api and the
reconciler, the six `busy_database::` tests, all `recovery::` tests unchanged (no amendment was needed),
and `inventory::idle_command_poll_answers_while_another_writer_holds_the_database`.

**Mutation spot-checks (production edited, tests re-run, edits reverted with `git checkout`):**
- Dropping `c.state IN ('pending','delivered')` from `UNDELIVERABLE`: the plan test FAILS.
- Reverting `LAST_FAILURE` to a non-host predicate: the new `LAST_FAILURE` plan test FAILS.
- Making `resend_while_leased` return the first `Unavailable` (no retry): 4 of 6 `busy_database::` tests FAIL
  (the two resend arms, the lease-expiry arm, the held-lock arm); the 401 and result arms stay green, as they should.

## Tier 1 results
- `cargo fmt --all -- --check`: PASS.
- `cargo +1.98.1 clippy --workspace --all-targets --locked -- -D warnings -A clippy::unused_async_trait_impl`
  (CI's toolchain and flags, as F ran): PASS, no warnings.
- Four-crate test run: PASS except the macOS-only pre-existing failure above. Same result as F.
- Server start / API smoke: covered by the `fleet_http` integration suite, which boots the real api router
  and the real agent Supervisor. No separate `curl` smoke run (no UI, no deploy).

## Tier 2 results
- **Coverage per criterion:** PASS (table below).
- **Test quality:** PASS. Each test names a behavior and pins it at the cheapest tier (core plan and behaviour tests
  in-crate; agent behaviour through the real Supervisor against the real api via the extended proxy). No duplicates
  found. Proportionate to the change.
- **Plan-test contract added this phase:** `the_start_retry_reads_one_hosts_events_not_the_whole_table`
  (`crates/gridops-core/src/fleet/registry/query_plan_tests.rs`) asserts `LAST_FAILURE` plans as a
  `SEARCH fleet_events USING INDEX ... host_id=?` and no `SCAN fleet_events`. This was the Phase-6 item F
  handed to T (the `host_commands` "no `(host_id=?)`" rule does not apply to this statement).
- **Type safety:** PASS. No `any`-style escapes; no new `pub` items; the consts are `pub(super)`; `log` is a
  direct dependency at the version sqlx already pulls in (Cargo.lock grows by one line).
- **Error handling:** PASS. 401 stops the agent, a 503 on a result still ends the session, `Refused` and poll
  failures are unchanged, and the lease-expiry arm ends the session.
- **Data integrity / replay:** PASS. The resent heartbeat and inventory are byte-identical, and the tests assert one
  handshake and the same `authority_session_id`. The server's replay rules are in evidence.md with verbatim excerpts.
- **Migration safety:** N/A. No migration (each changed predicate reaches an existing index).
- **Security:** no new inputs or auth surfaces. Not applicable.
- **Playwright:** N/A (no UI, none in this project's change).
- **Evidence appendix:** checked. Every unchanged-code fact the tests rely on (the FK that makes `LAST_FAILURE`
  equivalent, the replay rules, `backoff`, `begin`, the heartbeat's call to `cancel_undeliverable`) is in
  `docs/handoffs/179/evidence.md`. The `lease_expires_at` check is behavioural. Nothing to append.
- **F's results cross-checked:** the same commands give the same results, apart from the +1 test T added.
- **Sweep table:** F's table states its grep and a count of 25, matching the brief. Spot-checked against
  the changed statements: consistent.

## Acceptance criteria status
| Criterion | Status | Backing test |
|---|---|---|
| Plan: poll, cancel and settle selects use `host_commands_poll_idx` with `state=?`, never `(host_id=?)` | PASS | `hot_path_command_statements_read_only_open_commands_through_the_poll_index` |
| Plan: other swept statement (`LAST_FAILURE`) bounded | PASS | `the_start_retry_reads_one_hosts_events_not_the_whole_table` (added this phase) |
| Same cancellations, failures and delivery order with a 20k history | PASS | `a_long_history_changes_nothing_poll_cancels_or_delivers` plus the unchanged api poll/cancel tests |
| Slow statements: both default filters admit `sqlx::query` WARN; a 100 ms+ statement through `connect_database_path` logs a WARN | PASS | api and reconciler `log_filter_tests` (RED on old code per T-red) |
| Heartbeat and inventory 503s resent unchanged, one handshake, same session, backoff >= 1 s | PASS | `busy_database::` heartbeat and inventory resend tests |
| Outage past the lease ends the session and reconnects | PASS | `a_heartbeat_outage_that_outlasts_the_lease_ends_the_session` |
| 401 stops the loop; a 503 on a result still ends the session | PASS | `a_heartbeat_refused_as_unauthenticated...`, `a_result_the_api_cannot_take...` |
| Held write lock past `busy_timeout`: session survives | PASS | `a_database_held_past_the_busy_timeout_leaves_the_session_alone` |
| Existing recovery tests unchanged and passing | PASS | `recovery::*` |
| CHANGELOG entry (Upgrading note with `RUST_LOG` hint, plus Fixed entry) | PASS | diff of CHANGELOG.md |
| fmt, clippy, four-crate tests | PASS (one macOS-only pre-existing failure) | above |

## Blocking issues
None.

## Advisory notes
- `lease_expires_at` is a local in the heartbeat loop, not an `Authority` field as the brief worded it. Behaviour is
  what the brief requires, and A's R3 warn 1 recommended it. Not a blocker.
- The agent compares the server's wall-clock lease with its own clock. Skew only shifts the resend window; it cannot
  loop forever, because a server-side lease expiry answers `Refused`.
- The PR should name the Mars host cron (`/etc/cron.hourly/free`, `fstrim`) and the intended new WARNs from migrations
  and the reconciler's retention deletes, and say which parts are proven (1 and 2) and defensive (3).
