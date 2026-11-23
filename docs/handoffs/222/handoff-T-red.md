# Handoff-T-red: Phase 4 - Lock storm: DELETE FROM host_samples scans 628k rows inside the write lock (#222)

**Date:** 2026-11-22
**Branch:** claude/fix-222-retention-batches (head aab847f; only the two test files are staged)
**Brief / wireframe reviewed:** docs/handoffs/222-brief.md, docs/handoffs/222/handoff-A.md (PASS). Wireframe: N/A (no UI).

## A precondition
Confirmed: A returned **PASS** on the plan (Phase 3), with 0 blocks and 7 warns. The tests apply the warns that bear on them:
- **Finding 1 (placement):** the tests import `gridops_core::retention::{RetentionTable, prune, BATCH_SIZE, MAX_BATCHES, BATCH_PAUSE}`.
- **Finding 2 (sharing):** a const test asserts that `snapshot_retention`'s three constants equal `retention`'s.
- **Finding 3:** I extended `crates/gridops-core/tests/retention.rs` and added no second home for plan tests. Its three literal rows (webhook, audit, capacity) became a loop over `RetentionTable::ALL` that plans `table.prune_batch()`, the real const. An exhaustive `match` holds each table's expectation. The `fleet_agent_logs` and `log_streams` literal rows stay. I put the batching tests in the same file, so there is one file.
- **Finding 4 (planner text):** some step must start with `SEARCH <t> USING ` and end with `INDEX <idx> (<col><?)`, which accepts COVERING and pins the index name. No step may start with `SCAN `.
- **The isolation test is in the reconciler (placement decision):** it extends `crates/gridops-reconciler/src/snapshot_retention_tests.rs`, reuses its `Harness`, and injects the fault with a `BEFORE DELETE … RAISE(ABORT)` trigger.

## Contract the tests fix (F must provide these names)
**Core: a new `crates/gridops-core/src/retention.rs`, declared `pub mod retention;` in `lib.rs`.**
- `pub enum RetentionTable { WebhookDeliveries, AuditEvents, CapacitySamples, HostSamples, ReconcilerPasses }`. It must be **`Copy` and `Debug`**: the tests use a variant more than once and print it with `{:?}`.
- `RetentionTable::ALL: [Self; 5]`. The test requires each table exactly once.
- `const fn as_str(self) -> &'static str`, which returns the table name.
- `const fn prune_batch(self) -> &'static str`, which binds `(cutoff_millis, batch_size)` in that order.
- `pub const BATCH_SIZE: i64` (500), `pub const MAX_BATCHES: u32` (40), and `pub const BATCH_PAUSE: Duration` (at least 25 ms). `fleet::registry::snapshot_retention` must expose the same three values.
- `pub async fn prune(pool: &SqlitePool, table: RetentionTable, cutoff_millis: i64, batch_size: i64, max_batches: u32) -> Result<u64, sqlx::Error>`, which returns the number of rows deleted.

**Migration `migrations/0050_host_samples_received_idx.sql`** creates `host_samples_received_idx`. The plan test checks for it by name.

**Reconciler:** no new names. The tests go through `cleanup_retention(&Reconciler)`. They use table names as strings and `snapshot_retention::{BATCH_SIZE, MAX_BATCHES}`, so the reconciler tests compile today.

**The core `retention` integration binary won't compile until `gridops_core::retention` exists.** A RED over new Rust items always works this way (#152, #140, #146, #148, #179). It's a separate test binary, so core's lib tests and its other integration binaries still build and run. The reconciler tests compile today and fail at runtime.

## Tests authored

### Core: `crates/gridops-core/tests/retention.rs` (extended)
An integration binary over a fresh migrated database, through the public API. The seed holds the parents the sample tables need: an installation, a runner pool and a fleet host.

