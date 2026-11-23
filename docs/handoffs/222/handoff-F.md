# Handoff-F: Phase 5 - Lock storm: DELETE FROM host_samples scans 628k rows inside the write lock (fresh run after the amendment: eight tables)

**Date:** 2026-11-22
**Branch:** claude/fix-222-retention-batches
**Issue:** #222

**Why F ran again:** the first cycle (3e0ec3a) batched five tables. #221 then merged three more retention deletes into `cleanup_retention`. O merged master (6173973), renamed the migration to 0051, and amended the brief. A passed it again (72bde79), and T added the three tables to the existing suites (5d96b7f). This cycle adds the three variants and removes #221's unbatched deletes. It also records the two mutation checks that the first diff gate blocked on (222-diff-result-r1.md, B-1).

## What was done
This cycle's edits:
- `crates/gridops-core/src/retention.rs`: `RetentionTable` gets `FleetHealthHourly`, `FleetHostHourly` and `FleetHealthSnapshots`.
  - `ALL` is now `[Self; 8]`.
  - `as_str` returns `fleet_health_hourly`, `fleet_host_hourly` and `fleet_health_snapshots`.
  - `prune_batch` gives each one its own `const` statement in the existing shape, `DELETE FROM <t> WHERE rowid IN (SELECT rowid FROM <t> WHERE <col> < ? ORDER BY <col> LIMIT ?)`. The columns are `hour`, `hour` and `recorded_at`.
- `crates/gridops-reconciler/src/main.rs` (`cleanup_retention`):
  - `fleet_health_cutoff` is now computed with the other cutoffs, before the loop.
  - The exhaustive cutoff `match` gets one arm for the three variants, using `fleet_health_cutoff`. #221's comment moved onto that arm.
  - #221's three `sqlx::query("DELETE FROM fleet_… WHERE … < ?").execute(..).await?` calls are removed.

The branch's whole production change against master, unchanged from the first cycle apart from the edits above:
- `migrations/0051_host_samples_received_idx.sql`: the header comment, then exactly `CREATE INDEX host_samples_received_idx ON host_samples(received_at);`.
- `crates/gridops-core/src/retention.rs`: the batch constants, the closed `RetentionTable` enum, `prune`, and the shared `pub(crate) delete_in_batches` loop.
- `crates/gridops-core/src/lib.rs`: `pub mod retention;`.
- `crates/gridops-core/src/fleet/registry/snapshot_retention.rs`: re-exports the constants, and `prune_inventory_snapshots` delegates to `delete_in_batches`.
- `crates/gridops-reconciler/src/main.rs`: the `RetentionTable::ALL` loop and `prune_retained_table`, which logs `info` on a non-zero count, logs `warn` on `Err` and carries on.

## Design decisions
- **One cutoff arm for the three tables, on the existing constant.** `FLEET_HEALTH_RETENTION_DAYS` stays in the reconciler beside the other windows, as A placed it. Computing the cutoff before the loop keeps every cutoff in one block. The 90-day value is unchanged, and `fleet_health_wiring_tests::retention_keeps_90_days_of_fleet_health_rows` still guards it.
- **No separate fleet-health path.** The three tables go through `prune_retained_table`, like the other five, so they get the same batching, `info` count and `warn`-and-continue. A said at Phase 3 that a kept literal delete or a helper in `fleet_health` would count as a parallel path.
- **Each statement orders by its cutoff column.** That column is the leading primary-key column, so the planner walks `sqlite_autoindex_<t>_1`. The plan test pins this, and T's M2b shows `ORDER BY rowid` falls back to a `SCAN`.

## Reuse / extend-vs-new
- This cycle creates no new object. It extends `gridops_core::retention::RetentionTable` (three variants and their arms) and `cleanup_retention`'s cutoff `match` (one arm). A approved both objects in the first cycle and confirmed them for eight tables at this run's Phase 3.
- The first cycle's reuse is unchanged:
  - `snapshot_retention`'s loop and constants moved to `crate::retention`, so there is one copy of the loop.
  - A put `retention` there because most of the retained tables aren't fleet-registry data.

## Architecture notes for A
- **Public enum.** `RetentionTable` (pub, in core) gets three variants. Any exhaustive `match` on it outside core would need new arms. The only one is `cleanup_retention`; a grep for `RetentionTable` finds no other consumer outside the tests.
- **Dependency direction is unchanged.** `crate::retention` still imports nothing from `fleet_health`: the table names are SQL text only.
- **No other changes:** no new dependency, no schema change beyond 0051, and no API, UI or setting change.

## Deviations from spec / wireframe
None. In particular, the brief's mutation-check deliverable is met below (the diff gate's B-1 in the first cycle).

