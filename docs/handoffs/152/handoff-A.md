# Handoff-A: Phase 3 - Prune fleet_inventory_snapshots: it grows without bound (#152)  (up-front plan review, re-review)

**Date:** 2026-11-22
**Branch:** claude/fix-152-prune-snapshots (head 9a543bf; no code yet. The brief is amended in the working tree, blob 10f0e70.)
**Brief reviewed:** docs/handoffs/152-brief.md (amended)   **Reuse map:** there is no `survey.md`. The brief names the extension points:
- `cleanup_retention` and `setting_i64` in `crates/gridops-reconciler/src/main.rs`;
- `SystemSettings`, `get_settings` and `save_settings` in `crates/gridops-api/src/resources.rs`;
- a new `pub mod snapshot_retention` in `crates/gridops-core/src/fleet/registry/`;
- migration 0049.

**Wireframe:** N/A (no UI)
**Verdict:** PASS

## Summary
PASS, with 0 block and 1 warn findings. This review supersedes the BLOCK at commit 9a543bf.

- **The earlier block is fixed.** The amended brief adds migration 0049, `fleet_inventory_snapshots(received_at)`, and orders the batch subquery by `snap.received_at`.
  - I applied all 48 migrations plus the proposed index to a scratch database. The brief's exact statement now plans as an index search on `received_at<?`, with no `SCAN` step.
  - Test 5, the plan test, pins that plan.
- **All four earlier warns are in the brief.** They cover the module placement, the 25 ms sleep between batches, the settings audit field, and the extra timing measurements.
- **The one remaining warn** is that test 5 describes the sessions step by its table name. The planner reports that step by its alias, `s`.

## Findings

