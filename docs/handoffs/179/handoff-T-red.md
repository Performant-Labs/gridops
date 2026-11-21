# Handoff-T-red: Phase 4 - SQLITE_BUSY reconnect storm: bounded hot-path queries, lock-holder visibility, in-session retry (#179)

**Date:** 2026-11-21
**Branch:** claude/lock-storm-179-17e061 (base 1d06c81)
**Brief / wireframe reviewed:** docs/handoffs/179-brief.md (as amended in 5356629 and f5fc862), docs/handoffs/179-brief-result-r2.md, docs/handoffs/179/handoff-A.md (R3). Wireframe: N/A (no UI).

## A precondition
Confirmed: A returned PASS on the plan (Phase 3, R3), with 0 blocks and 5 warns. The tests follow these warns:
- **Warn 1 (lease deadline).** The lease-expiry arm reads the deadline from the last short lease the proxy granted before the outage. It checks behaviour only: it does not care whether the deadline is kept in `Authority` or in a local variable.
- **Warn 2 (plan assertion).** No test names a forbidden index. The plan test requires a step containing `INDEX host_commands_poll_idx (host_id=? AND state=?)` and forbids any step ending in `(host_id=?)` (through any index) and any `SCAN`. The 20k seed is kept, as the brief asks, but the plan does not depend on it.
- **Warn 3 (test labels).** The existing poll and cancel behaviour tests are api-crate `fleet_http` tests. I re-ran them unchanged (see below). The new large-history behaviour test is in core.
- **Warn 4 (worker thread).** The slow-statement test installs a process-wide subscriber once (`Once`) in the api binary's unit tests. It is built from the api's own default filter plus a capture layer.
- **Warn 5 (`log` dependency).** No test depends on how `log` is declared. That is F's change.

## Contract the tests fix (F must provide these names)
1. **Core, `crates/gridops-core/src/fleet/registry/commands.rs`:** three `pub(super) const` statements, `&str`, each the exact SQL that the function executes:
   - `NEXT_DELIVERY`: the delivery select in `poll_commands`.
   - `UNDELIVERABLE`: the select in `cancel_undeliverable`.
   - `UNRECEIVED`: the select in `settle_unreceived`, which is already bounded and needs no SQL change.
2. **Api, `crates/gridops-api/src/main.rs`:** `fn default_log_filter() -> tracing_subscriber::EnvFilter` at the crate root. It returns the default filter, used when `RUST_LOG` is unset or invalid, and `main` uses it in place of the inline `"gridops=info,tower_http=info"`. It must not read the environment, because tests call it directly.
3. **Reconciler, `crates/gridops-reconciler/src/main.rs`:** the same function, `fn default_log_filter() -> EnvFilter`, with the same rules.

**Add these three items first.** Until all three exist, the core lib test binary and both binaries' unit-test targets do not compile, so none of their existing tests can run. In Rust, a RED over new items always works this way (the #140, #146 and #148 precedents). The `fleet_http` integration binary needs no new symbol and runs today.

## Tests authored

### Core: `crates/gridops-core/src/fleet/registry/query_plan_tests.rs` (new; included from `registry/tests.rs`, shaped like `tests/retention.rs`)
The fixture is `History`: an enrolled host with `seed_history`, a current session with work authority, and **20,000 finished commands** (`succeeded`/`failed`/`cancelled`, sealed to another session, deadlines in the past). It also has one open or recently open command of each kind the cancel and delivery rules tell apart. One of them, `deliver-first`, is queued *before* the history.

| Test | Pins | Tier |
|---|---|---|
| `hot_path_command_statements_read_only_open_commands_through_the_poll_index` | Brief Done-when "Planner regression test". For each of `NEXT_DELIVERY`, `UNDELIVERABLE` and `UNRECEIVED`, `EXPLAIN QUERY PLAN` must contain a step with `INDEX host_commands_poll_idx (host_id=? AND state=?)`, no step ending in `(host_id=?)` and no `SCAN`. It lists every failing statement, not only the first. | Unit, against the real migrated schema. That is the cheapest place that sees SQLite's planner. |
| `a_long_history_changes_nothing_poll_cancels_or_delivers` | Brief Done-when "Behaviour tests (core)": the same cancellations, placement failures and delivery order with a long history. **Guard: green before and after.** See the table below. | Integration, through `RegistryService::poll_commands`. Cancellation and delivery run in the same transaction, so only the service shows both. |

The behaviour test's cases:

