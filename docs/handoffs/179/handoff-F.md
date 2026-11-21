# Handoff-F: Phase 5 - SQLITE_BUSY reconnect storm: bounded hot-path queries, lock-holder visibility, in-session retry (#179)

**Date:** 2026-11-21
**Branch:** claude/lock-storm-179-17e061 (base 8f14479)
**Issue:** #179

## What was done
- `crates/gridops-core/src/fleet/registry/commands.rs`: the three selects that run under the write lock on every heartbeat, every poll with an open command and every handshake are now `pub(super) const`s:
  - `NEXT_DELIVERY`: the poll delivery select. It gains a top-level `c.state IN ('pending','delivered')` and is otherwise unchanged.
  - `UNDELIVERABLE`: the `cancel_undeliverable` select. It gains the same top-level conjunct and is otherwise unchanged.
  - `UNRECEIVED`: the `settle_unreceived` select. The SQL is unchanged and only moved into a const.

  `poll_commands`, `cancel_undeliverable` and `settle_unreceived` execute those consts.
- `crates/gridops-core/src/fleet/retry.rs`: a sweep finding. The in-lock read of a placement's newest failure event is now `pub(super) const LAST_FAILURE` and names the placement's host (`host_id=?`). Before, it was a whole-table `SCAN fleet_events`; now it uses `fleet_events_host_cursor_idx`. The FK makes the results identical (see Design decisions).
- `crates/gridops-core/src/db.rs`: `connect_database_path` sets `.log_slow_statements(log::LevelFilter::Warn, SLOW_STATEMENT)`, with `SLOW_STATEMENT` = 100 ms. `ConnectOptions` is now imported for the trait method.
- `Cargo.toml` (workspace): `log = "0.4.33"` in `[workspace.dependencies]`. `crates/gridops-core/Cargo.toml`: `log.workspace = true`. `Cargo.lock`: gridops-core now lists `log`. This is the version sqlx already pulls in, so nothing new is downloaded.
- `crates/gridops-api/src/main.rs`: `fn default_log_filter() -> EnvFilter` returns `gridops=info,tower_http=info,sqlx::query=warn`. `main` uses it when `RUST_LOG` is unset or invalid.
- `crates/gridops-reconciler/src/main.rs`: `fn default_log_filter() -> EnvFilter` returns `gridops_reconciler=info,sqlx::query=warn`. `main` uses it the same way.
- `crates/gridops-host-agent/src/supervisor.rs`: changes to the session loop:
  - New `resend_while_leased` helper, next to `backoff()`.
  - The heartbeat and the inventory refresh build their request once and send it through the helper.
  - New local `lease_expires_at`. It is seeded from the session's first inventory and updated from every successful refresh and heartbeat.
  - The module doc comment is updated.
- `CHANGELOG.md` (Unreleased):
  - Upgrading: the 100 ms slow-statement WARN, the `sqlx::query=warn` note for anyone who sets `RUST_LOG`, and the expected warnings from migrations and retention deletes.
  - Fixed: the storm fix.

## Design decisions
- **The rewrite is the smallest sargable change.** Each select gets one top-level conjunct, `c.state IN ('pending','delivered')`, and keeps its original disjunction unchanged under it. Both original disjunctions already require `state` to be `pending` or `delivered`, so the conjunct removes no row the original selected. That makes "same rows, same order" true by construction. I chose this over rewriting the OR into a UNION, which would change the shape and need its own ordering proof. The poll keeps `ORDER BY c.cursor LIMIT 1`. Its plan still sorts in a temp B-tree, but now only over the host's open commands. T's test allows that.
- **`lease_expires_at` is a local variable in the `heartbeats` block, not a field in `Authority`** (A R3 warn 1). This is a deviation from the brief's wording (see Deviations).
  - Only that loop reads or writes it.
  - `current` is read once per iteration, before the inventory refresh, so a lease kept in `Authority` would be stale for the heartbeat that follows a successful refresh.
  - It stays separate from the work-authority `Authority::expires_at`, as the brief requires.
- **The deadline is fixed per request.** `resend_while_leased` takes the lease by value when the request is first sent. The local changes only after a successful reply, so this is the lease held at the first failure, and the helper never re-reads it.
  - Each wait is `min(backoff(attempts), deadline - now)`, so the last wait is cut short at the deadline (A warn 1).
  - The next `Unavailable` at or after the deadline is returned, and the session ends as today.
  - The helper's `attempts` counter is separate from `run`'s reconnect `failures`.
