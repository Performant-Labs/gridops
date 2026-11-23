# Handoff-A: Phase 3 - Lock storm: DELETE FROM host_samples scans 628k rows inside the write lock (#222)  (up-front plan review, fresh run after amendment)

**Date:** 2026-11-22
**Branch:** claude/fix-222-retention-batches (head 20df2d2; origin/master 36dbc1a is an ancestor, through integration merge 6173973)
**Brief reviewed:** docs/handoffs/222-brief.md (blob a0817fe, amended in 20df2d2 from 0a006ec)   **Reuse map:** there is no `survey.md`. The brief names the extension points: `gridops_core::retention` (`RetentionTable`, `prune`, `delete_in_batches`), `cleanup_retention` in the reconciler, and `tests/retention.rs`.   **Wireframe:** N/A (no UI)
**Verdict:** PASS

**Why A ran again:** the previous pass (1ac794e) blocked with 3 block findings because PR #221 merged under the run. O then merged master into the branch (6173973) and amended the brief (20df2d2). This is the fresh run's up-front review.

## Summary
PASS, with 0 block and 4 warn findings. All three earlier block findings are resolved:
- the brief names `0051`, and the branch carries `migrations/0051_host_samples_received_idx.sql`;
- the brief covers all eight retention deletes, with the three fleet-health tables as `RetentionTable` variants;
- `origin/master` (36dbc1a) is merged in, and the result compiles.

On a scratch database with the branch's migrations 0001-0051, all eight proposed batch statements plan as a `SEARCH … USING COVERING INDEX` with no `SCAN`. Without 0051, `host_samples` falls back to a `SCAN`. The first pass's placement and sharing decisions stand. Folding the three fleet-health deletes into the closed enum is the planned extension of the approved object, not a parallel path.

## Earlier BLOCK findings, re-checked

| Earlier # | Was | Now | Evidence |
|---|---|---|---|
| 1 | [correctness] migration 0050 collides with master's `0050_fleet_health.sql` | **Resolved** | Brief Scope 1 and Tests name `0051`, and `migrations/` on the branch ends at `0050_fleet_health.sql`, `0051_host_samples_received_idx.sql`. `git ls-tree` over every local and remote ref finds 0051 only on this branch. The one open PR (#121) touches no migrations. `retention.rs:41` and `tests/retention.rs:64,183` already say 0051. |
| 2 | [completeness] master's `cleanup_retention` has eight retention deletes, and the brief covered five | **Resolved** | Brief Scope 2 lists all eight. It adds the variants `FleetHealthHourly`, `FleetHostHourly` and `FleetHealthSnapshots`, each with its own const, an `as_str` arm and a plan arm. It gives one cutoff arm on `FLEET_HEALTH_RETENTION_DAYS`, which stays in the reconciler, and it removes #221's three `.await?` deletes. Tests: "each of the eight statements", and the #221 literal rows fold into the `ALL` plan test. |
| 3 | [completeness] no step in the run could integrate master | **Resolved** | Merge 6173973 (parents 1ac794e and 36dbc1a). `git merge-base --is-ancestor origin/master HEAD` is true, and `git fetch` shows master still at 36dbc1a. `git diff origin/master HEAD -- crates/gridops-reconciler/src/main.rs` shows the union: the branch's `RetentionTable::ALL` loop, then #221's `fleet_health_cutoff` and its three deletes verbatim. `CARGO_BUILD_JOBS=2 cargo check -p gridops-core -p gridops-reconciler --tests` passes (shared target from the parent `.cargo/config.toml`, `CARGO_TARGET_DIR` unset). |

## Findings