## Tier 1 self-check (incl. tests now GREEN)
Environment: `CARGO_BUILD_JOBS=2 RUST_TEST_THREADS=2`, the shared target from the parent `.cargo/config.toml`, and `CARGO_TARGET_DIR` unset. I ran `find crates -name '*.rs' -exec touch {} +` before each build (#152's shared-target hazard).

- `cargo fmt --all -- --check`: exit 0.
- **T's suites are now GREEN:**
  - `cargo test -p gridops-core --test retention --test fleet_health_schema`:
    - `retention`: `test result: ok. 7 passed; 0 failed`. This includes `every_retention_batch_finds_its_rows_through_its_cutoff_index` and `a_tables_prune_takes_only_its_rows_older_than_the_cutoff_oldest_first`, now over eight tables.
    - `fleet_health_schema`: `test result: ok. 5 passed; 0 failed`, including `window_reads_go_through_an_index`.
  - `cargo test -p gridops-reconciler --bin gridops-reconciler -- snapshot_retention_tests fleet_health_wiring_tests`: `test result: ok. 8 passed; 0 failed`. This includes:
    - `a_failing_table_prune_does_not_stop_the_other_tables_or_the_pass`;
    - `a_pass_deletes_at_most_batch_size_times_max_batches_rows_of_each_table`;
    - `each_retained_table_keeps_its_own_window`;
    - `retention_keeps_90_days_of_fleet_health_rows`.
- **The full run,** `cargo test -p gridops-core -p gridops-reconciler`, exits 0. All 24 test binaries report `ok`, with no `FAILED` and no panics:
  - core lib: `323 passed; 0 failed; 19 ignored`;
  - reconciler: `136 passed; 0 failed; 11 ignored`;
  - `retention` 7 and `fleet_schema` 17, among the integration binaries.
- `cargo +1.98.1 clippy -p gridops-core -p gridops-reconciler --all-targets --locked -- -D warnings -A clippy::unused_async_trait_impl`: `Finished`, exit 0.
- **The brief's two required mutation checks,** each applied alone to the GREEN tree by a scratchpad script, then restored:

  | Mutation | Test | Result |
  |---|---|---|
  | M1: move `migrations/0051_host_samples_received_idx.sql` away | `every_retention_batch_finds_its_rows_through_its_cutoff_index` | **FAILED**: `the host_samples batch must search it through host_samples_received_idx: ["SEARCH host_samples USING INTEGER PRIMARY KEY (rowid=?)", "LIST SUBQUERY 1", "SCAN host_samples USING COVERING INDEX host_samples_host_received_idx", "USE TEMP B-TREE FOR ORDER BY"]` |
  | M5: `prune_retained_table(app, table, cutoff).await;` replaced by `retention::prune(&app.database, table, cutoff, retention::BATCH_SIZE, retention::MAX_BATCHES).await?;` | `a_failing_table_prune_does_not_stop_the_other_tables_or_the_pass` | **FAILED**: `with webhook_deliveries refusing deletes: cleanup returned Err(error returned from database: (code: 1811) webhook_deliveries deletes refused by the test` |

  After the restore, `git status --short` showed only `retention.rs` and `main.rs` modified. Both guarded tests passed again on the restored tree (`7 passed` and `1 passed`).
- **The SQLite plan strings (first diff gate NV-1)** come from the bundled SQLite through sqlx; M1's output above is real output. On the GREEN tree, the plan test asserts `SEARCH <t> USING … INDEX <idx> (<col><?)` for all eight tables, and it passes.

## Evidence appendix
`docs/handoffs/222/evidence.md`. This cycle:
- updated the `cleanup_retention(app).await?` call-site lines, now `main.rs:737` and `:678-681` after the merge. A repo-wide grep finds no other production call site, which answers the first diff gate's NV-4;
- updated the migration number to 0051 in the index facts;
- appended the three fleet-health primary keys (`0050_fleet_health.sql:75-76, 95-96, 113-114`), which are rowid `STRICT` tables led by the cutoff column, and `FLEET_HEALTH_RETENTION_DAYS` (`main.rs:3357-3358`).

## Tests that look wrong (for T)
None.

## Known issues
- **Out of scope, per A's finding 4. These stay as they are:**
  - `DELETE FROM fleet_agent_logs WHERE received_at < ?` in `reconcile()` (`main.rs:739`), still one unbounded `.await?`. The follow-up issue, which O opens before the PR, makes it one more `RetentionTable` variant.
  - The `log_streams` delete, the `UPDATE log_streams` and `enforce_log_storage_budget` ahead of the prunes, which keep their `?`.
- **What the change guarantees, and no more:** a failed prune of one table doesn't stop the other tables' prunes, the snapshot prune or the pass. A failure in the log-stream steps before the loop still skips every prune.

## Files changed
- migrations/0051_host_samples_received_idx.sql
- crates/gridops-core/src/retention.rs
- crates/gridops-core/src/lib.rs
- crates/gridops-core/src/fleet/registry/snapshot_retention.rs
- crates/gridops-reconciler/src/main.rs

Changed in this cycle: `crates/gridops-core/src/retention.rs`, `crates/gridops-reconciler/src/main.rs`.
