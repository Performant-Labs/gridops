# [Prune fleet_inventory_snapshots: it grows without bound](https://github.com/Performant-Labs/gridops/issues/152)

`fleet_inventory_snapshots` is never pruned. On prod (Mars) it was 711 MB of a 2.3 GB database after 26 days: 181,546 rows, about 7,000 a day across 8 hosts, about 4 KB each, plus 38 MB of index. It slows the pre-deploy `VACUUM INTO` backup and grows the database by about 27 MB a day. No UI change.

This lands right after #179 (PR #180), which fixed a SQLITE_BUSY lock storm on the same database. **Every delete added here must hold the write lock only briefly.** The agent hot paths (heartbeat, inventory, poll) take `BEGIN IMMEDIATE` and wait at most `busy_timeout` (5 s, `crates/gridops-core/src/db.rs`). A single unbounded `DELETE` of ~150k rows of 4 KB would hold the lock for seconds and recreate the storm.

## Who reads the table (checked for this brief)

Every reader reads only the **current** snapshot of a session: the row whose `(host_id, authority_session_id, inventory_sequence)` equals the session's `(host_id, session_id, current_inventory_sequence)` in `fleet_observation_sessions`.

- `registry/inventory.rs` ~line 91-110: idempotent resend of an inventory report. It reads the row at `session.current_inventory_sequence` of the session loaded by `load_current_session`.
- `registry/mod.rs` ~line 1287 (approval proof), `registry/queries.rs` ~line 1134 (backend currency), ~line 1354 and ~line 1912: each joins `snap.inventory_sequence = s.current_inventory_sequence` and requires `s.ended_at IS NULL`.
- Nothing references the table by foreign key. It has an immutable-UPDATE trigger and provenance triggers on INSERT/UPDATE, but no DELETE trigger, so a DELETE is allowed by the schema.
- `receipt_state` is written as `'reconciling'` on every insert (`inventory.rs` ~line 184) and the row is immutable, so the issue's idea of "keep rows an `observing` or `reconciling` receipt still needs" would keep every row. The receipt a resend needs is the current row, which the rule below already keeps. Do not filter on `receipt_state`.

Verify this list against the code before relying on it; if you find another reader of a non-current row, stop and report it.

## Retention rule (decided)

Delete a snapshot row only when **both** hold:

1. `received_at` is older than the retention window (`now - days * 86_400_000`), and
2. it is **not** the current snapshot of a session that has not ended, i.e. there is no `fleet_observation_sessions s` with `s.host_id = snap.host_id AND s.session_id = snap.authority_session_id AND s.current_inventory_sequence = snap.inventory_sequence AND s.ended_at IS NULL`.

The current snapshot of an ended session is pruned like any other old row once it passes the window: every reader requires `ended_at IS NULL`. (Confirm the resend path in `inventory.rs` cannot run against an ended session through `load_current_session`; if it can, keep current snapshots of ended sessions too and say why in the handoff.)

No change-detection ("keep a row when the mappings differ") history: nothing reads history, so it is not built.

## Setting

- New setting key `inventorySnapshotRetentionDays`, default **7**, read in the reconciler with the existing `setting_i64` (`crates/gridops-reconciler/src/main.rs` ~line 3858), clamped to `1..=3650` at use.
- Make it settable through the system settings API (`SystemSettings` in `crates/gridops-api/src/resources.rs` ~line 258) as an **optional** field, following the `macos_vm_slots` precedent (`#[serde(default)] Option<i64>`, "left out, it stays as it is"), validated `1..=3650` like the other retention days, stored and returned like `webhookRetentionDays` (GET returns `stored_i64(&stored, "inventorySnapshotRetentionDays", 7)`), and included in `save_settings`'s `settings.updated` audit JSON as `macosVmSlots` is. Leaving it out of a request must not change it, so the existing web client keeps working unchanged. No web UI change.
- Document the setting, its default and its range where the other retention settings are documented (find it; if no doc lists them, add a short section to `docs/fleet-api.md` or the most fitting doc) and in CHANGELOG.

## Batched delete (the lock-safety requirement)