| # | Severity | Plan element | Drift dimension | Finding | Suggested fix |
|---|---|---|---|---|---|
| 1 | warn | Tests: batching and error isolation over eight tables (reconciler) | abstraction level (a parallel list), test-harness fit | **`crates/gridops-reconciler/src/snapshot_retention_tests.rs:248` keeps its own `TABLES: [Table; 5]`, which isn't keyed to `RetentionTable::ALL`.** So a new variant compiles without isolation or cap coverage.<br>- Its `rows()` (:155-171) stamps each row `? - i`, one millisecond apart, from an unaligned `now_millis()`. That breaks `CHECK (hour % 3600000 = 0)` on `fleet_health_hourly` and `fleet_host_hourly` (`0050_fleet_health.sql:61,83`), whose composite primary keys also need distinct values.<br>- The 90-day window is already guarded through `cleanup_retention` by `fleet_health_wiring_tests.rs::retention_keeps_90_days_of_fleet_health_rows`. | This is T's call:<br>- extend the isolation test and the cap test to all eight tables, with hour-aligned stamps that step by one hour for the two hourly tables;<br>- preferably derive the list from an exhaustive `match` on `RetentionTable`, so the next variant fails to compile;<br>- keep one home for the fleet-health window guard (the existing wiring test) instead of adding the three to `each_retained_table_keeps_its_own_window`. |
| 2 | warn | Tests: "Fold the three hand-written fleet-health delete plan checks in `fleet_health_schema.rs` … Keep that file's other rows" | naming (stale test name and doc) | **When the three rows (`tests/fleet_health_schema.rs:391-394`) leave, the file still claims to cover them:**<br>- the test name `window_reads_and_retention_deletes_go_through_an_index` (:374);<br>- the module doc, "and an index under each 90-day retention `DELETE`" (:11);<br>- the row comment "The three 90-day retention deletes (#191, #200, #204)" (:391).<br>The brief names this edit to #221's merged file, so it is a planned change, not a drive-by. | Drop the comment with the rows, rename the test to match what's left (for example `window_reads_go_through_an_index`), and point the module-doc clause at `tests/retention.rs`'s `RetentionTable::ALL` plan test. |
| 3 | warn | Tests: the core harness in `tests/retention.rs` over eight tables | pattern consistency (one exhaustive expectation) | **The `expected()` match (:39-77) gets three arms. Their insert shapes must satisfy #221's STRICT DDL:**<br>- `fleet_health_hourly` needs every NOT NULL column, including two 9-element JSON arrays;<br>- `fleet_host_hourly` needs `start_failures_json` as a JSON object;<br>- `fleet_health_snapshots` needs `"window"` in `('1h','24h','7d')`.<br>The existing stamps (`CUTOFF - n * DAY`, `CUTOFF`, `RECENT`) are hour-aligned, because `NOW = 1_800_000_000_000` is exactly 500,000 hours, so the per-table test fits as written. Several stale counts also need updating:<br>- the sorted-names assertion (:188-200) lists five;<br>- the doc comment at :151 says "The five batched ones";<br>- `ALL: [Self; 5]` (`retention.rs:49`). | - Add the three arms with index `sqlite_autoindex_<t>_1`. These are SQLite's deterministic names for the primary-key autoindexes, confirmed below.<br>- Keep every new stamp hour-aligned.<br>- Update the counts as part of the change. No new harness is needed. |
| 4 | warn | The brief's headline invariant against its scope (the first pass's finding 7 and the second pass's finding 6) | scope (reported, not expanded) | **The `fleet_agent_logs` follow-up issue still isn't open.** `gh issue list --state all --search fleet_agent_logs` returns nothing. `DELETE FROM fleet_agent_logs WHERE received_at < ?` is still one unbounded `.await?` in `reconcile()` (`main.rs:739`), right after `cleanup_retention`.<br>Also, earlier in `cleanup_retention` (`main.rs:3384-3399`), the `log_streams` expiry delete, the `UPDATE log_streams` and `enforce_log_storage_budget` all still end in `?`. So a failure there still skips every prune. | - O opens the follow-up. In this design it is one more `RetentionTable` variant.<br>- F's handoff and the PR body must claim only "one table's prune failure doesn't stop the other tables' prunes or the pass", not "nothing stops the prunes". |

## Notes for O
None are required, because the verdict is PASS. The warns need these actions:
- **Findings 1-3:** T and F apply them as written. Each is a test-harness or naming detail inside the planned change.
- **Finding 4:** open the `fleet_agent_logs` batching follow-up issue before the PR is opened, and link it from the PR body.

## Placement and layering, confirmed for the eight-table plan (recorded for T and F)
- **Core.** The three new statements go in `gridops_core::retention::RetentionTable::prune_batch`, as `&'static str` consts in the existing `DELETE FROM <t> WHERE rowid IN (SELECT rowid FROM <t> WHERE <col> < ? ORDER BY <col> LIMIT ?)` shape.
  - `ALL` becomes `[Self; 8]`.
  - `crate::retention` still imports nothing from `fleet_health`: the table names are SQL text only. So the dependency direction is unchanged, with `fleet::registry` → `crate::retention` and nothing back.
  - The other tables in the enum (webhooks, audit, autoscaling) are also owned by other modules. So putting fleet-health's age-only prunes there follows the established cadence. `snapshot_retention` keeps its own statement only because its WHERE clause differs.
