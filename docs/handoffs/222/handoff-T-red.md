# Handoff-T-red: Phase 4 - Lock storm: DELETE FROM host_samples scans 628k rows inside the write lock (#222)  (fresh run after the amendment: eight tables)

**Date:** 2026-11-22
**Branch:** claude/fix-222-retention-batches (head 72bde79; only the three test files below are staged)
**Brief / wireframe reviewed:** docs/handoffs/222-brief.md (amended in 20df2d2), docs/handoffs/222/handoff-A.md (PASS, fresh run). Wireframe: N/A (no UI).

**Why T ran again:** the first cycle (b7659e3 to c47dffc) batched five tables. #221 then merged three more retention deletes into `cleanup_retention`. The amended brief covers all eight, and A passed it. This RED adds the three fleet health tables to the existing suites. The first cycle's tests, and its mutation table M1-M11 (in this file's history at b7659e3), still stand for the five original tables.

## A precondition
Confirmed: A returned **PASS** on the plan (Phase 3, fresh run), with 0 blocks and 4 warns. These tests apply the three warns that bear on tests:
- **Warn 1 (reconciler harness):** the reconciler's `TABLES: [Table; 5]` is gone. The table list now comes from an exhaustive `const fn table(RetentionTable) -> Table`, mapped over `RetentionTable::ALL`, so a new variant doesn't compile until it has test rows, and the cap and isolation tests then cover it. Row stamps now step by `Table::step`: 1 ms for most tables, and one hour for the two hourly tables, whose `hour` must be a whole hour and is part of their primary key. The cap and isolation tests start from a whole hour. The fleet health 90-day window stays guarded in one place, `fleet_health_wiring_tests::retention_keeps_90_days_of_fleet_health_rows`, so `each_retained_table_keeps_its_own_window` keeps an explicit list of the five original tables.
- **Warn 2 (stale names):** in `fleet_health_schema.rs` the three `DELETE` rows and their comment are removed. The test is renamed `window_reads_go_through_an_index`, and the module doc points at `tests/retention.rs` for the retention deletes. The file's other rows are unchanged.
- **Warn 3 (core harness):** `expected()` gets three arms whose inserts satisfy #221's `STRICT` DDL: every `NOT NULL` column, the two 9-element JSON arrays, the JSON object, and `"window"` = `'24h'`. Their index is `sqlite_autoindex_<t>_1`. The sorted-names assertion lists eight tables, and the doc comment no longer says "five". Every stamp in that file is a whole hour, because `NOW` = 500,000 h and `DAY` is 24 h.

## Contract the tests fix (F must provide these names)
- **`gridops_core::retention::RetentionTable` gets three variants:** `FleetHealthHourly`, `FleetHostHourly` and `FleetHealthSnapshots`.
- **`ALL` names all eight tables, each once.** The test sorts `as_str()` over `ALL`, so the order is free.
- **The `as_str()` values** are `fleet_health_hourly`, `fleet_host_hourly` and `fleet_health_snapshots`.
- **`prune_batch()`** is one `const` statement per new variant in the existing `rowid IN (SELECT rowid … WHERE <col> < ? ORDER BY <col> LIMIT ?)` shape. The columns are `hour`, `hour` and `recorded_at`.
- **Reconciler:** `cleanup_retention`'s cutoff `match` gets an arm for the three variants on `FLEET_HEALTH_RETENTION_DAYS` (90 days), and #221's three unbatched `sqlx::query(…).await?` deletes are removed. The tests add no new reconciler names.

**Both test binaries fail to compile until the three variants exist.** These are the core `retention` integration binary and the reconciler's bin test target. A RED over new Rust items always works this way (#152, #140, #146, #148, #179, and this run's first cycle). Core's lib tests and its other integration binaries still build and run.

**The reconciler bin test target is the one exception.** While the RED stands, none of its 147 tests compile, not only these three. That's acceptable because the variants are the first thing F adds. It's the price of A's preferred exhaustive `match`, which turns "a new table without test coverage" into a compile error.

## Tests authored (changed; no new test functions)

