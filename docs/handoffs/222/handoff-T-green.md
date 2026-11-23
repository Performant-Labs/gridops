# Handoff-T-green: Phase 7 - Lock storm: DELETE FROM host_samples scans 628k rows inside the write lock (#222), eight-table cycle

**Date:** 2026-11-22
**Branch:** claude/fix-222-retention-batches
**Issue:** #222
**Handoff-F reviewed:** docs/handoffs/222/handoff-F.md
**Handoff-T-red:** docs/handoffs/222/handoff-T-red.md

This replaces the first cycle's green handoff (five tables, migration 0050). The numbers here are from this cycle's tree (migration 0051, eight tables).

## GREEN confirmation
Environment: `CARGO_BUILD_JOBS=2 RUST_TEST_THREADS=2`, shared target, `CARGO_TARGET_DIR` unset, `find crates -name '*.rs' -exec touch {} +` first.

`cargo test -p gridops-core -p gridops-reconciler`: exit 0, every binary `ok`, no FAILED or panics. Core `--test retention` 7 passed, `fleet_schema` 17 passed, reconciler bin 136 passed / 11 ignored (155 s, dominated by the eight-table cap test). Same counts as F reported; no discrepancy. F flagged no wrong tests and I changed none.

Mutation spot-checks I ran on F's real code (each reverted; `git status` clean afterwards):
- **M1, move `migrations/0051_host_samples_received_idx.sql` away:** `every_retention_batch_finds_its_rows_through_its_cutoff_index` FAILED: `the host_samples batch must search it through host_samples_received_idx: ["SEARCH host_samples USING INTEGER PRIMARY KEY (rowid=?)", "LIST SUBQUERY 1", "SCAN host_samples USING COVERING INDEX host_samples_host_received_idx", "USE TEMP B-TREE FOR ORDER BY"]`. The other six core tests pass.
- **M-fleet, `fleet_host_hourly` batch `ORDER BY rowid`:** the plan test FAILED (`the fleet_host_hourly batch must search it through sqlite_autoindex_fleet_host_hourly_1: [... "SCAN fleet_host_hourly", "CREATE BLOOM FILTER"]`) and `a_tables_prune_takes_only_its_rows_older_than_the_cutoff_oldest_first` FAILED. So the new variants are guarded, not just the original five.
- M5 (`?` restored on a prune; error isolation test) is F's recorded mutation; the first cycle's T run confirmed the same test fails on that mutation. Not re-run, since the production line is unchanged this cycle.

## Tier 1 results
- `cargo fmt --all -- --check`: PASS.
- `cargo +1.98.1 clippy -p gridops-core -p gridops-reconciler --all-targets --locked -- -D warnings -A clippy::unused_async_trait_impl`: PASS (Finished).
- Build and existing tests (`snapshot_retention_tests`, `fleet_health_wiring_tests`, `fleet_health_schema`): PASS.
- Server start / curl smoke: N/A. No API, route or UI change; the migrated test databases apply 0051.

## Tier 2 results
- Coverage of each brief criterion: PASS (below).
- Test quality: PASS. Each test names a behavior and has a mutation that fails it. Plan and bound tests run per table because there is one `const` per table; the pause, early-stop and batch-size tests use one table because the loop is shared. No redundant tests found. The three hand-written fleet-health `DELETE` plan rows are gone from `fleet_health_schema.rs` and folded into the batched plan test, as the brief asks.
- Type safety: PASS. `u64::try_from(batch_size)` guards a negative `LIMIT`; no casts.
- Error handling: PASS. `prune_retained_table` logs `warn` with the table name and continues; `cleanup_retention` still returns `Ok`. The exhaustive cutoff `match` makes a ninth table a compile error until it has a cutoff and test rows.
- SQL safety: PASS. Statements are `const &'static str` chosen by a closed enum; `delete_in_batches` is `pub(crate)` and takes `&'static str`; values are bound.
- No transaction spans batches: PASS by review (each batch is `.execute(pool)` autocommit).
- `info` per table when non-zero: PASS by review (`Ok(0) => {}`, `Ok(deleted) => info!`).
- #221's three unbatched `.await?` deletes removed from `cleanup_retention`: PASS (diff vs master). The only remaining unbatched deletes (`fleet_agent_logs`, `log_streams`) are explicitly out of scope per the brief and A.
- Migration safety: PASS. Exactly `CREATE INDEX host_samples_received_idx ON host_samples(received_at);` with a header comment, no `IF NOT EXISTS`, data untouched, reversible by `DROP INDEX`. Number 0051 follows master's 0050.
- API contract: N/A. `snapshot_retention` re-exports keep the three constants' paths and values.
- Playwright: N/A (no UI); U is N/A.
- Evidence appendix: re-checked the fleet-health entries against source: `0050_fleet_health.sql:75-76, 95-96, 113-114` and `main.rs:3358` are verbatim; the two `cleanup_retention(app).await?` call sites are at `main.rs:737` and `:680`. Nothing to append.

## Acceptance criteria status
- Migration 0051, exact contents: PASS (`every_retention_batch_finds_its_rows_through_its_cutoff_index`, M1).
- All eight deletes batched with const SQL and `ORDER BY <col>`: PASS (same plan test over eight tables; `a_tables_prune_takes_only_its_rows_older_than_the_cutoff_oldest_first`; M-fleet).
- Bound of `BATCH_SIZE * MAX_BATCHES`, newer rows kept, early stop, pause: PASS (`a_tables_prune_takes_only...`, `a_prune_st[REDACTED-OP-TOKEN]`, `a_prune_pauses_between_its_batches`, `a_pass_deletes_at_most_batch_size_times_max_batches_rows_of_each_table`).
- Error isolation with `warn`, each table in turn: PASS (`a_failing_table_prune_does_not_stop_the_other_tables_or_the_pass`, M5).
- `info` log per table: PASS by review.
- Per-table cutoffs and the 90-day fleet-health window unchanged: PASS (`each_retained_table_keeps_its_own_window`, `retention_keeps_90_days_of_fleet_health_rows`).
- Fleet-health plan checks folded into the `RetentionTable::ALL` plan test: PASS.

## Blocking issues
None.

## Advisory notes
- A failure in the log-stream steps before the loop (`log_streams` delete, `UPDATE`, `enforce_log_storage_budget`) still skips every prune; this is stated in F's Known issues and is outside the brief.
- The eight-table cap test adds about 10 s to the reconciler suite; proportionate, since each table has its own `const`.