| Test | Pins | Tier, and why |
|---|---|---|
| `retention_deletes_find_their_rows_through_an_index` (kept, trimmed) | The deletes that stay unbatched (`fleet_agent_logs` ×2, `log_streams`) still use an index. I removed its three rows that production will no longer run (A finding 3). | Unit against the real planner, the only place the plan is visible. |
| `every_retention_batch_finds_its_rows_through_its_cutoff_index` | **Brief test 1, the incident's regression test.** For each of the 5 `prune_batch()` consts, the inner select must `SEARCH <t> USING … INDEX <cutoff idx> (<col><?)`, and no step may be a `SCAN`. Also: `ALL` names each table exactly once, and `as_str()` is the table name. | Same as above. |
| `a_tables_prune_takes_only_its_rows_older_than_the_cutoff_oldest_first` | **Brief test 2 (bound, newer rows kept), per table.** Each table gets 5 old rows (rowid order is the reverse of age), one row at the cutoff and one recent row. `prune(…, 2, 2)` returns 4 and takes the 4 oldest. The next call returns 1, and the one after that returns 0. The at-cutoff and recent rows always survive. Tables not yet pruned are untouched, so each const deletes only from its own table. | Integration on the real schema. The rule is SQL in five separate consts, so each one is exercised. |
| `a_prune_stops_at_the_first_short_batch` | **Brief test 2 (early stop).** There are 3 old rows. `prune(…, 2, u32::MAX)` must return 3 within 10 s. A loop that ignores the short batch would sleep `BATCH_PAUSE` per remaining batch, about 3 years. | Integration. Shared loop logic, so one table (`host_samples`) is enough. |
| `a_prune_pauses_between_its_batches` | **Brief test 2 (pause), #152's pattern.** 7 old rows and `prune(…, 2, 3)` give 6 deleted, with elapsed time at least `2 * BATCH_PAUSE`. Only a lower bound, so it's deterministic. | Same as above. |
| `a_prune_with_a_batch_size_below_one_deletes_nothing` | `batch_size` 0 and -1 delete nothing. A negative `LIMIT` is unbounded in SQLite: it would be exactly the unbounded delete this issue removes. | Same as above. |
| `every_prune_is_bounded_to_forty_batches_of_five_hundred_with_a_pause_between` | The decided bounds (500, 40, at least 25 ms), and that `snapshot_retention` shares them. Without it, `BATCH_PAUSE = 0` would pass the pause test trivially, and a huge `MAX_BATCHES` would pass the reconciler cap test, which reads the constant. | Unit (const). The cheapest pin on the lock-safety numbers. |

### Reconciler: `crates/gridops-reconciler/src/snapshot_retention_tests.rs` (extended)
These reuse #152's `Harness` and add helpers on it:
- `capacity_parents`;
- `rows(table, count, youngest)`, a bulk insert through a recursive CTE that inserts the youngest row first;
- `stamps(table)`;
- `setting(key, value)`.

A total const `Table` array describes the five tables, so there is no panicking fallback.