- **Only the client call is wrapped.** The closure is `|| self.client.heartbeat(host, &credential, &request)`, and `inventory` is wrapped the same way. Requests are built outside it, and `internal()` errors from building them still end the session at once. The resend sends the same `HeartbeatRequest`/`InventoryRequest` value, so it has the same operation id, sequence, `observed_at` and body. The server's replay rules answer it (evidence.md).
- **Deadline clock.** The lease is a server wall-clock `WireTimestamp`, and the helper compares it with the agent's `gridops_core::now_millis()`. The other option was to convert `expires_at - received_at` to a monotonic local deadline. That is more robust to skew, but it is more code, and nothing in the agent today guards against wall-clock skew. If the agent's clock runs ahead, the session gives up early. If it runs behind, a server-side lease expiry answers `SessionConflict` (`Refused`), which ends the session anyway. Neither case can loop forever.
- **`LAST_FAILURE` (retry.rs).** The sweep found this statement. It runs inside `retry_failed_start`'s `BEGIN IMMEDIATE`, on the reconciler's fleet pass. It was `SCAN fleet_events`, a reverse rowid walk over every machine's events back to the failure. If a placement had no failure event, it read the whole table.
  - `fleet_events` has a composite FK `(placement_id,host_id,target_id)` to `workload_placements(id,host_id,target_id)`, plus `CHECK (placement_id IS NULL OR host_id IS NOT NULL …)`. So every event with that `placement_id` has the placement's `host_id`, and adding `host_id=?` (bound to the `p.host_id` the statement's caller already selected) returns the same row.
  - No migration was needed: the predicate reaches the existing `fleet_events_host_cursor_idx`.
  - The new plan is `SEARCH fleet_events USING INDEX fleet_events_host_cursor_idx (host_id=?)`, walking the host's events newest first with `LIMIT 1`. It is bounded to one machine's events, not to a single row. That is enough here: the lock-free pre-check (evidence.md) only lets placements through whose runner is still `starting`, and those failed within the 5-minute retry window.
- **The threshold is a named const** (`SLOW_STATEMENT`, 100 ms) in `db.rs`, next to `MIGRATOR`. `busy_timeout(Duration::from_secs(5))` now uses the imported `Duration`. The behaviour is unchanged.

## Reuse / extend-vs-new
EXTEND in place, as the brief and A R3 recommend:
- the statements in `registry/commands.rs`, following the existing `STARTED`/`LIVE_WORKLOADS` const pattern;
- `connect_database_path`;
- each process's existing `EnvFilter` construction, now in one function per binary as T's contract requires;
- `backoff()` and the existing heartbeat loop in `supervisor.rs`.

No new module, service, client method or `pub` item. `client.rs` is unchanged.

## Architecture notes for A
- **Layers.** Core registry SQL and core `fleet::retry` SQL. Core DB connection options. Each binary's log filter. The host agent's session loop. No crate reaches into another.
- **New dependency.** `log` 0.4.33 is a direct dependency of gridops-core, declared through `[workspace.dependencies]` (A warn 5). sqlx already used it.
- **Visibility.**
  - The consts are `pub(super)`: `registry::commands::*` is visible to `registry::tests`, and `fleet::retry::LAST_FAILURE` is visible within `crate::fleet`, including `fleet::registry::tests`.
  - `default_log_filter` is private at each binary's crate root.
  - `resend_while_leased` is private.
- **Protocol / schema.** No schema or migration change. No protocol change. The agent now resends a byte-identical heartbeat or inventory, which the existing replay rules answer.
- **Unchanged, as the brief requires.**
  - The api's 503 mapping.
  - Command poll and result failures, which still end the session.
  - The handshake, the first inventory and `deliver_results`.
  - Credential rotation.
  - Log and output shipping.