### Core: `crates/gridops-core/tests/retention.rs`
| Test | What changes, and what it pins | Tier, and why |
|---|---|---|
| `every_retention_batch_finds_its_rows_through_its_cutoff_index` | Now covers **all eight** `prune_batch()` consts (brief: "for each of the eight statements"). The new rows require `SEARCH <t> USING … INDEX sqlite_autoindex_<t>_1 (<col><?)` and no `SCAN`. `ALL` must name the eight tables exactly once. This replaces #221's three hand-written `DELETE` plan rows, so the plan checked is the batched statement production runs. It is still the incident's regression test: it fails when 0051 is removed (M1). | Unit against the real planner, the only place the plan is visible. |
| `a_tables_prune_takes_only_its_rows_older_than_the_cutoff_oldest_first` | Now runs per table over eight. For each new table: strict cutoff, oldest first, `batch_size * max_batches` per call, and only its own table. | Integration on the real schema. Each table has its own const, so each is exercised. |
| `retention_deletes_find_their_rows_through_an_index` | The doc comment only; it no longer says "five". | — |

### Core: `crates/gridops-core/tests/fleet_health_schema.rs`
- The three `DELETE` rows move out, folded into the plan test above.
- The test is renamed to `window_reads_go_through_an_index`, and the module doc is updated.
- This is a fold, not a new test. It passes on the real tree today (5 passed).

### Reconciler: `crates/gridops-reconciler/src/snapshot_retention_tests.rs`
| Test | What changes, and what it pins | Tier, and why |
|---|---|---|
| `a_failing_table_prune_does_not_stop_the_other_tables_or_the_pass` | Now **each of the eight tables** in turn refuses its deletes through a test-only trigger. `cleanup_retention` must return `Ok`, every other table must be pruned, and the snapshot prune must still run. This is brief test 3 for the fleet health tables: today #221's `.await?` on their deletes aborts the cleanup. | Integration through `cleanup_retention`. Isolation is reconciler wiring. |
| `a_pass_deletes_at_most_batch_size_times_max_batches_rows_of_each_table` | Now gives **each of the eight tables** 20,001 old rows. One pass must leave exactly the youngest. This catches an unbounded fleet health delete that remains, even next to a correct batched one. | Same as above. It takes about 10 s, which is 8 tables × 39 pauses of 25 ms plus the seeding. |
| `each_retained_table_keeps_its_own_window` | Its list of the five original tables is now explicit (`WINDOWED`). The fleet health window has its one home in the wiring test (A warn 1). | Guard, unchanged behaviour. |