| Case | Expected after `poll_commands` |
|---|---|
| `deliver-first` (pending, queued before the history) | delivered first: attempt 1, one attempt row |
| `stale-session-start` (pending start sealed to a replaced session) | cancelled; placement `failed` once, `agent_session_lost`, `session: replaced`, `command_kind: start_runner`; allocation `released` |
| `past-margin-prepare` (pending, deadline within the 5 s margin) | cancelled; placement `failed`, `prepare_deadline_expired`; allocation `released` |
| `old-epoch-inspect` (pending, epoch 2) | cancelled |
| `registry-command` (`seed_history`'s pending inspect with no session) | cancelled |
| `expired-start` (delivered, past deadline) | uncertain; placement `uncertain`, not failed; allocation kept (`reserved`) |
| `expired-inspect` (delivered placement inspect, past deadline) | uncertain; placement stays `running` |
| `accepted-old`, `uncertain-old` (past deadline, stale session, not open) | untouched |
| `redeliver` (delivered, last attempt 120 s ago) | second poll: redelivered, attempt 2 |
| `deliver-second` (pending, after the history) | third poll |
| `delivered-recent` (attempt 1 s ago) | never redelivered; a fourth poll returns nothing |
| `ended-session-start` (pending, after the session is cleared) | cancelled; placement `failed`, `agent_session_lost`, `session: ended` |
| the 20,000 history rows | per-state counts unchanged |

### Api agent tests: `crates/gridops-api/tests/fleet_http/busy_database.rs` (new), using the real Supervisor and the real API binary through the recovery `Proxy`
**Proxy extension, in `recovery.rs`:** `Faults` was extended as the brief allows. No new fake server.
- `fail(suffix, status, times)` and `heal(suffix)`: answer the next N requests for a path with a status, without reaching the API. `usize::MAX` means until healed.
- `shorten_leases(ms)`: rewrites `lease.expires_at` in successful heartbeat and inventory replies.
- `seen(suffix)`: every request the proxy answered, with its body, arrival `Instant`, arrival wall-clock ms, answer time, status, and any shortened lease.

`Proxy`, `Faults`, `Seen`, `within` and `agent_session` became `pub(super)`. The existing proxy behaviours (`refuse_results`, `swallow_command`, `hold_bootstrap`) are unchanged. **All 14 existing `recovery::` tests pass unchanged on the extended proxy** (see RED confirmation), and no test body in `recovery.rs` changed.

| Test | Pins (brief Done-when "Agent test" / "Held-lock test") | RED today? |
|---|---|---|
| `a_heartbeat_the_api_cannot_answer_is_resent_unchanged_in_the_same_session` | **Heartbeat arm.** 3 × 503, then 200. Exactly one handshake, the same `agent_session_id`, and four byte-identical bodies (same operation id, sequence and `observed_at`) carrying the session id. Consecutive attempts are ≥ 900 ms apart (`MIN_BACKOFF` 1 s, minus scheduling). | **RED** |
| `an_inventory_refresh_the_api_cannot_answer_is_resent_unchanged_in_the_same_session` | **Inventory-refresh arm**, asserted separately with the same checks. The fault is armed after the session's first inventory, so it hits the 30 s refresh. | **RED** |
| `a_heartbeat_outage_that_outlasts_the_lease_ends_the_session` | **Lease-expiry arm.** The proxy shortens leases to 4 s, then fails every heartbeat. Exactly one handshake arrives before the lease held at the outage runs out, and the second arrives at or after it. The second handshake carries a new `agent_session_id`, and the database records the new session. | **RED** |
| `a_heartbeat_refused_as_unauthenticated_still_stops_the_agent` | A 401 on a heartbeat still stops the loop as `Stop::CredentialRejected`, with no second handshake. **Guard.** | green by design |
| `a_result_the_api_cannot_take_still_ends_the_session` | A command result that gets one 503 still ends the session: a second handshake and a new session. The result is then delivered (results seen: `[503, 200]`). **Guard:** an in-session result retry would fail it. | green by design |
| `a_database_held_past_the_busy_timeout_leaves_the_session_alone` | **Held-lock test.** With no open command (asserted), `BEGIN IMMEDIATE` is held from a second connection for 7.5 s, past the 5 s `busy_timeout`. At least one heartbeat must have answered 503 during the hold, or the test proves nothing (checked first). After the release a heartbeat is answered. There is still exactly one handshake, `fleet_observation_sessions` has no new row, and the session id is unchanged. | **RED** |

These sit at the integration tier because the behaviour is the real agent's reaction to real HTTP answers. A fake client would test the fake.

### Api: `crates/gridops-api/src/log_filter_tests.rs` (new; `#[cfg(test)] mod log_filter_tests;` in `main.rs`)
- `a_statement_running_past_100_ms_is_logged_as_a_slow_statement_warning` (brief Done-when "Slow-statement logging test", behaviour level).
  - **Setup:** a global subscriber, installed once, built from `default_log_filter()` plus a capture layer. It has to be global because sqlx emits from its worker thread (A warn 4). A pool comes from `connect_database_path`. A recursive-CTE statement is doubled in size until it runs for at least 200 ms.
  - **Asserts:** the statement produced a WARN on target `sqlx::query` with "slow statement" in the message and `slow_threshold = 100ms`, and no non-slow `sqlx::query` event was let through.
  - **What it catches:** with today's 1 s default, the 200 ms run logs nothing. A run of 1 s or more on a slow machine would log `1s` and fail on the threshold. Either way, only a 100 ms threshold passes.

### Reconciler: `crates/gridops-reconciler/src/log_filter_tests.rs` (new; wired in `main.rs` beside the other `*_tests` modules)
- `the_default_filter_admits_slow_statement_warnings_and_not_every_statement`: under a scoped subscriber built from `default_log_filter()`, the test emits four events: `sqlx::query` WARN, `sqlx::query` INFO, `sqlx::query` DEBUG and `gridops_reconciler` INFO. Exactly the first and the last get through. This is a unit test, the cheapest tier: the reconciler's part of part 2 is its filter only, because `connect_database_path`'s threshold is already covered by the api test.

### Not authored, by design
- **Extra plan assertions for other swept statements.** The brief says to add them "for every other hot-path statement the sweep changed". Which statements the sweep changes is not known before F's sweep. I ran a preliminary `EXPLAIN QUERY PLAN` on a migrated scratch DB:
  - the hot-path statements on `fleet_observation_sessions` and `fleet_inventory_snapshots` are bounded by their PK or the partial `fleet_observation_sessions_active_host` index;
  - the per-placement `host_commands` reads use `host_commands_placement_idx`.

  I expect no other change. If F's sweep table changes any statement, I add its plan assertion in Phase 6. A statement outside core's `fleet` module gets it in its own crate (A warn 3).
- **The amendment to `an_api_restart_leaves_a_running_runner_alone`.** It passes today, and A traced that it still passes under R3: the refused result ends the session. If it times out after F's change, I amend it in Phase 6 as the brief authorises.

## RED confirmation
Run with `CARGO_BUILD_JOBS=2 RUST_TEST_THREADS=2` on HEAD 1d06c81 plus the test files only. `cargo fmt --all -- --check` is clean.

**1. Compile RED (the real state of the branch). These are the missing contract items:**
```
cargo test -p gridops-core --lib --no-run
error[E0432]: unresolved imports `super::super::commands::NEXT_DELIVERY`, `super::super::commands::UNDELIVERABLE`, `super::super::commands::UNRECEIVED`
  --> crates/gridops-core/src/fleet/registry/query_plan_tests.rs:10:30
cargo test -p gridops-api --bin gridops-api --no-run
error[E0425]: cannot find function `default_log_filter` in module `super`
  --> crates/gridops-api/src/log_filter_tests.rs:74:30
cargo test -p gridops-reconciler --bin gridops-reconciler --no-run
error[E0425]: cannot find function `default_log_filter` in module `super`
  --> crates/gridops-reconciler/src/log_filter_tests.rs:34:22
```

**2. Runtime RED behind the compile RED.** It used throwaway stubs that reproduce today's behaviour: the three consts holding today's SQL verbatim, and `default_log_filter()` returning today's default strings. The stubs were reverted and never staged.
```
query_plan_tests::hot_path_command_statements_read_only_open_commands_through_the_poll_index ... FAILED
these must reach host_commands through INDEX host_commands_poll_idx (host_id=? AND state=?) only:
poll_commands delivery: ["SEARCH c USING INDEX sqlite_autoindex_host_commands_2 (host_id=?)", "CORRELATED SCALAR SUBQUERY 1", "SEARCH a USING INDEX sqlite_autoindex_host_command_attempts_1 (command_id=?)", "USE TEMP B-TREE FOR ORDER BY"]
cancel_undeliverable: ["SEARCH h USING INDEX sqlite_autoindex_fleet_hosts_1 (id=?)", "SEARCH c USING INDEX host_commands_host_kind_issued_idx (host_id=?)"]
query_plan_tests::a_long_history_changes_nothing_poll_cancels_or_delivers ... ok   (guard)

log_filter_tests::a_statement_running_past_100_ms_is_logged_as_a_slow_statement_warning ... FAILED
a statement that ran for 200 ms logged nothing on sqlx::query

log_filter_tests::the_default_filter_admits_slow_statement_warnings_and_not_every_statement ... FAILED
  left: [("gridops_reconciler", Level(Info))]
 right: [("sqlx::query", Level(Warn)), ("gridops_reconciler", Level(Info))]
```
`settle_unreceived` is absent from the plan failure, as expected: it is already bounded.

**3. Agent RED (real tree, no stubs needed).** `cargo test -p gridops-api --test fleet_http -- recovery:: busy_database:: inventory::idle_command_poll` gave 17 passed and 4 failed in 53 s:
```
busy_database::a_heartbeat_the_api_cannot_answer_is_resent_unchanged_in_the_same_session ... FAILED
  a failed /heartbeat ended the session: the agent handshook again   left: 4  right: 1
busy_database::an_inventory_refresh_the_api_cannot_answer_is_resent_unchanged_in_the_same_session ... FAILED
  a failed /inventory ended the session: the agent handshook again   left: 4  right: 1
busy_database::a_heartbeat_outage_that_outlasts_the_lease_ends_the_session ... FAILED
  the agent handshook again 2787 ms before the lease it held ran out   left: 2  right: 1
busy_database::a_database_held_past_the_busy_timeout_leaves_the_session_alone ... FAILED
  the busy database ended the session: the agent handshook again   left: 2  right: 1
busy_database::a_heartbeat_refused_as_unauthenticated_still_stops_the_agent ... ok   (guard)
busy_database::a_result_the_api_cannot_take_still_ends_the_session ... ok           (guard)
inventory::idle_command_poll_answers_while_another_writer_holds_the_database ... ok
recovery::* (14 tests, incl. host_recovers_when_a_delivered_command_never_reaches_it and an_api_restart_leaves_a_running_runner_alone) ... ok
```
Each agent failure is today's reconnect on the first 503, which is the feature's own assertion. The held-lock test's check that a heartbeat answered 503 during the hold passed before the handshake assertion failed, so the hold really did exceed `busy_timeout`.

**4. The tests can pass, and the guards bite:**
- **Satisfiable, part 2:** with throwaway `sqlx::query=warn` in the filter and `log_slow_statements(Warn, 100 ms)` in `db.rs`, the api test **passes** (0.7 s). With the filter only, it fails. The reconciler test passes with the filter stub fixed. All throwaway changes, including the temporary `log` dependency, were reverted with `git checkout`.
- **Behaviour guard, mutation 1:** with the inline poll select changed to `ORDER BY c.cursor DESC`, it fails: `left: ["deliver-second"] right: ["deliver-first"]`.
- **Behaviour guard, mutation 2:** with the `c.host_epoch IS NOT h.epoch` branch removed from `cancel_undeliverable`, it fails: `command old-epoch-inspect left: "pending" right: "cancelled"`.
- **Lint:** `cargo +1.98.1 clippy -p gridops-core -p gridops-api -p gridops-reconciler --all-targets --locked -- -D warnings -A clippy::unused_async_trait_impl` (CI's toolchain and flags) exits 0 with the stubs in place, so every test file compiled.
- **Correction:** my first draft of the 401 guard dropped the fixture (the API process) through a `..` pattern, and timed out for that reason. It now binds `f: _api`, and passes as a guard.

## Notes for F
- The poll's plan needs only the top-level `state IN ('pending','delivered')`. `USE TEMP B-TREE FOR ORDER BY` stays allowed: the test does not forbid it.
- **Inventory arm:** the agent resends the refresh's identical `InventoryRequest`, with the same `request_id` and the same `report` including `observed_at`. Build it once before the retry loop.
- **Heartbeat arm:** the same rule applies to the heartbeat's `HeartbeatRequest`.
- **Lease-expiry arm:** the session must not end before the lease captured at the first failure has passed. Giving up early, because the next backoff would cross the deadline, fails the test. Clipping the last sleep to the deadline (A warn 1) passes.
- The agent tests take about 50 s in total. The inventory arm waits for the 30 s refresh.

## Ready for F
RED is valid. Each RED test fails on the assertion the feature will satisfy:
- the plan steps for the poll and the cancel select;
- the missing slow-statement warning;
- the reconciler filter's missing `sqlx::query` WARN;
- the second handshake in the four agent tests.

The only other failures are the compile errors for the three contract items named above. The three guards (large-history behaviour, 401, result 503) are labelled as guards and are green by design. F may implement against these tests.