## Deviations from spec / wireframe
- **`lease_expires_at` is not stored in `Authority`.** The brief says to "store it in `Authority` as `lease_expires_at`". I kept it as a local of the same name in the heartbeat loop, as A's R3 warn 1 recommends (reasons under Design decisions). The behaviour is what the brief requires, and T's test checks behaviour only. `Authority` and its doc comment are unchanged.
- **One statement outside the brief's two named selects changed:** `retry.rs` `LAST_FAILURE`. The brief's sweep clause covers it ("every statement on the other history tables … in the reconciler's fleet pass … gets the same treatment"). T's handoff says T adds a plan assertion for any statement the sweep changed. Expected plan: a step `SEARCH fleet_events USING INDEX fleet_events_host_cursor_idx (host_id=?)` and no `SCAN fleet_events`. **Note for T:** the `host_commands` rule "no step ending in `(host_id=?)`" does not fit this statement. `(host_id=?)` on `fleet_events_host_cursor_idx` with `ORDER BY cursor DESC LIMIT 1` is the intended bound.
- No wireframe (no UI).

## Sweep table (brief Part 1 hand-off)
**Completeness grep:** `grep -rnE '(FROM|JOIN) host_commands\b' crates --include='*.rs' | grep -vE '_tests\.rs:|/tests\.rs:|/tests/'` gives **25 hits**, matching the brief's per-file counts (resources.rs 3, placement.rs 6, commands.rs 6, delete.rs 1, retire.rs 1, unconfirmed.rs 3, retry.rs 1, start_health.rs 1, targets.rs 3). Each has a row below, by current line.

**Method:**
- **Static plans.** The plans come from `EXPLAIN QUERY PLAN` (sqlite3 3.54) against a scratch DB with all 47 migrations applied. For the three `host_commands` consts, T's `query_plan_tests` also confirms the plan with the crate's bundled SQLite. The brief's Mars plans are noted where they differ.
- **Runtime capture.** To find every statement the hot paths actually run, I ran `fleet_http`'s `agent::`, `placement::`, `recovery::`, `inventory::`, `busy_database::` and `start_retry::` modules (52 tests, all passing) with `RUST_LOG=gridops=info,sqlx::query=debug` and `TMPDIR` set to a scratch directory. I took each `db.statement` from every API log and ran `EXPLAIN QUERY PLAN` on each distinct statement that touches a history table (44 statements). Those runs cover handshake, inventory, heartbeat, poll, result, logs, output and credential rotation.

### The 25 grep hits

| # | Site | Function / path | Plan before | Plan after | Verdict |
|---|---|---|---|---|---|
| 1 | fleet/targets.rs:834 | `delete_pool` (admin), DELETE attempts | attempts autoindex `(command_id=?)` ← `host_commands_placement_idx (placement_id=?)` | same | Bounded per placement. Not a hot path. Unchanged. |
| 2 | fleet/targets.rs:839 | `delete_pool`, DELETE results | results autoindex `(command_id=?)` ← `host_commands_placement_idx (placement_id=?)` | same | Unchanged |
| 3 | fleet/targets.rs:844 | `delete_pool`, DELETE commands | `host_commands_placement_idx (placement_id=?)` | same | Unchanged |
| 4 | fleet/placement.rs:102 | `prepared_with` (reconciler) | `host_commands_placement_idx (placement_id=? AND kind=?)` | same | Unchanged |
| 5 | fleet/placement.rs:142 | `succeeded_evidence` (reconciler) | `placement_idx (placement_id=? AND kind=?)` + results `(command_id=?)` + temp B-tree over that placement's rows | same | Unchanged |
| 6 | fleet/placement.rs:169 | `reported_gone` (reconciler) | as #5 | same | Unchanged |
| 7 | fleet/placement.rs:202 | `reported_diagnostic` (reconciler) | as #5 | same | Unchanged |
| 8 | fleet/placement.rs:231 | `command_state` (reconciler) | `placement_idx (placement_id=? AND kind=?)` | same | Unchanged |
| 9 | fleet/placement.rs:387 | start issue, in `BEGIN IMMEDIATE` | `COVERING INDEX placement_idx (placement_id=? AND kind=?)` | same | Unchanged |
| 10 | fleet/retry.rs:134 | `retry_failed_start` `unsettled`, in lock | `placement_idx (placement_id=? AND kind=?)` | same | Unchanged |
| 11 | fleet/start_health.rs:149 | start gate (api and reconciler) | `host_commands_host_kind_issued_idx (host_id=? AND kind=? AND issued_at>?)` + results `(command_id=?)` | same | Bounded by a time window. Unchanged. |
| 12 | registry/unconfirmed.rs:67 | `STARTED` (admin read and repair) | `placement_idx (placement_id=? AND kind=?)` | same | Unchanged |
| 13 | registry/unconfirmed.rs:100 | `unconfirmed_placements` LEFT JOIN | `sqlite_autoindex_host_commands_1 (id=?)` | same | Unchanged |
| 14 | registry/unconfirmed.rs:101 | the same subquery `u` | `placement_idx (placement_id=?)` | same | Unchanged |
| 15 | registry/retire.rs:147 | retire counts | `host_commands_poll_idx (host_id=? AND state=?)` | same | Unchanged |
| 16 | registry/delete.rs:94 | delete counts | `host_commands_poll_idx (host_id=? AND state=?)` | same | Unchanged |
| 17 | registry/commands.rs:48 | `NEXT_DELIVERY` (poll, in lock) | scratch/bundled: `sqlite_autoindex_host_commands_2 (host_id=?)` + temp B-tree; Mars: `host_kind_issued_idx (host_id=?)` + temp B-tree | `host_commands_poll_idx (host_id=? AND state=?)`, attempts autoindex `(command_id=?)`, temp B-tree over open rows | **Changed** |
| 18 | registry/commands.rs:62 | `UNDELIVERABLE` (poll and heartbeat, in lock) | `host_commands_host_kind_issued_idx (host_id=?)` | `host_commands_poll_idx (host_id=? AND state=?)` | **Changed** |
| 19 | registry/commands.rs:72 | `UNRECEIVED` (handshake, in lock) | `host_commands_poll_idx (host_id=? AND state=?)` + results `(command_id=?)` | same | Already bounded. Moved into a const only. |
| 20 | registry/commands.rs:107 | poll `open` EXISTS (read tx) | `COVERING INDEX host_commands_poll_idx (host_id=? AND state=?)` | same | Unchanged; kept as the brief requires |
| 21 | registry/commands.rs:212 | `record_command_result` row | `sqlite_autoindex_host_commands_1 (id=?)` | same | Unchanged |
| 22 | registry/commands.rs:548 | `keep_uncertain_while_unsettled` | `placement_idx (placement_id=?)` | same | Unchanged |
| 23 | api/src/resources.rs:5781 | `runner_start_phases` (overview read, pool, no write tx) | `placement_idx (placement_id=? AND kind=?)` | same | Unchanged (see note) |
| 24 | api/src/resources.rs:5783 | the same | `placement_idx (placement_id=? AND kind=?)` + results `(command_id=?)` | same | Unchanged |
| 25 | api/src/resources.rs:5786 | the same | `placement_idx (placement_id=? AND kind=?)` | same | Unchanged |