Unchanged and still relied on:
- `fleet_health_wiring_tests::retention_keeps_90_days_of_fleet_health_rows` (#221) guards the fleet health cutoff arm (M4).
- The early-stop, pause, batch-size and const tests in core are table-independent, so they need no new rows.

### Not authored, by design
These are the same three as the first cycle. They are reviewed in Phase 6, as #152 did:
- the `info` log;
- the absence of a transaction across batches;
- stopping on a short batch, as against an empty one.

## RED confirmation
I ran with `CARGO_BUILD_JOBS=2 RUST_TEST_THREADS=2`, the shared target from the parent `.cargo/config.toml` and `CARGO_TARGET_DIR` unset. I ran `find crates -name '*.rs' -exec touch {} +` before each build (#152's hazard: the shared target directory). `cargo fmt --all -- --check` is clean.

**1. Compile RED on the real tree (72bde79 plus the staged tests). Only the three missing variants:**
```
cargo test -p gridops-core --test retention --no-run
error[E0599]: no variant, associated function, or constant named `FleetHealthHourly` found for enum `RetentionTable` in the current scope
  --> crates/gridops-core/tests/retention.rs:81:25
error[E0599]: no variant ... named `FleetHostHourly` ...       --> crates/gridops-core/tests/retention.rs:91:25
error[E0599]: no variant ... named `FleetHealthSnapshots` ...  --> crates/gridops-core/tests/retention.rs:99:25
error: could not compile `gridops-core` (test "retention") due to 3 previous errors

cargo test -p gridops-reconciler --bin gridops-reconciler --no-run
error[E0599]: no variant ... named `FleetHealthHourly` found for enum `gridops_core::retention::RetentionTable`
   --> crates/gridops-reconciler/src/snapshot_retention_tests.rs:297:25
error[E0599]: ... `FleetHostHourly` ...       --> snapshot_retention_tests.rs:298:25
error[E0599]: ... `FleetHealthSnapshots` ...  --> snapshot_retention_tests.rs:299:25
error: could not compile `gridops-reconciler` (bin "gridops-reconciler" test) due to 3 previous errors
```
`cargo test -p gridops-core --test fleet_health_schema`: 5 passed (the fold).

**2. Runtime RED behind the compile RED, on stub A.** Stub A is a throwaway that adds the three variants to the enum and `ALL`, with correct consts and the reconciler cutoff arm, but **keeps #221's three unbatched `.await?` deletes**. That is today's behaviour for those tables, plus only what the tests need in order to compile. The stub was never staged.
```
cargo test -p gridops-reconciler --bin gridops-reconciler -- snapshot_retention_tests fleet_health_wiring_tests
a_failing_table_prune_does_not_stop_the_other_tables_or_the_pass ... FAILED
  with fleet_health_hourly refusing deletes: cleanup returned Err(error returned from database:
  (code: 1811) fleet_health_hourly deletes refused by the test), rows left [("webhook_deliveries", 0),
  ("audit_events", 0), ("capacity_samples", 0), ("host_samples", 0), ("reconciler_passes", 0),
  ("fleet_health_hourly", 1), ("fleet_host_hourly", 0), ("fleet_health_snapshots", 0)]
a_pass_deletes_at_most_batch_size_times_max_batches_rows_of_each_table ... FAILED
  fleet_health_hourly: one pass deletes the 20000 oldest of 20001 old rows, no more
  left: []   right: [1760850000000]
each_retained_table_keeps_its_own_window ... ok                     (guard)
retention_keeps_the_configured_days_of_snapshots_seven_by_default ... ok
a_failing_snapshot_prune_does_not_stop_the_rest_of_the_cleanup ... ok
fleet_health_wiring_tests::retention_keeps_90_days_of_fleet_health_rows ... ok   (guard)
test result: FAILED. 6 passed; 2 failed
```
Every seed succeeded, including 20,001 hour-stepped rows in each `STRICT` hourly table. So both failures are assertion failures, and they show exactly the defect the brief removes: #221's unbatched deletes are unbounded, and their `?` aborts the cleanup and the pass. On stub A all 7 core `retention` tests pass. The core-side RED is the compile RED; mutations M6 and M7 below show the core tests catching a missing `ALL` entry or a wrong const.

**3. The tests can pass, and they catch mutations.** The full throwaway is stub A with #221's three deletes removed. It passes the whole `cargo test -p gridops-core -p gridops-reconciler`, exit 0:
- core lib: 323 passed;
- every core integration binary, including `retention` (7), `fleet_health_schema` (5) and `fleet_schema` (17);
- reconciler: 136 passed.

`cargo +1.98.1 clippy -p gridops-core -p gridops-reconciler --all-targets --locked -- -D warnings -A clippy::unused_async_trait_impl` is clean.

**Mutation checks.** Each was applied alone to the passing throwaway by `mutate.py` and `run_mutations.sh` in the session scratchpad. The throwaway was restored after each one and reverted at the end.

| # | Mutation | Fails (exact output) |
|---|---|---|
| **M1** | **Delete migration 0051** (the brief's required check) | `every_retention_batch_finds_its_rows_through_its_cutoff_index`: `the host_samples batch must search it through host_samples_received_idx: ["SEARCH host_samples USING INTEGER PRIMARY KEY (rowid=?)", "LIST SUBQUERY 1", "SCAN host_samples USING COVERING INDEX host_samples_host_received_idx", "USE TEMP B-TREE FOR ORDER BY"]`. The other 6 core tests and all reconciler tests pass, so only the plan test guards the index. |
| M2 | The `capacity_samples` batch `ORDER BY rowid` | Plan test (`"SCAN capacity_samples", "CREATE BLOOM FILTER"`), per-table test (`CapacitySamples: the four oldest went first`), cap test (`left: [1760849980000] right: [1760850000000]`) |
| M2b | The `fleet_health_snapshots` batch `ORDER BY rowid` | Plan test (`["SEARCH fleet_health_snapshots USING INTEGER PRIMARY KEY (rowid=?)", "LIST SUBQUERY 1", "SCAN fleet_health_snapshots", "CREATE BLOOM FILTER"]`), per-table test (`FleetHealthSnapshots: the four oldest went first`), cap test (`fleet_health_snapshots: … left: [1760849980000] right: [1760850000000]`) |
| M3 | The `fleet_health_hourly` batch made unbounded (`DELETE FROM fleet_health_hourly WHERE hour < ? AND ? > 0`) | Per-table test (`FleetHealthHourly: two batches of two left: 5 right: 4`), cap test (`fleet_health_hourly: … left: [] right: [1760850000000]`). The plan test **passes**, because the statement still searches the autoindex. That's why both tests are needed. |
| M4 | Fleet health arm given the 31-day `capacity_cutoff` | `retention_keeps_90_days_of_fleet_health_rows`: `fleet_health_hourly: 91 days old is deleted, 89 is kept left: [] right: [1787720400000]` |
| **M5** | **A `?` restored on the prune** (`retention::prune(…).await?;` in place of `prune_retained_table`; the brief's required check) | `a_failing_table_prune_does_not_stop_the_other_tables_or_the_pass`: `with webhook_deliveries refusing deletes: cleanup returned Err(error returned from database: (code: 1811) webhook_deliveries deletes refused by the test` |
| M6 | `ALL` leaves out `FleetHostHourly` | Plan test: `ALL names each retained table once` (`left` lacks `"fleet_host_hourly"`). Wiring test: `fleet_host_hourly: 91 days old is deleted, 89 is kept left: [1787547600000, 1787720400000]` |
| M7 | The `fleet_host_hourly` const deletes from `fleet_health_hourly` | Plan test (`the fleet_host_hourly batch must search it through sqlite_autoindex_fleet_host_hourly_1: ["SEARCH fleet_health_hourly …"]`), per-table test (`FleetHostHourly: two batches of two left: 0 right: 4`), isolation (`cleanup returned Ok(()), rows left [… ("fleet_host_hourly", 1) …]`), cap and wiring tests |

Afterwards, `git status --short` shows only the three staged test files. `crates/gridops-core/src/retention.rs`, `crates/gridops-reconciler/src/main.rs` and `migrations/` have no diff from HEAD.

## Notes for F
- **The shared target directory hazard (#152).** Run `find crates -name '*.rs' -exec touch {} +` before building, so this worktree's `gridops-core` artifact and the `sqlx::migrate!` embed are rebuilt.
- **Variant names are fixed by both test files:** `FleetHealthHourly`, `FleetHostHourly` and `FleetHealthSnapshots`. Add them to `ALL`, which becomes `[Self; 8]`.
- **Remove all three of #221's deletes** at `main.rs:3412-3426` (the comment and the three `sqlx::query(…).await?` calls). If any one stays, the cap test fails (its last old row goes), and so does the isolation test (its `?` returns `Err`).
- **Compute `fleet_health_cutoff` before the `RetentionTable::ALL` loop**, and give the three variants one arm. The 90-day window is pinned by the existing wiring test.
- **The cap test now seeds 8 × 20,001 rows** and takes about 10 s of the reconciler run.
- **Evidence:** in Phase 6 I'll append to `evidence.md` the unchanged-source facts these tests rely on: 0050's `CHECK (hour % 3600000 = 0)`, and the primary keys led by `hour` and `recorded_at`, which give the `sqlite_autoindex_<t>_1` names.

## Ready for F
**RED is valid.**
- **Both binaries** are compile-RED on exactly the three missing variants.
- **Behind that,** the reconciler's isolation and cap tests are runtime-RED against stub A. Stub A keeps #221's unbatched deletes, which is the defect the brief removes.
- **Every test can pass.** The full throwaway passes the whole core and reconciler suite, and clippy is clean.
- **Each change guards a named mutation,** including the two the brief requires: M1 (delete 0051) and M5 (`?` restored on a prune).

F may implement against these tests.