| Test | Pins | Tier, and why |
|---|---|---|
| `a_failing_table_prune_does_not_stop_the_other_tables_or_the_pass` | **Brief test 3, error isolation.** Each of the 5 tables in turn, on a fresh database, gets a trigger that refuses its deletes. `cleanup_retention` must then return `Ok`, so the reconcile pass continues (`main.rs:699` has `cleanup_retention(app).await?`). The other 4 tables and the old snapshot must be pruned, and only the refused table keeps its row. | Integration through `cleanup_retention`. Isolation is reconciler wiring (A's layering decision). |
| `a_pass_deletes_at_most_batch_size_times_max_batches_rows_of_each_table` | **The incident at the wiring level.** There are 20,001 old rows in each of the 5 tables. One pass must leave exactly the youngest one, so it deletes `BATCH_SIZE * MAX_BATCHES`, oldest first. Without this test, F could keep the unbounded deletes and only wrap them in warn-and-continue, and every other test would pass. | Same as above. It takes about 5 s, which is 5 tables × 39 pauses of 25 ms. |
| `each_retained_table_keeps_its_own_window` | **Guard (green before and after).** Rows 1 h either side of the defaults (webhook 90 d, audit 365 d, the other three 31 d) are split correctly. Then `webhookRetentionDays` 10 and `auditRetentionDays` 20 are honoured. The rewiring must not move or swap a cutoff. | Same as above. |

The #152 tests `retention_keeps_the_configured_days_of_snapshots_seven_by_default` and `a_failing_snapshot_prune_does_not_stop_the_rest_of_the_cleanup` are unchanged.

### Not authored, by design
- **The `info` log of rows deleted per table.** It's observability, not behaviour, and #152 didn't test it either. The `warn` path is pinned through the isolation test's `Ok`. I check both by review in Phase 6.
- **No transaction spanning batches.** I found no deterministic headless test. The pause test proves batches are separate awaits with a sleep between them. I review for a `begin()` in Phase 6.
- **Stop on a short batch, as against stop on an empty batch.** Only one extra pause separates them, which an upper bound on wall time can't assert without flaking. The early-stop test catches a loop that never stops. Phase 6 review covers the rest.
- **A "drop the table" variant of the isolation test.** A's warn W-2 prefers the trigger, which fails only the delete.

## RED confirmation
I ran with `CARGO_BUILD_JOBS=2 RUST_TEST_THREADS=2` and the shared target from `.cargo/config.toml`, on aab847f plus the test files only. `cargo fmt --all -- --check` is clean.

**1. Compile RED for core (the real tree). Only the missing contract module:**
```
cargo test -p gridops-core --test retention --no-run
error[E0432]: unresolved import `gridops_core::retention`
  --> crates/gridops-core/tests/retention.rs:18:19
error[E0277]: can't compare `Duration` with `{integer}`
    --> crates/gridops-core/tests/retention.rs:332:17
```
The second error is a knock-on of the first: `BATCH_PAUSE` is unresolved, so `2 * BATCH_PAUSE` has no type. It disappears once the module exists, as the throwaway run below shows.

**2. Runtime RED behind the compile RED.** I used a throwaway stub of today's behaviour:
- each `prune_batch()` is today's single `DELETE FROM <t> WHERE <col> < ?`;
- `prune` runs it once;
- there is no 0050.

The stub was removed afterwards and never staged.
```
every_retention_batch_finds_its_rows_through_its_cutoff_index ... FAILED
  the host_samples batch must search it through host_samples_received_idx: ["SCAN host_samples"]
a_tables_prune_takes_only_its_rows_older_than_the_cutoff_oldest_first ... FAILED
  WebhookDeliveries: two batches of two   left: 5  right: 4
a_prune_pauses_between_its_batches ... FAILED
  three full batches of two ran           left: 7  right: 6
a_prune_with_a_batch_size_below_one_deletes_nothing ... FAILED
  batch_size 0                            left: 3  right: 0
a_prune_stops_at_the_first_short_batch ... ok        (compile-RED only; mutation M4 below)
every_prune_is_bounded_to_forty_..._with_a_pause_between ... ok   (const pin; compile-RED only)
retention_deletes_find_their_rows_through_an_index ... ok   (unbatched deletes, unchanged)
```
The plan failure is the incident itself: today's statement plans as `SCAN host_samples`.

**3. Reconciler RED (the real tree, no stubs):**
```
cargo test -p gridops-reconciler --bin gridops-reconciler -- snapshot_retention_tests
a_failing_table_prune_does_not_stop_the_other_tables_or_the_pass ... FAILED
  with webhook_deliveries refusing deletes: cleanup returned Err(error returned from database:
  (code: 1811) webhook_deliveries deletes refused by the test), rows left [("webhook_deliveries", 1),
  ("audit_events", 1), ("capacity_samples", 1), ("host_samples", 1), ("reconciler_passes", 1)]
a_pass_deletes_at_most_batch_size_times_max_batches_rows_of_each_table ... FAILED
  webhook_deliveries: one pass deletes the 20000 oldest of 20001 old rows, no more
  left: []   right: [1760847904356]
each_retained_table_keeps_its_own_window ... ok                                  (guard)
retention_keeps_the_configured_days_of_snapshots_seven_by_default ... ok         (#152)
a_failing_snapshot_prune_does_not_stop_the_rest_of_the_cleanup ... ok            (#152)
```
Every seed succeeded, so both failures are assertion failures, not setup failures. The isolation failure shows exactly the bug in the brief: the `?` on the first delete aborts every other prune and fails the pass.

**4. The tests can pass, and they catch mutations.** Everything here used throwaway code, reverted afterwards. `git status` shows only the two staged test files; `main.rs`, `lib.rs` and `migrations/` have no diff.

**The throwaway implementation:**
- **core:** A's design, `retention.rs` with the five `rowid IN (… ORDER BY <col> LIMIT ?)` consts and the shared loop, plus a throwaway 0050;
- **reconciler:** a loop over `RetentionTable::ALL` with per-table cutoffs, `prune(BATCH_SIZE, MAX_BATCHES)`, `info` when the count isn't zero, and `warn` and continue on `Err`.

It made all **7 core** and all **5 reconciler** tests **pass**. With it in place:
- the whole `cargo test -p gridops-core -p gridops-reconciler` exits 0: core has 318 lib tests and every integration binary, including `fleet_schema` (17) and `retention` (7), and the reconciler has 133;
- CI's lint, `cargo +1.98.1 clippy -p gridops-core -p gridops-reconciler --all-targets --locked -- -D warnings -A clippy::unused_async_trait_impl`, is clean.

**Mutation checks (each applied alone to the passing throwaway):**

| # | Mutation | Fails (exact output) |
|---|---|---|
| **M1** | **Delete migration 0050** (the brief's required check) | `every_retention_batch_finds_its_rows_through_its_cutoff_index`: `the host_samples batch must search it through host_samples_received_idx: ["SEARCH host_samples USING INTEGER PRIMARY KEY (rowid=?)", "LIST SUBQUERY 1", "SCAN host_samples USING COVERING INDEX host_samples_host_received_idx", "USE TEMP B-TREE FOR ORDER BY"]`. The other 6 core tests pass, so only the plan test guards the index. |
| M2 | `host_samples` batch `ORDER BY rowid` | The plan test (`"SCAN host_samples", "CREATE BLOOM FILTER"`) and the per-table test (newest rows deleted first) |
| M3 | `audit_events` cutoff `<=` | The per-table test (`left: 2 right: 1`, so the at-cutoff row was deleted). The plan test **passes**: SQLite prints `(created_at<?)` for `<=` too, which is why both tests are needed. |
| M4 | The loop never breaks on a short batch | `a_prune_stops_at_the_first_short_batch`: `the prune did not stop after its short second batch` (timeout at 10 s) |
| M5 | No sleep between batches | `a_prune_pauses_between_its_batches` |
| M6 | No batch-size guard (`-1` passed through as `LIMIT`) | `a_prune_with_a_batch_size_below_one_deletes_nothing`: `left: 3 right: 0` |
| M7 | The `capacity_samples` const deletes from `webhook_deliveries` | The plan test and the per-table test |
| **M8** | **The reconciler propagates a prune error with `?`** (the brief's required isolation check) | `a_failing_table_prune_does_not_stop_the_other_tables_or_the_pass`: `cleanup returned Err(… webhook_deliveries deletes refused by the test)` |
| M11 | The reconciler logs the error but `break`s out of the table loop | The same test: `cleanup returned Ok(()), rows left [("webhook_deliveries", 1), ("audit_events", 1), …]` |
| M9 | The reconciler passes `u32::MAX` instead of `MAX_BATCHES` | `a_pass_deletes_at_most_…_of_each_table`: `left: [] right: [<youngest>]` |
| M10 | Webhook and audit cutoffs swapped | `each_retained_table_keeps_its_own_window`: `left: [<90d+1h>, <90d-1h>] right: [<90d-1h>]` |

## Notes for F
- **The shared target directory hazard (#152).** Another worktree's build can replace this worktree's `gridops-core` artifact and still look fresh. Run `find crates -name '*.rs' -exec touch {} +` before building. Touching also rebuilds `db.rs`, so the `sqlx::migrate!` embed picks up 0050.
- **`RetentionTable` must derive `Clone, Copy, Debug`.** `PartialEq` and `Eq` are harmless but not required.
- **The core tests call `prune` with small `batch_size` and `max_batches`** (2 and 2, 2 and 3, 2 and `u32::MAX`). The function must use its arguments, not the consts.
- **The early-stop test passes `max_batches = u32::MAX`.** Any per-batch cost besides the pause, such as a per-call allocation sized by `max_batches`, would show up there.
- **The isolation test's trigger refuses every DELETE on one table**, in each of the five runs. Each table's error must be logged at `warn` and swallowed, and `cleanup_retention` must return `Ok`.
- **The cap test seeds 20,001 rows per table and expects exactly the youngest to survive.** The reconciler must pass `BATCH_SIZE` and `MAX_BATCHES` (or the equal `snapshot_retention` re-exports).
- **Leave `log_streams` and `enforce_log_storage_budget` as they are.** That part of `cleanup_retention`, above the five prunes, keeps its `?`; no test changes it (A finding 7).

## Ready for F
**RED is valid.** Every new behaviour test fails on its feature assertion:
- **core:** compile RED for the missing `gridops_core::retention` contract, then runtime RED behind it against a stub of today's behaviour;
- **reconciler:** runtime RED with no stubs.

Every test can pass, as the throwaway shows, and each guards a named mutation, including the two the brief requires: M1, deleting 0050, and M8, propagating the prune error. F may implement against these tests.