Note on #23-25: the outer query is `SCAN pl` over `workload_placements`, filtered by `created_at`. It is an overview read on the pool. Under WAL a reader never blocks a writer, and `workload_placements` is not one of the brief's history tables. Out of scope.

### Other statements on the hot paths, in the registry write lock (from the runtime capture)

| Path | Statement (table) | Plan | Verdict |
|---|---|---|---|
| handshake | load the active session (`fleet_observation_sessions`) | `fleet_observation_sessions_active_host (host_id=?)`, a partial UNIQUE index `WHERE ended_at IS NULL`, so at most one row | Bounded. Unchanged. |
| handshake | `UPDATE … SET ended_at`/`conflicted WHERE session_id=?` | `sqlite_autoindex_fleet_observation_sessions_1 (session_id=?)` | Unchanged |
| handshake | `INSERT INTO fleet_observation_sessions` | n/a | Unchanged |
| handshake | `settle_unreceived` | row #19 | Unchanged |
| inventory / heartbeat | `load_current_session` (`fleet_observation_sessions`) | `sqlite_autoindex_fleet_observation_sessions_2 (host_id=? AND session_id=?)` | Unchanged |
| inventory | replay lookup in `fleet_inventory_snapshots` | `sqlite_autoindex_fleet_inventory_snapshots_1 (host_id=? AND authority_session_id=? AND inventory_sequence=?)` | Unchanged |
| inventory | `INSERT INTO fleet_inventory_snapshots`, `INSERT INTO host_samples` | n/a | Unchanged |
| inventory / heartbeat | `UPDATE fleet_observation_sessions … WHERE session_id=? AND host_id=?` | autoindex_2 `(host_id=? AND session_id=?)` | Unchanged |
| heartbeat | work-authority checks (`fleet_observation_sessions` ⋈ `host_credentials` ⋈ `fleet_hosts`, `EXISTS fleet_inventory_snapshots`) | sessions autoindex_2 `(host_id=? AND session_id=?)`; snapshots autoindex_2 `(host_id=? AND authority_session_id=? AND inventory_sequence=? AND digest=?)`; credentials `host_credentials_rotation_owner (…)` | Unchanged |
| heartbeat | `cancel_undeliverable` | row #18 | **Changed** |
| heartbeat | `SELECT lease_expires_at FROM fleet_observation_sessions …` | autoindex_2 `(host_id=? AND session_id=?)` | Unchanged |
| poll | `UPDATE host_commands … WHERE id=?`; `INSERT INTO host_command_attempts` | `(id=?)` / n/a | Unchanged |
| result | `SELECT disposition FROM host_command_results WHERE command_id=? AND result_hash=?` | autoindex_2 `(command_id=? AND result_hash=?)` | Unchanged |
| result | `EXISTS … host_command_results WHERE command_id=? AND disposition='applied'` | autoindex_2 `(command_id=?)` | Unchanged |
| result | `INSERT INTO host_command_results`; attempts upsert; `INSERT INTO fleet_events` | n/a | Unchanged |
| logs | `count(*)` / `DELETE FROM fleet_agent_logs WHERE host_id=? AND received_at…` | `fleet_agent_logs_host_received_idx (host_id=? AND received_at>?/<?)` | Bounded by time. Unchanged. |
| output | `fleet_output_streams`, `log_streams`, `fleet_hosts`, `workload_placements` by key | no history table | Unchanged |
| credentials (rotation) | `UPDATE fleet_observation_sessions … WHERE host_id=? AND session_id=? …` | autoindex_2 `(host_id=? AND session_id=?)` | Unchanged |
| start gate (api and reconciler) | `fleet_events WHERE host_id=? AND kind=? AND created_at>=?` (3 shapes) | `fleet_events_host_kind_time_idx (host_id=? AND kind=? AND created_at>?)` | Bounded by time. Unchanged. |
| reconciler fleet pass: `retry_failed_start` (in lock) | `LAST_FAILURE`, `fleet_events WHERE placement_id=? AND kind=? ORDER BY cursor DESC LIMIT 1` | before: `SCAN fleet_events`; after (adds `host_id=?`): `SEARCH fleet_events USING INDEX fleet_events_host_cursor_idx (host_id=?)` | **Changed** |
| reconciler fleet pass: `admission.rs` / `placement.rs` command and event INSERTs | n/a | n/a | Unchanged |