- Put the prune in a new `pub mod snapshot_retention` in `fleet::registry` (`crates/gridops-core/src/fleet/registry/snapshot_retention.rs`), not in `inventory.rs` (a private module documented as the transactional, audited ingest path). It holds a free fn, e.g. `pub async fn prune_inventory_snapshots(pool: &SqlitePool, cutoff_millis: i64, batch_size: i64, max_batches: u32) -> Result<u64, sqlx::Error>`, with the batch size and max batches as `pub const`s, following the `fleet::targets` free-fn and `registry::logs` precedents. Call it from `cleanup_retention` in the reconciler.
- Add migration `migrations/0049_fleet_inventory_snapshot_retention.sql`, with a header comment in the style of `migrations/0034_retention_indexes.sql`, containing exactly `CREATE INDEX fleet_inventory_snapshots_received_idx ON fleet_inventory_snapshots(received_at);` (no `IF NOT EXISTS`). Without it every batch that finds fewer than 500 eligible rows (every steady-state pass) scans the whole retained table (~190 MB at 7 days) under the write lock; 0034 fixed the same failure for `webhook_deliveries`.
- Each batch is **one autocommit statement** (no surrounding `BEGIN`, no transaction spanning batches), so the write lock is released between batches. Shape:

      DELETE FROM fleet_inventory_snapshots
       WHERE rowid IN (SELECT snap.rowid FROM fleet_inventory_snapshots snap
                        WHERE snap.received_at < ?
                          AND NOT EXISTS (SELECT 1 FROM fleet_observation_sessions s
                                           WHERE s.host_id=snap.host_id
                                             AND s.session_id=snap.authority_session_id
                                             AND s.current_inventory_sequence=snap.inventory_sequence
                                             AND s.ended_at IS NULL)
                        ORDER BY snap.received_at LIMIT ?)

  `received_at` is indexed (migration 0049), so each batch reads only rows older than the cutoff (the eligible ones plus at most one kept current row per open session), however many recent rows the table holds. The statement is a `const` shared by the function and its plan test (below). `ORDER BY snap.received_at` is required: with `ORDER BY snap.rowid` the planner keeps `SCAN snap` even with the index.
- Batch size **500 rows** (a const). About 2 MB of row data per batch, so each batch holds the lock for milliseconds.
- Between batches, sleep a named const of at least the measured per-batch hold (e.g. 25 ms) with `tokio::time::sleep`. The api's writers are in another process and SQLite's busy handler polls with 1-100 ms sleeps, so a `yield_now()` would not let them in.
- Stop when a batch deletes fewer rows than the batch size, or after a per-pass cap of **40 batches (20,000 rows)**. The rest drains on the next reconcile passes. This bounds each pass's work, so the first prune on prod (~150-170k old rows) drains over about 8-9 passes instead of one long pass.
- Log the number of rows deleted per pass at `info` when it is non-zero.
- An error in the prune is logged at `warn` and does not fail the rest of `cleanup_retention` or the reconcile pass.
- Space: SQLite reuses freed pages, so the file stops growing; it does not shrink without `VACUUM`. Do not run `VACUUM` from the code (it would hold the lock for the whole copy). Say this in the PR body.

## Tests (all in gridops-core `*_tests.rs` unless a reconciler wiring test is needed)

1. Seed snapshots across several sessions: for each, an old non-current row, an old **current** row of an open session, a recent non-current row, and an old current row of an **ended** session. After the prune, exactly the old non-current rows and the old ended-session current row are gone; every current snapshot of an open session and every recent row remains.
2. Batch limit: seed more eligible rows than `batch_size * max_batches`; one call deletes exactly `batch_size * max_batches` and returns that count; a second call deletes the rest. Also show a single batch never deletes more than `batch_size` (call with `max_batches = 1`).
3. The settings API: setting `inventorySnapshotRetentionDays` stores it; leaving it out keeps the stored value; values outside `1..=3650` are rejected (api crate, next to the existing settings tests).
4. The reconciler reads the setting with default 7 (a unit test of the cutoff computation, or the existing reconciler test style).
5. Plan test, in the style of `registry/query_plan_tests.rs` (#179): `EXPLAIN QUERY PLAN` of the shared prune const on the migrated schema contains `SEARCH snap USING INDEX fleet_inventory_snapshots_received_idx (received_at<?)` and a `SEARCH` of `fleet_observation_sessions` using an index, and no step starts with `SCAN `.
6. Existing tests that delete snapshot rows (`registry/invariants_tests.rs`) keep passing.

## Timing estimate (required in the result handoff)

Measure locally: seed a file-backed SQLite database (in the scratch dir, not the repo) with 20,000 snapshot rows of ~4 KB `snapshot_json` plus matching sessions, run the prune, and report the per-batch time (median and max) and the total. Keep the seed small: the Mac's disk is nearly full. Extrapolate to prod's ~170k eligible rows and state the expected first-prune lock time per batch and the number of passes. Also measure:

- a steady-state call (all rows recent except a handful eligible): one batch, its time and pages read;
- the build time of the 0049 index on the seed, extrapolated to prod's ~711 MB+ table. The PR body must say the deploy's migration builds this index once, under the write lock, and how long that is expected to take.

## Constraints

- Build with `CARGO_BUILD_JOBS=2` and `RUST_TEST_THREADS=2`. Use the shared cargo target configured in the untracked `.cargo/config.toml` of the main checkout; never set `CARGO_TARGET_DIR`; prefer `cargo test -p <crate>`.
- Out of scope: `webhook_deliveries` retention (mentioned in the issue as related), batching the other existing retention deletes, `host_commands` retention.
- Do not deploy or touch prod.
