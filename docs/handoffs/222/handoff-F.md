# Handoff-F: Phase 5 - Lock storm: DELETE FROM host_samples scans 628k rows inside the write lock

**Date:** 2026-11-22
**Branch:** claude/fix-222-retention-batches
**Issue:** #222

## What was done
- `migrations/0050_host_samples_received_idx.sql` (new): a header comment in the 0034/0049 style, then exactly `CREATE INDEX host_samples_received_idx ON host_samples(received_at);`.
- `crates/gridops-core/src/retention.rs` (new), placed where A decided:
  - `BATCH_SIZE` (500), `MAX_BATCHES` (40) and `BATCH_PAUSE` (25 ms), moved here from `snapshot_retention`. The doc comments are rewritten so they hold for every table.
  - `RetentionTable`, deriving `Clone, Copy, Debug, PartialEq, Eq`, with five variants, `ALL`, `const fn as_str` and `const fn prune_batch`. `prune_batch` returns one `const` statement per variant: `DELETE FROM <t> WHERE rowid IN (SELECT rowid FROM <t> WHERE <col> < ? ORDER BY <col> LIMIT ?)`.
  - `pub async fn prune(pool, table, cutoff_millis, batch_size, max_batches) -> Result<u64, sqlx::Error>`.
  - `pub(crate) async fn delete_in_batches(pool, statement: &'static str, ...)`: the shared loop, moved here unchanged from `prune_inventory_snapshots`. It keeps the negative-`LIMIT` guard, the pause between batches, the stop on a short batch and the autocommit `.execute(pool)` per batch.