### Seen and deliberately left (not on a hot path)
- `delete_pool`'s `DELETE FROM fleet_events WHERE placement_id IN (…)` (targets.rs:849) is `SCAN fleet_events`. It runs only when an administrator deletes a pool, which is a rare action and not an agent or reconciler hot path. It is outside the brief's scope.
- Read-only overview queries on the pool, outside any write transaction:
  - the disk alert's `host_samples … ORDER BY observed_at DESC LIMIT 1` (resources.rs:5890; `host_samples_host_time_idx (host_id=?)`, newest first);
  - the events page (queries.rs:1638; `fleet_events_host_cursor_idx (host_id=?)` with a cursor and LIMIT).

  Under WAL neither holds the write lock.
- The reconciler's unbatched retention deletes (`main.rs` ~3343) are out of scope per the brief. They will now log at WARN, as intended and named in the CHANGELOG.

**No migration was needed.** Every changed predicate reaches an existing index.

## Tier 1 self-check (incl. tests now GREEN)
Run with `CARGO_BUILD_JOBS=2 RUST_TEST_THREADS=2`.

```
cargo fmt --all -- --check                                   -> clean
cargo +1.98.1 clippy --workspace --all-targets --locked -- -D warnings -A clippy::unused_async_trait_impl   (CI's toolchain and flags)
    Finished `dev` profile ... (no warnings)
```
The local default toolchain (clippy 1.99) reports pre-existing lints in files this change does not touch: `host-agent/src/helper.rs:188`, `native.rs:200`, `manager/src/main.rs:233`, `manager/src/vm_slots.rs:755`, and a core autoscaling test. CI pins 1.98.1 for this reason (`.github/workflows/ci.yml:88-113`).

