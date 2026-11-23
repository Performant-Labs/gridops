# Handoff-T-green: Phase 7 - Lock storm: DELETE FROM host_samples scans 628k rows inside the write lock (#222)

**Date:** 2026-11-22
**Branch:** claude/fix-222-retention-batches
**Issue:** #222
**Handoff-F reviewed:** docs/handoffs/222/handoff-F.md
**Handoff-T-red:** docs/handoffs/222/handoff-T-red.md

## GREEN confirmation
Environment: `CARGO_BUILD_JOBS=2 RUST_TEST_THREADS=2`, shared target, `find crates -name '*.rs' -exec touch {} +` first (so `sqlx::migrate!` embeds 0050).

`cargo test -p gridops-core -p gridops-reconciler`: 21 binaries, **574 passed, 0 failed** (1 pre-existing `#[ignore]` live GitLab test). Includes core `--test retention` 7 passed, `fleet_schema` 17 passed, reconciler `snapshot_retention_tests` 5 passed (the full reconciler run was green). F reported the same numbers; no discrepancy. F flagged no wrong tests, and I changed none.

Mutation spot-checks on F's real code (each reverted with `git checkout`; the tree is clean):
- **M1, delete migration 0050:** `every_retention_batch_finds_its_rows_through_its_cutoff_index` fails: `the host_samples batch must search it through host_samples_received_idx: ["SEARCH host_samples USING INTEGER PRIMARY KEY (rowid=?)", "LIST SUBQUERY 1", "SCAN host_samples USING COVERING INDEX host_samples_host_received_idx", "USE TEMP B-TREE FOR ORDER BY"]`. 6 other core tests pass, so the plan test alone guards the index.
- **M2, host_samples batch `ORDER BY rowid`:** `every_retention_batch_finds_its_rows_through_its_cutoff_index` and `a_tables_prune_takes_only_its_rows_older_than_the_cutoff_oldest_first` fail.
- **M8, `?` on the prune error in `cleanup_retention`:** `a_failing_table_prune_does_not_stop_the_other_tables_or_the_pass` fails: `cleanup returned Err(... (code: 1811) webhook_deliveries deletes refused by the test`.

## Tier 1 results
- `cargo fmt --all -- --check`: PASS.
- `cargo +1.98.1 clippy -p gridops-core -p gridops-reconciler -p gridops-api --all-targets --locked -- -D warnings -A clippy::unused_async_trait_impl`: PASS (exit 0).
- Build/compile of the touched crates and existing tests (incl. #152's `query_plan_tests`, `snapshot_retention_tests`): PASS.
- Server start / curl smoke: N/A (no API or route change; migration 0050 is applied by every test's migrated DB, which the plan test exercises).

## Tier 2 results
- Coverage of each brief criterion: PASS (below).
- Test quality: PASS. Each test names a behavior and has a mutation that fails it. The suite is proportionate: the `a_prune_stops_at_the_first_short_batch` and pause tests use one table because the loop is shared; plan and bound tests run per table because the SQL is five separate consts. No redundant tests found.
- Type safety: PASS. No `any`-style casts; `u64::try_from(batch_size)` guards negative `LIMIT`.
- Error handling: PASS. Per-table failure logs `warn` with the table and continues; pass returns `Ok`. A `batch_size` below 1 deletes nothing.
- SQL safety: PASS. Statements are `const &'static str` selected by an enum; `delete_in_batches` is `pub(crate)` taking `&'static str`; values are bound.
- No transaction spans batches: PASS by review (`retention.rs` has no `begin()`; each batch is `.execute(pool)` autocommit).
- Info log per table when count is nonzero: PASS by review (`Ok(deleted) => tracing::info!(table, deleted, ...)`, `Ok(0)` silent).
- Migration safety: PASS. Index-only, no `IF NOT EXISTS`, per brief; data untouched. Reversible by `DROP INDEX`.
- API contract: N/A (no API change). `snapshot_retention` re-exports keep the three constants' public paths and values (pinned by the const test).
- Playwright: N/A (no UI).
- Evidence appendix: verified every excerpt in `evidence.md` against source and found it verbatim. Line ranges: `main.rs:700`, `main.rs:641-644`, `0027:21-22`, `0001:14`, `0034:5-6`, `0036:9`, `db.rs:45-49` match. The `compose.yaml` range cited as 147-149 is actually 147-149 (`condition: service_healthy` at line 149), consistent. Nothing to append.

## Acceptance criteria status
- Migration 0050, exact contents: PASS (`every_retention_batch_finds_its_rows_through_its_cutoff_index`, M1).
- Batch all five deletes, const SQL, `ORDER BY <col>`: PASS (plan test, per-table test).
- Bound of `BATCH_SIZE * MAX_BATCHES`, newer rows kept, early stop, pause: PASS (`a_tables_prune_takes_only...`, `a_prune_stops_at_the_first_short_batch`, `a_prune_pauses_between_its_batches`, `a_pass_deletes_at_most_batch_size_times_max_batches_rows_of_each_table`, bounds const test).
- Error isolation with warn: PASS (`a_failing_table_prune_does_not_stop_the_other_tables_or_the_pass`, M8).
- Info log per table: PASS by review.
- Per-table cutoffs unchanged: PASS (`each_retained_table_keeps_its_own_window`).

## Blocking issues
None.

## Advisory notes
- Migration number 0050 collides with open PR #221's `0050_fleet_health.sql`; whichever lands second renumbers, and #221's new `cleanup_retention` deletes should become `RetentionTable` variants (A findings 5 and 6, noted by F).
- Out of scope per A finding 7: the `fleet_agent_logs` delete (`main.rs:702`) and the `log_streams` delete / `enforce_log_storage_budget` ahead of the five prunes keep their `?`.
- Not covered by tests, by design: the `info` log and no-spanning-transaction (reviewed above).