- `crates/gridops-core/src/lib.rs`: `pub mod retention;`, between `provisioning` and `tart_host`.
- `crates/gridops-core/src/fleet/registry/snapshot_retention.rs`: now re-exports the three constants with `pub use crate::retention::{...}`. `prune_inventory_snapshots` keeps its signature, doc and `PRUNE_BATCH`; its body is now one `delete_in_batches` call.
- `crates/gridops-reconciler/src/main.rs`:
  - `cleanup_retention` replaces the five unbounded `DELETE ... ?` statements with a loop over `RetentionTable::ALL`. The cutoff comes from an exhaustive `match`, so a new variant without a cutoff won't compile.
  - The new `prune_retained_table` does nothing on `Ok(0)`, logs `info!(table, deleted)` on `Ok(n)`, and logs `warn!(table, error)` and continues on `Err`.
  - The fn-local alias in `prune_inventory_snapshots` is renamed `retention` -> `snapshots`, so `retention::` means only `gridops_core::retention` (A's note).

## Design decisions
- **Shared loop (A finding 2):** I moved the loop instead of copying it. `&'static str` plus `pub(crate)` keeps the set of statements closed: a `format!` result is a `String`, and nothing outside core can pass a statement in.
- **Cutoff per table stays in the reconciler.** The settings reads (`webhookRetentionDays`, `auditRetentionDays`) and the fixed 31 days are unchanged, so `each_retained_table_keeps_its_own_window` stays green. Capacity samples, machine samples and reconciler passes share one match arm, and the comment about the workload chart moved onto that arm.
- **What runs before the five prunes is untouched.** The `log_streams` delete, the `UPDATE log_streams` and `enforce_log_storage_budget` keep their `?` (brief, and A finding 7). So the guarantee is "one table's prune failure doesn't stop the other tables, the snapshot prune or the pass". It is not "no failure stops the prunes".
- **The log message is generic** ("pruned old rows" / "could not prune old rows"). The `table` field tells the tables apart, which keeps it to one message per outcome.

## Reuse / extend-vs-new
- I extended `fleet::registry::snapshot_retention`. Its loop and constants moved to the crate-level `retention` module, and `prune_inventory_snapshots` delegates to them, so there is one copy of the loop.
- `retention` is new only because A placed it there (handoff-A finding 1: four of the five tables aren't fleet-registry data). Dependency direction: `snapshot_retention` -> `retention`, never the reverse.

## Architecture notes for A
- New public core module `gridops_core::retention` (enum, consts, `prune`), plus a crate-private `delete_in_batches`.
- `snapshot_retention`'s constants are now re-exports. Their public paths and values are unchanged.
- One new migration (0050), an index only. **It collides with open PR #221's `0050_fleet_health.sql` (A finding 5): whichever lands second renumbers.** PR #221's three new `cleanup_retention` deletes must become `RetentionTable` variants (A finding 6).
- No new dependencies. No API or UI changes, and no new setting.

## Deviations from spec / wireframe
None.

## Tier 1 self-check (incl. tests now GREEN)
- Command environment: `CARGO_BUILD_JOBS=2 RUST_TEST_THREADS=2`, the shared target from `../../../.cargo/config.toml`, and `find crates -name '*.rs' -exec touch {} +` beforehand.
- `cargo fmt --all -- --check`: clean (exit 0).
- `cargo test -p gridops-core --test retention`: `test result: ok. 7 passed; 0 failed`
  - `a_prune_st[REDACTED-OP-TOKEN]`, `a_prune_pauses_between_its_batches`, `a_prune_with_a_batch_size_below_one_deletes_nothing`, `every_prune_is_bounded_to_forty_batches_of_five_hundred_with_a_pause_between`, `every_retention_batch_finds_its_rows_through_its_cutoff_index`, `a_tables_prune_takes_only_its_rows_older_than_the_cutoff_oldest_first` and `retention_deletes_find_their_rows_through_an_index`: all ok.
- `cargo test -p gridops-reconciler --bin gridops-reconciler -- snapshot_retention_tests`: `test result: ok. 5 passed; 0 failed`
  - `a_failing_table_prune_does_not_stop_the_other_tables_or_the_pass`, `a_pass_deletes_at_most_batch_size_times_max_batches_rows_of_each_table`, `each_retained_table_keeps_its_own_window`, and the two #152 tests: all ok.
- `cargo test -p gridops-core -p gridops-reconciler` (full run): every binary reports `ok`, with no `FAILED` and no panics. Core lib has 318 passed (this includes #152's `query_plan_tests` over `PRUNE_BATCH` and the core `snapshot_retention_tests`). `fleet_schema` has 17 passed, `retention` 7 passed, and the reconciler 133 passed.
- `cargo +1.98.1 clippy -p gridops-core -p gridops-reconciler --all-targets --locked -- -D warnings -A clippy::unused_async_trait_impl`: finished, exit 0.
- The same clippy on `-p gridops-api`, which uses `snapshot_retention`'s settings items: finished, clean.
- Mutation checks: T's handoff-T-red already ran M1-M11 against an equivalent throwaway, and running them again is T's Phase 6.

## Evidence appendix
`docs/handoffs/222/evidence.md`. It covers:
- the two `cleanup_retention(app).await?` call sites;
- the `host_samples` index that leads with `host_id` (0027);
- the cutoff indexes of the other four tables (0001, 0034, 0036);
- migrations running before the API serves (`db.rs`);
- the reconciler's `depends_on: api: service_healthy` (`compose.yaml`).

## Tests that look wrong (for T)
None.

## Known issues
- **Out of scope, per A finding 7.** These stay unbatched or keep their `?`:
  - the `fleet_agent_logs` retention delete in `reconcile()` (`main.rs:702`);
  - the `log_streams` delete and `enforce_log_storage_budget` (`?`) ahead of the five prunes.
- **Migration number collision with PR #221 (both use 0050).** It is resolved at land time by renumbering whichever lands second.

## Files changed
- migrations/0050_host_samples_received_idx.sql
- crates/gridops-core/src/retention.rs
- crates/gridops-core/src/lib.rs
- crates/gridops-core/src/fleet/registry/snapshot_retention.rs
- crates/gridops-reconciler/src/main.rs