**T's tests, now GREEN:**
```
cargo test -p gridops-core --lib query_plan
test ...query_plan_tests::hot_path_command_statements_read_only_open_commands_through_the_poll_index ... ok
test ...query_plan_tests::a_long_history_changes_nothing_poll_cancels_or_delivers ... ok
cargo test -p gridops-api --bin gridops-api log_filter
test log_filter_tests::a_statement_running_past_100_ms_is_logged_as_a_slow_statement_warning ... ok
cargo test -p gridops-reconciler --bin gridops-reconciler log_filter
test log_filter_tests::the_default_filter_admits_slow_statement_warnings_and_not_every_statement ... ok
cargo test -p gridops-api --test fleet_http -- busy_database:: recovery:: inventory::idle_command_poll
test busy_database::a_heartbeat_the_api_cannot_answer_is_resent_unchanged_in_the_same_session ... ok
test busy_database::an_inventory_refresh_the_api_cannot_answer_is_resent_unchanged_in_the_same_session ... ok
test busy_database::a_heartbeat_outage_that_outlasts_the_lease_ends_the_session ... ok
test busy_database::a_database_held_past_the_busy_timeout_leaves_the_session_alone ... ok
test busy_database::a_heartbeat_refused_as_unauthenticated_still_stops_the_agent ... ok
test busy_database::a_result_the_api_cannot_take_still_ends_the_session ... ok
test inventory::idle_command_poll_answers_while_another_writer_holds_the_database ... ok
recovery::* (14 tests, incl. host_recovers_when_a_delivered_command_never_reaches_it and an_api_restart_leaves_a_running_runner_alone) ... ok
test result: ok. 21 passed; 0 failed
```
`an_api_restart_leaves_a_running_runner_alone` passes unchanged, so no amendment is needed.

**The four crates in full** (`cargo test --no-fail-fast -p gridops-core -p gridops-api -p gridops-host-agent -p gridops-reconciler`): 33 binaries, 887 passed, 1 failed. The failure is a pre-existing test that fails on macOS only and is not caused by this change; see "Known issues".

## Evidence appendix
`docs/handoffs/179/evidence.md`

## Tests that look wrong (for T)
- None of T's #179 tests is wrong.
- `release_preflight::the_release_preflight_fails_early_without_a_macos_arm64_machine` (an existing test, not #179's) fails on a macOS development machine:
  - The agent's OS is fixed at compile time (`probe::host_os`, `cfg!(target_os = "macos")`; see evidence.md), so on a Mac the fixture's "Linux" machine enrolls as macOS ARM64, and the preflight passes `macos-machine`.
  - It depends on the environment, not on this change: nothing here touches enrollment, the probe or the preflight script. It passes on CI's Linux runners.
  - It is flagged only so T's Phase 6 run on a Mac is not mistaken for a regression.
- **Plan assertion for `LAST_FAILURE`.** Add it in Phase 6 (see Deviations): expect `fleet_events_host_cursor_idx (host_id=?)` and no `SCAN fleet_events`.

## Known issues
- **Full four-crate run.** `cargo test --no-fail-fast -p gridops-core -p gridops-api -p gridops-host-agent -p gridops-reconciler` ran 33 test binaries: **887 passed, 1 failed**.
  - The failure is `release_preflight::the_release_preflight_fails_early_without_a_macos_arm64_machine`. It fails on macOS only and does not involve this change (see "Tests that look wrong").
  - Per binary: gridops-core lib 309 (including both `query_plan_tests`); gridops-api bin 131; `control_plane_contracts` 36; `fleet_http` 149 of 150; gridops-host-agent lib 8; gridops-reconciler bin 122. Every other integration binary in the four crates passed.
- The agent's lease deadline compares the server's wall-clock lease with the agent's wall clock (see Design decisions). Heavy clock skew shortens or lengthens the in-session resend window. It can never make the resend loop forever, because the server's own lease expiry answers `SessionConflict` (`Refused`), which ends the session.
- **Host cron (out of scope, for the PR description).** Mars's `/etc/cron.hourly/free` (`drop_caches`) and `/etc/cron.hourly/fstrim` cause the hourly cold-cache burst at :17. The operator removes them. No code change is involved.

## Files changed
- `Cargo.toml`
- `Cargo.lock`
- `CHANGELOG.md`
- `crates/gridops-core/Cargo.toml`
- `crates/gridops-core/src/db.rs`
- `crates/gridops-core/src/fleet/registry/commands.rs`
- `crates/gridops-core/src/fleet/retry.rs`
- `crates/gridops-api/src/main.rs`
- `crates/gridops-reconciler/src/main.rs`
- `crates/gridops-host-agent/src/supervisor.rs`