- **Reconciler.** `cleanup_retention`'s exhaustive cutoff `match` gets one arm, `FleetHealthHourly | FleetHostHourly | FleetHealthSnapshots => fleet_health_cutoff`.
  - Compute `fleet_health_cutoff` before the loop, beside the other cutoffs.
  - `FLEET_HEALTH_RETENTION_DAYS` (`main.rs:3358`) stays where it is, like the 31-day capacity literal and the webhook and audit defaults.
  - Then delete the three `sqlx::query(…).await?` calls (`main.rs:3415-3426`).
  - `prune_retained_table` covers the new tables unchanged: `Ok(0)` → nothing, `Ok(n)` → `info!`, `Err` → `warn!`, each with `table = as_str()`.
- **No new object.** The change is three variants and their arms in the objects the first pass approved and F built. At Phase 7, any separate delete path for the fleet-health tables (a helper in `fleet_health`, or a kept literal delete) is a parallel path.

## Checked and consistent (no finding)
- **Rowid tables.** No migration declares `WITHOUT ROWID` (grep over `migrations/*.sql`). The three fleet-health tables are `STRICT` rowid tables whose composite primary key leads with the cutoff column (`0050_fleet_health.sql:75,95,113`). So the `rowid IN (…)` shape is valid for all eight (brief-review NV-1 and W-4).
- **Query plans,** on a scratch database with the branch's `migrations/*.sql` (0001-0051) in the sqlite3 3.54.0 CLI. Each outer step is `SEARCH <t> USING INTEGER PRIMARY KEY (rowid=?)`. The inner steps are:
  - `SEARCH webhook_deliveries USING COVERING INDEX webhook_deliveries_received_idx (received_at<?)`
  - `SEARCH audit_events USING COVERING INDEX audit_events_created_idx (created_at<?)`
  - `SEARCH capacity_samples USING COVERING INDEX capacity_samples_recorded_idx (recorded_at<?)`
  - `SEARCH host_samples USING COVERING INDEX host_samples_received_idx (received_at<?)`
  - `SEARCH reconciler_passes USING COVERING INDEX reconciler_passes_started_idx (started_at<?)`
  - `SEARCH fleet_health_hourly USING COVERING INDEX sqlite_autoindex_fleet_health_hourly_1 (hour<?)`
  - `SEARCH fleet_host_hourly USING COVERING INDEX sqlite_autoindex_fleet_host_hourly_1 (hour<?)`
  - `SEARCH fleet_health_snapshots USING COVERING INDEX sqlite_autoindex_fleet_health_snapshots_1 (recorded_at<?)`
- **Without 0051,** `host_samples` plans as `SCAN host_samples USING COVERING INDEX host_samples_host_received_idx` plus `USE TEMP B-TREE FOR ORDER BY`. So the existing plan test's no-`SCAN` assertion stays the incident's regression guard, and the brief's mutation check (remove 0051 → red) is meaningful.
- **Brief prose,** "the other seven already have an index on their cutoff column (0034 and 0001)": `reconciler_passes_started_idx` is from 0036, and the three fleet-health indexes are the primary-key autoindexes from 0050. This changes nothing, because `expected()` pins the real index names.
- **Design doc.** `docs/design/fleet-health.md:378-382` ("each deleted through its primary key's leading time column") stays accurate after batching, so it needs no edit.

## Patterns referenced
- `crates/gridops-core/src/retention.rs` and `crates/gridops-core/src/fleet/registry/snapshot_retention.rs`: the approved closed-set enum and the shared batch loop.
- `crates/gridops-reconciler/src/main.rs:3358-3451`: `cleanup_retention` after the union merge, and `prune_retained_table`.
- `migrations/0050_fleet_health.sql:60-114`: the fleet-health DDL (STRICT rowid tables, PKs led by the time column, the hour CHECK).
- `crates/gridops-core/tests/retention.rs`, `crates/gridops-core/tests/fleet_health_schema.rs:374-405` and `crates/gridops-reconciler/src/snapshot_retention_tests.rs:200-430`: the plan-test home, #221's literal rows, and the reconciler harness.
- `crates/gridops-reconciler/src/fleet_health_wiring_tests.rs:38-86`: the existing 90-day window guard.
