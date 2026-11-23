# [Lock storm: DELETE FROM host_samples scans 628k rows inside the write lock](https://github.com/Performant-Labs/gridops/issues/222)

## What happened on prod

On Mars (build 451), from 7:39 to 7:42 PM PST on 2026-11-22, the api logged 21 `fleet registry dependency failed kind="database" code=5` errors and answered with 503s. Code 5 is SQLITE_BUSY after the 5 s `busy_timeout` (`crates/gridops-core/src/db.rs`).

The reconciler's retention pass held the write lock. `cleanup_retention` in `crates/gridops-reconciler/src/main.rs` (around line 3330–3360) runs `DELETE FROM host_samples WHERE received_at < ?` as one statement. During that window it logged `rows_affected=0` at 6.1, 7.6, 11.7, 11.9 and 13.2 s. It deleted nothing, so the whole cost was the scan: `host_samples` has 628,524 rows, and **no index leads with `received_at`**. Its indexes are `(host_id, observed_at, id)`, `(host_id, received_at, id)` and `(host_id, domain_id, id)`. A normal pass takes about 0.2 s; when Mars's disk is busy it takes seconds, while every other writer waits.

This is the same failure #152 fixed for `fleet_inventory_snapshots` (migration 0049 and batched deletes, PR #189), and the same failure #179 was about (PR #180). **Every delete in the retention pass must hold the write lock only briefly.**

No UI change, no API change, no new setting.

## Scope (decided)

1. **Index.** Add migration `migrations/0051_host_samples_received_idx.sql` (0050 is master's `0050_fleet_health.sql`, from #221; the branch already carries the renamed file after the integration merge). Give it a header comment in the style of `0034_retention_indexes.sql` and `0049`. It contains exactly `CREATE INDEX host_samples_received_idx ON host_samples(received_at);`, with no `IF NOT EXISTS`.
   - Timing: on a copy of prod, creating 0049's index on a 207k-row table took 6.6 s. This one is 628k narrower rows, so expect seconds. That's acceptable at deploy, because migrations run before the services start.
2. **Batch every retention delete in `cleanup_retention`**, not just `host_samples`. That covers:
   - `webhook_deliveries` (`received_at`)
   - `audit_events` (`created_at`)
   - `capacity_samples` (`recorded_at`)
   - `host_samples` (`received_at`)
   - `reconciler_passes` (`started_at`)
   - `fleet_health_hourly` (`hour`), added by #221
   - `fleet_host_hourly` (`hour`), added by #221
   - `fleet_health_snapshots` (`recorded_at`), added by #221

   For the three fleet-health tables: add `RetentionTable` variants `FleetHealthHourly`, `FleetHostHourly` and `FleetHealthSnapshots`, each with its own `const` statement in the same `rowid IN (… ORDER BY <col> LIMIT ?)` shape, an `as_str` arm and a plan-test arm. Their primary key leads with the cutoff column, so the expected plan is `SEARCH <t> USING COVERING INDEX sqlite_autoindex_<t>_1 (<col><?)`. In the reconciler, the cutoff `match` gets one arm for all three using `now - FLEET_HEALTH_RETENTION_DAYS * 86_400_000`; that constant stays in the reconciler. Then remove #221's three unbatched `sqlx::query(...).await?` deletes from `cleanup_retention`.

   The other seven already have an index on their cutoff column (0034 and 0001), but each is still one unbounded statement. The first pass after any long gap, or a retention-setting change, would delete everything old in one write lock. Fix the class, not the instance.
   - Reuse the shape and constants of `fleet::registry::snapshot_retention` (`crates/gridops-core/src/fleet/registry/snapshot_retention.rs`): 500-row batches, a 25 ms `tokio::time::sleep` between batches, a cap of 40 batches per pass, one autocommit statement per batch, and no transaction spanning batches.
   - Each batch has the form `DELETE FROM <t> WHERE rowid IN (SELECT rowid FROM <t> WHERE <col> < ? ORDER BY <col> LIMIT ?)`. Keep `ORDER BY <col>`: #152 found the planner keeps a full scan with `ORDER BY rowid`.
   - Put a small generic helper in gridops-core, next to `snapshot_retention` or in a sibling `retention` module; the architecture reviewer decides where. It should take the table and column from a **closed set of `const` statements**, not interpolated strings. Don't build SQL from runtime input. A `const` statement per table, chosen by an enum, is fine.
   - If the helper can't share code with `prune_inventory_snapshots` cleanly (its WHERE clause differs), leave that function alone; don't force it.
   - `enforce_log_storage_budget` deletes files row by row and isn't part of this change.
3. **Errors.** A failed prune of one table logs a `warn` naming the table, and doesn't stop the other tables' prunes or the reconcile pass. Today the `?` on each delete aborts the rest of `cleanup_retention`, and that changes as part of this.
4. **Logging.** Log rows deleted per table per pass at `info` when the count isn't zero, as the snapshot prune does.

## Tests (gridops-core `*_tests.rs`; reconciler wiring only if needed)

- For **each** of the eight statements, a query-plan test: `EXPLAIN QUERY PLAN` on a migrated database must show the inner select using the cutoff-column index (`SEARCH ... USING INDEX ...<col>...`), with no `SCAN <table>`. It must fail if migration 0051 is removed: the `host_samples` case is the regression test for this incident.
- Fold the three hand-written fleet-health delete plan checks in `crates/gridops-core/tests/fleet_health_schema.rs` (the `DELETE FROM fleet_health_hourly/fleet_host_hourly/fleet_health_snapshots` rows of the `for sql in [...]` list, about lines 390-395) into the `RetentionTable::ALL` plan test, so the batched statements are what's checked. Keep that file's other rows.
- Batching: with more than one batch's worth of old rows, a pass deletes at most `BATCH_SIZE * MAX_BATCHES`. Rows newer than the cutoff are never deleted. The pass stops early when a batch comes back short. It pauses between batches; follow #152's `a_pass_pauses_between_its_batches` pattern.
- Error isolation: when one table's prune fails (for example, the table was dropped in the test database), the other tables are still pruned.
- Each test must fail when the code it guards is broken. Show a mutation check for the plan test and the error-isolation test in the handoff: the plan test goes red when 0051 is removed, and the error-isolation test goes red when a `?` is restored on a prune.

## Constraints

- Keep `CARGO_BUILD_JOBS=2` and `RUST_TEST_THREADS=2`, and use the shared cargo target from the untracked `.cargo/config.toml`. Never set `CARGO_TARGET_DIR`. Prefer `-p gridops-core` and `-p gridops-reconciler` test runs.
- Don't run `VACUUM` from the code.
- PR against master on Performant-Labs/gridops; never upstream iamngoni/gridops.