| # | Severity | Plan element | Drift dimension | Finding | Suggested fix |
|---|---|---|---|---|---|
| 1 | warn | Tests, item 5: "contains … a `SEARCH` of `fleet_observation_sessions` using an index" | pattern consistency (plan tests match the planner's literal text, as in `query_plan_tests.rs`) | **The planner names the sessions step `s`, not `fleet_observation_sessions`.** On the migrated schema plus 0049, the plan is:<br>`SEARCH fleet_inventory_snapshots USING INTEGER PRIMARY KEY (rowid=?)`<br>`LIST SUBQUERY 2`<br>`SEARCH snap USING INDEX fleet_inventory_snapshots_received_idx (received_at<?)`<br>`CORRELATED SCALAR SUBQUERY 1`<br>`SEARCH s USING INDEX sqlite_autoindex_fleet_observation_sessions_2 (host_id=? AND session_id=?)`<br>- A literal `contains("SEARCH fleet_observation_sessions")` fails against correct code. T would then have a RED test that F can't turn GREEN without changing correct SQL.<br>- The sessions table has three indexes that could each bound this lookup: the `session_id` primary key, the `(host_id, session_id)` unique key, and the partial `fleet_observation_sessions_active_host`. Pinning one autoindex name ties the test to the planner's choice and to the constraint order in 0026. | - Match the alias: some step starts with `SEARCH s USING ` and contains `INDEX`.<br>- Don't pin a sessions index name. The "no step starts with `SCAN `" assertion already guards against a scan.<br>- Keep the `snap` assertion exactly as the brief has it. It matches the planner's text. |

**The earlier findings are resolved:**
- **Block 1 (unbounded scan), resolved.**
  - The brief adds 0049 with exactly `CREATE INDEX fleet_inventory_snapshots_received_idx ON fleet_inventory_snapshots(received_at);`, no `IF NOT EXISTS`, and a header in 0034's style.
  - The subquery is now `ORDER BY snap.received_at LIMIT ?`, and the old "walking by rowid" rationale is gone.
  - The old EXPLAIN criterion is replaced by test 5, which runs on a shared const and rejects any `SCAN ` step.
  - I re-planned the statement exactly as the brief writes it: there is no `SCAN` step and no temp B-tree for the ORDER BY.
  - My earlier `.stats` run measured this exact variant: 21 pages in steady state, compared with the original's 10,044.
- **Warn 2 (placement), resolved.** The prune goes in a new `pub mod snapshot_retention` in `fleet::registry`, as a free fn over `&SqlitePool` returning `Result<u64, sqlx::Error>`, with `pub const`s.
  - **#179 is the precedent for core placement.** The reconciler calls `fleet::retry::retry_failed_start(&app.database, …)` (`crates/gridops-reconciler/src/fleet.rs:636`). Its write-lock SQL is a core const (`LAST_FAILURE`) that `registry/query_plan_tests.rs` plan-tests.
  - **`sqlx::Error` has precedent.** Core functions that only touch the database return it: `fleet::ensure_control_plane` (`fleet/mod.rs:51`), `fleet::store`, `fleet::start_health`, and `autoscaling::waiting_job_pools`.
- **Warn 3 (pause between batches), resolved.** The brief uses `tokio::time::sleep` on a named const of at least the per-batch hold, and gives the cross-process reason. Core already gets tokio with `full` features from the workspace (`Cargo.toml:42`), so `time` needs no new dependency.
- **Warn 4 (settings audit), resolved.** `inventorySnapshotRetentionDays` is added to `save_settings`'s `settings.updated` audit JSON, like `macosVmSlots`. GET returns `stored_i64(&stored, "inventorySnapshotRetentionDays", 7)`.
- **Warn 5 (timing), resolved.** The brief also asks for a steady-state measurement and the index build time on the seed, and the PR body must give the one-time index build time on prod.

**Checked and consistent (no new finding):**
- **Migration 0049 is free, and nothing else in flight claims it.**
  - Master's last migration is `0048_fleet_target_retire.sql`. No local or remote branch has a `0049_` or later migration.
  - The sibling runs don't add migrations: #124 says "no migration", and #130 prefers none.
- **The new index breaks no existing schema test.** The upgrade tests in `crates/gridops-core/tests/fleet_schema.rs` compare only the objects that existed before migrating (`old_schema`) and the row data. A new index is neither, and #183's test checks only that there is exactly one 0048.
- **The bundled SQLite is new enough.** sqlx bundles SQLite 3.51.3 (libsqlite3-sys 0.37.0), and my scratch CLI is 3.54.0. Both use the `SEARCH x USING INDEX … (col<?)` text that the #179 plan tests already assert against the bundled library. Test 5 also re-checks the plan on 3.51.3.
- **Calling the prune from `cleanup_retention` with warn-and-continue fits.**
  - It matches `reconcile()`'s step pattern and `cleanup_retention`'s own handling of file-removal errors.
  - The table deletes in `cleanup_retention` use `?`. The brief explicitly asks for warn-and-continue here so that a slow prune can't skip the deletes after it, which is a reviewed decision.
- **The rest of the brief is unchanged and was checked in the earlier pass:** the reader inventory, the retention rule (including pruning the current row of an ended session), the autocommit statements, and the settings contract.
- **Where the tests go (a note for T, not a finding).** The registry test files are `#[path]` children of `registry/tests.rs` (`delete_tests`, `invariants_tests`, `query_plan_tests`, `retire_tests`). A `snapshot_retention_tests.rs` attached the same way would reuse the existing `Fixture`. Test 5 belongs in `query_plan_tests.rs` itself.

## Notes for O
None required, because the verdict is PASS. Optionally, before T starts, reword Tests item 5's sessions clause as in finding 1: "a step starting `SEARCH s USING ` that uses an index". If you don't, T should apply finding 1 when writing the assertion.

## Patterns referenced
- `migrations/0034_retention_indexes.sql`: retention deletes run under the write lock, and each needs an index on its time column.
- `crates/gridops-core/src/fleet/registry/query_plan_tests.rs` with `crates/gridops-core/src/fleet/retry.rs` (`LAST_FAILURE`) and `crates/gridops-reconciler/src/fleet.rs:636` (#179): write-lock SQL the reconciler runs lives as a core const, with its plan pinned in core.
- `crates/gridops-reconciler/src/main.rs`: `cleanup_retention` (:3290), the agent-log retention that reads `registry::logs::AGENT_LOG_RETENTION_MILLIS` (:697), and `setting_i64`.
- `crates/gridops-core/src/fleet/mod.rs:51`, `fleet/store.rs` and `autoscaling.rs:460`: core database functions that return `sqlx::Error`.
- `crates/gridops-core/tests/fleet_schema.rs` (:105-160, :1346-1410): the upgrade tests compare only the objects that existed before migrating.
