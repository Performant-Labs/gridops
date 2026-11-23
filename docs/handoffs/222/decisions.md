# Decision journal: #222 batch the retention deletes (host_samples lock storm)

## Phase 3 Architecture (A, up-front plan review) - 2026-11-22T20:06:22-08:00
- **Decided:** PASS, with 0 block and 7 warn findings (`docs/handoffs/222/handoff-A.md`).
  - **Placement (the brief left it to A):** a new crate-level `crates/gridops-core/src/retention.rs` (`pub mod retention`). It does not go under `fleet::registry`, because four of the five tables aren't fleet-registry data. It holds:
    - `RetentionTable` (5 variants), with `ALL`, `as_str` and `prune_batch`, which returns one const statement per variant;
    - `prune(pool, table, cutoff, batch_size, max_batches) -> Result<u64, sqlx::Error>`.
  - **Sharing with `snapshot_retention`:**
    - the 500/40/25 ms constants and the batch loop move into `retention`, the loop as `pub(crate) delete_in_batches(pool, &'static str, …)`;
    - `snapshot_retention` re-exports the constants;
    - `prune_inventory_snapshots` delegates to the loop, and its signature, `PRUNE_BATCH` and tests are unchanged.
  - **Layering:** the cutoffs, settings, per-table loop and info/warn logging stay in the reconciler's `cleanup_retention`, as with #152. So the error-isolation test is a reconciler test: it extends `snapshot_retention_tests.rs` and its trigger-based fault injection.
  - **The warns:**
    - (3) extend `crates/gridops-core/tests/retention.rs`, the existing retention plan test over copied literals, instead of adding a second plan-test home;
    - (4) the planner prints `USING COVERING INDEX`, with the exact step strings given;
    - (5) open PR #221 also claims migration 0050, and the second to land renumbers;
    - (6) #221 adds three unbatched deletes to `cleanup_retention`, and the second to land folds them into the enum;
    - (7) the `fleet_agent_logs` delete (main.rs:701) is in the same class but out of scope, so it needs a follow-up.
- **Assumed:**
  - #222, the small hotfix, lands before #221, so 0050 stays as the brief says.
  - The bundled SQLite (libsqlite3-sys 0.37.0, 3.51.3) plans these statements as the 3.54.0 CLI does, as #152's review found for the same shape.
- **Hedged:**
  - Sharing the batch loop with `prune_inventory_snapshots` is a judgment call. The brief permits leaving that function alone "if it can't share code cleanly". I judged that it can: the loop doesn't depend on the statement, and the binds are identical. F may keep them separate only with a concrete written reason.
  - Findings 5 and 6 depend on the merge order of two open PRs, which neither run controls.
  - Finding 7 is scope that I reported, not expanded.
- **Evidence:**
  - **Files read:**
    - the brief and its round-1 result;
    - `snapshot_retention.rs` and `snapshot_retention_tests.rs` (core), and `registry/{mod.rs, tests.rs, query_plan_tests.rs}`;
    - `lib.rs`, `fleet/mod.rs` and `db.rs` (the migrator);
    - `tests/retention.rs`;
    - the reconciler `main.rs` (`reconcile`, `cleanup_retention`, `prune_inventory_snapshots`) and `snapshot_retention_tests.rs`;
    - `compose.yaml` (`depends_on`);
    - migrations 0001, 0022, 0027, 0034, 0036 and 0049;
    - sqlx-core 0.9 `migrate/migrator.rs` and sqlx-sqlite 0.9 `migrate.rs`;
    - #152's `handoff-A.md` and `decisions.md`.
  - **Commands run:**
    - applied migrations 0001-0049 plus the proposed 0050 to a scratch DB, then ran `EXPLAIN QUERY PLAN` on the five batch statements. All five plan as `SEARCH <t> USING COVERING INDEX <idx> (<col><?)`. Without 0050, host_samples plans as `SCAN host_samples USING COVERING INDEX host_samples_host_received_idx` plus a temp B-tree.
    - `ORDER BY rowid` plans as `SCAN` (host_samples, audit_events).
    - #221's 0050 tables plan as a `SEARCH` through their primary-key autoindexes.
    - `git ls-tree` over all branches found `0050_fleet_health.sql` on `claude/fleet-197-contract`.
    - `gh pr view 221` showed it OPEN and MERGEABLE. The master branch protection has `strict: true`.

## Phase 4 Tester (T, RED) - 2026-11-22T20:25:11-08:00
- **Decided:** RED is valid, so F may start (`docs/handoffs/222/handoff-T-red.md`).
  - **Core, in `crates/gridops-core/tests/retention.rs` (extended, A finding 3).** There are 6 new tests, and one existing test is trimmed to the deletes that stay unbatched:
    - a plan test over `RetentionTable::ALL` with an exhaustive expectation `match` (A finding 4's `SEARCH <t> USING …INDEX <idx> (<col><?)` and no `SCAN`);
    - a per-table prune test: strict cutoff, oldest first, bounded by `batch_size * max_batches`, own table only;
    - early stop (`max_batches = u32::MAX` under a 10 s timeout);
    - the pause (#152's lower-bound pattern);
    - batch size below 1;
    - a const pin, which also asserts that `snapshot_retention` shares the constants.
  - **Reconciler, in `snapshot_retention_tests.rs` (extended, A's layering).** There are 3 new tests:
    - per-table error isolation through a refusing trigger, on each of the 5 tables in turn, and `cleanup_retention` must return `Ok`;
    - a per-pass cap of 20,001 rows per table, so exactly the youngest survives;
    - a guard on each table's retention window, with defaults plus the two settings.
  - I kept the cap test, about 5 s, because without it the unbounded deletes could stay wrapped in warn-and-continue with every other test green.
  - I kept the const pin, although #152's rework dropped its literal equalities. Here the pause test and the cap test both read the constants, so only the pin stops a `BATCH_PAUSE = 0` or a huge `MAX_BATCHES` from passing them trivially.
- **Assumed:**
  - `RetentionTable` derives `Clone, Copy, Debug`; the tests need it, and the handoff states it for F.
  - The bundled SQLite plans as observed: these runs used it through sqlx, not the CLI.
- **Hedged:**
  - Not tested: the info log, the absence of a transaction across batches, and stopping on a short batch as against an empty one. These are reviewed in Phase 6, as #152 did.
  - Mutation M3 (`<=`) passes the plan test, because SQLite prints `<?` for `<=`. Only the per-table behaviour test catches it.
- **Evidence:**
  - Compile RED: `E0432 unresolved import gridops_core::retention` (plus its `Duration` knock-on).
  - Runtime RED against a stub of today's behaviour: 4 core tests fail on assertions, and the plan test shows `["SCAN host_samples"]`.
  - Reconciler RED with no stubs: isolation `cleanup returned Err(… webhook_deliveries deletes refused …)`, and cap `left: [] right: [youngest]`.
  - A throwaway correct implementation passes all 12, plus the full core (318 lib and every integration binary) and reconciler (133) suites, with clippy 1.98.1 `-D warnings` and fmt clean.
  - Mutations M1-M11 each fail their test. M1 (delete 0050) shows `SCAN host_samples USING COVERING INDEX host_samples_host_received_idx`, and M8 (`?` on the prune error) shows the isolation test failing.
  - The throwaway was reverted, and only the two test files are staged.

## Phase 5 Feature Implementor (F) - 2026-11-22T20:34:22-08:00
- **Decided:**
  - Implemented A's design as written: `gridops_core::retention` holds `RetentionTable` (ALL, as_str, prune_batch with one const per table), `prune` and the shared `pub(crate) delete_in_batches` loop.
  - The constants moved there from `snapshot_retention`, which re-exports them; `prune_inventory_snapshots` delegates to the shared loop with its signature unchanged.
  - Migration 0050 adds `host_samples_received_idx`.
  - `cleanup_retention` loops over `RetentionTable::ALL` with an exhaustive cutoff `match`. A non-zero count logs at info, and an error logs at warn and the loop continues.
- **Assumed:** #222 lands before PR #221, so it keeps 0050 (A finding 5).
- **Hedged:** The prunes still run after `enforce_log_storage_budget(app).await?`, so a failure there still skips them. That is out of scope (A finding 7).
- **Evidence:**
  - core `--test retention`: 7 passed;
  - reconciler `snapshot_retention_tests`: 5 passed;
  - full `-p gridops-core -p gridops-reconciler`: all ok (core lib 318, reconciler 133);
  - clippy 1.98.1 `-D warnings` on core, reconciler and api: clean;
  - fmt: clean.
  - Source facts are in `docs/handoffs/222/evidence.md`.

## Phase 7 Tester (T, GREEN) - 2026-11-22
- **Decided:** GREEN confirmed with no test changes; verdict PASS. Left the suite as authored since F flagged no wrong tests.
- **Assumed:** None.
- **Hedged:** The `info` log and the absence of a spanning transaction are verified by review only, as in #152.
- **Evidence:**
  - `cargo test -p gridops-core -p gridops-reconciler`: 574 passed, 0 failed.
  - fmt and clippy 1.98.1 `-D warnings` (core, reconciler, api): clean.
  - Mutations on F's real code: deleting 0050 fails the plan test (`SCAN host_samples USING COVERING INDEX host_samples_host_received_idx`); `ORDER BY rowid` fails the plan and per-table tests; `?` on the prune error fails the isolation test (`cleanup returned Err(... webhook_deliveries deletes refused ...)`). All reverted, tree clean.
  - Every excerpt in `evidence.md` checked against source: verbatim.

## Phase 3 Architecture (A, up-front plan review, re-entry after diff-gate r1 with archChanged) - 2026-11-22T20:48:52-08:00
- **Decided:** BLOCK, with 3 block and 3 warn findings (`docs/handoffs/222/handoff-A.md`).
  - **Why:** the brief (blob 0a006ec, unchanged) went stale. PR #221 merged to master as 36dbc1a at 8:26 PM PST (04:26 UTC), 19 minutes after the first pass.
  - **Block 1 [correctness]:** `0050_host_samples_received_idx.sql` collides with master's `0050_fleet_health.sql`. The brief must name 0051.
  - **Block 2 [completeness]:** master's `cleanup_retention` now has eight retention deletes. The brief must cover `fleet_health_hourly`, `fleet_host_hourly` and `fleet_health_snapshots` as `RetentionTable` variants. Otherwise their `?` can still abort the snapshot prune and the pass, which breaks Scope 3.
  - **Block 3 [completeness]:** the first pass deferred both cases to a "land step" that this non-Scion project doesn't have. The driver never merges the base, a CONFLICTING PR stops `mirror-merge-conflict`, and T (next) writes no production code. So O must merge `origin/master`, with a union of the one hunk and a `git mv` to 0051, before a fresh run.
  - **Warns:**
    - #221's literal plan rows in `tests/fleet_health_schema.rs:390-395` move into the `RetentionTable::ALL` plan test;
    - the reconciler test's `TABLES: [Table; 5]` isn't keyed to the enum, and the hourly tables need hour-aligned stamps;
    - the `fleet_agent_logs` follow-up issue is still unopened.
  - **F's code against the approved design (archChanged re-check):** consistent, with no drift. The first pass's placement and sharing decisions stand.
- **Assumed:**
  - The union merge followed by the rename is green. Every existing #222 test covers only the five tables, and #221's deletes and tests are untouched. I didn't build it.
  - The first pass's assumption that "#222 lands before #221" was wrong, and that is recorded here.
- **Hedged:**
  - I assigned the integration merge to O, not to F in this run. The route after an A PASS is T-red first, and T can't resolve a production hunk, so the run can't sequence the merge itself.
  - Warn 5's harness shape is T's call.
- **Evidence:**
  - **git:**
    - `git fetch` and `git log 9e6eaf3..origin/master` show 36dbc1a (#221). `gh pr view 221` shows MERGED at 2026-11-23T04:26:20Z.
    - `git merge-tree --write-tree HEAD origin/master` gives a CONFLICT in `main.rs`, in `cleanup_retention`, and a tree with both 0050 files.
    - `git ls-tree` over every local and remote branch, plus `gh pr list`, show 0051 is free.
  - **Code read:**
    - sqlx-core 0.9.0 `migrate/migrator.rs:255-285`: the applied map is built once, with no duplicate check.
    - The release's `coding-pipeline.workflow.mjs:3492` (the `mirror-merge-conflict` stop), `workflow-coding-pipeline.md` §"The land" (Scion-only) and `tester.md:8,23`.
  - **Query plans:** applied master's migrations plus the index as 0051 to a scratch DB, then ran `EXPLAIN QUERY PLAN` (sqlite3 3.54.0) on all eight batch statements. Each plans as `SEARCH <t> USING COVERING INDEX …`. The three fleet-health tables use `sqlite_autoindex_<t>_1 (<col><?)`. Without 0051, host_samples plans as `SCAN host_samples USING COVERING INDEX host_samples_host_received_idx` plus a temp B-tree.
  - **Files read:**
    - branch: `retention.rs`, `snapshot_retention.rs`, `0050_host_samples_received_idx.sql`, the `main.rs` and `lib.rs` diffs, `tests/retention.rs` and the reconciler's `snapshot_retention_tests.rs`;
    - master: `cleanup_retention`, `0050_fleet_health.sql`, `fleet_health_wiring_tests.rs`, `tests/fleet_health_schema.rs:375-410` and `tests/fleet_schema.rs:120-180`;
    - `docs/handoffs/222-diff-result-r1.md`.

## Phase 3 Architecture (A, up-front plan review, fresh run after the amendment) - 2026-11-22T21:06:30-08:00
- **Decided:** PASS, with 0 block and 4 warn findings (`docs/handoffs/222/handoff-A.md`). The previous pass's three blocks are resolved:
  - (1) the brief (blob a0817fe, 20df2d2) names `0051`, and the branch carries `migrations/0051_host_samples_received_idx.sql`;
  - (2) the brief covers all eight retention deletes, with `FleetHealthHourly`, `FleetHostHourly` and `FleetHealthSnapshots` as `RetentionTable` variants, one cutoff arm on `FLEET_HEALTH_RETENTION_DAYS` (which stays in the reconciler), and #221's three `.await?` deletes removed;
  - (3) merge 6173973 brings in origin/master 36dbc1a as a union in `cleanup_retention`.
  - The first pass's placement and sharing decisions stand. Folding the fleet-health deletes into the enum is the planned extension. At Phase 7, any separate fleet-health delete path counts as a parallel path.
  - **Warns:**
    - (1) the reconciler's `TABLES: [Table; 5]` isn't keyed to the enum, and its millisecond `?-i` stamps break the hourly tables' `CHECK (hour % 3600000 = 0)`;
    - (2) after the fold, `fleet_health_schema.rs`'s test name (:374), module doc (:11) and row comment (:391) go stale;
    - (3) the core `expected()` arms need #221's STRICT insert shapes, and the counts at `tests/retention.rs:151,188-200` and `retention.rs:49` need updating;
    - (4) the `fleet_agent_logs` follow-up issue is still unopened, and the earlier `?`s in `cleanup_retention` (:3384-3399) still skip every prune on failure.
- **Assumed:** the bundled SQLite (3.51.3) plans these statements as the 3.54.0 CLI does. #152's review and this run's earlier T and F runs, all through sqlx, matched the CLI for this shape.
- **Hedged:** I compile-checked the integration merge but didn't test it; T-red runs the suite next. Warns 1-3 are T's and F's to apply inside the planned change.
- **Evidence:**
  - **git:**
    - `git fetch` shows origin/master still at 36dbc1a, and `merge-base --is-ancestor origin/master HEAD` is true.
    - `git diff origin/master HEAD -- crates/gridops-reconciler/src/main.rs` shows the union.
    - `git ls-tree` over every ref finds 0051 only on this branch. The only open PR, #121, touches no migrations.
    - `gh issue list --state all --search fleet_agent_logs` is empty.
  - **Build:** `CARGO_BUILD_JOBS=2 cargo check -p gridops-core -p gridops-reconciler --tests` finished in 28.7 s, with the shared target from the parent `.cargo/config.toml` and `CARGO_TARGET_DIR` unset.
  - **Query plans:** with the branch's migrations 0001-0051 in a scratch DB, `EXPLAIN QUERY PLAN` on all eight batch statements gives `SEARCH <t> USING COVERING INDEX <idx> (<col><?)`. For the three fleet-health tables, `<idx>` is `sqlite_autoindex_<t>_1`. There is no `SCAN`, and no migration declares `WITHOUT ROWID`. Without 0051, `host_samples` gives `SCAN host_samples USING COVERING INDEX host_samples_host_received_idx` plus a temp B-tree.
  - **Files read:**
    - the amended brief and its diff;
    - `222-brief-result-r1.md` and `222-diff-result-r1.md`;
    - both earlier `handoff-A.md` versions (aab847f and 1ac794e);
    - `retention.rs` and `snapshot_retention.rs`;
    - `cleanup_retention` and `prune_retained_table` in the reconciler's `main.rs`;
    - `0050_fleet_health.sql:60-114` and `0051`;
    - `tests/retention.rs:1-300`, `tests/fleet_health_schema.rs:360-405` and the reconciler's `snapshot_retention_tests.rs:150-430`;
    - `fleet_health_wiring_tests.rs:1-88` and `docs/design/fleet-health.md:378-382`.

## Phase 4 Tester (T, RED, fresh run after the amendment) - 2026-11-22T21:23:38-08:00
- **Decided:** RED is valid, so F may start (`docs/handoffs/222/handoff-T-red.md`). No new test functions: the existing suites now cover all eight tables.
  - **Core, `tests/retention.rs`:** three `expected()` arms that satisfy #221's `STRICT` DDL, with index `sqlite_autoindex_<t>_1`; the eight-name `ALL` assertion; the stale "five" comments fixed (A warn 3).
  - **Core, `tests/fleet_health_schema.rs`:** the three hand-written `DELETE` plan rows are folded into the `RetentionTable::ALL` plan test. The test is renamed `window_reads_go_through_an_index`, and the module doc is updated (A warn 2).
  - **Reconciler, `snapshot_retention_tests.rs`:** `TABLES: [Table; 5]` is replaced by an exhaustive `const fn table(RetentionTable)` mapped over `ALL`. Stamps step by `Table::step` (one hour for the two hourly tables), from a whole hour. The isolation and cap tests now cover all eight tables. The window test keeps an explicit list of the five original tables, so the fleet health window has one home, the wiring test (A warn 1).
- **Assumed:** none beyond the first cycle's (the bundled SQLite plans as observed through sqlx).
- **Hedged:**
  - While the RED stands, the reconciler's whole bin test target fails to compile, not only these tests. That's the cost of A's preferred exhaustive `match`, and it ends with F's first edit.
  - M3 (an unbounded fleet health const) passes the plan test. Only the per-table and cap tests catch it.
  - The `info` log, the absence of a spanning transaction, and the short-batch stop are still checked by review in Phase 6.
- **Evidence:**
  - **Compile RED** in both binaries: `E0599 no variant … FleetHealthHourly / FleetHostHourly / FleetHealthSnapshots`, and nothing else.
  - **Runtime RED on stub A** (variants and arm added, #221's three `.await?` deletes kept):
    - isolation: `cleanup returned Err(… fleet_health_hourly deletes refused by the test)`;
    - cap: `fleet_health_hourly … left: [] right: [1760850000000]`.
  - **The full throwaway passes** the whole `cargo test -p gridops-core -p gridops-reconciler` (core lib 323, every integration binary, reconciler 136). Clippy 1.98.1 `-D warnings --all-targets` and fmt are clean.
  - **Mutations M1-M7 and M2b** each fail their test:
    - M1 (delete 0051): `SCAN host_samples USING COVERING INDEX host_samples_host_received_idx`;
    - M5 (`?` on the prune): `cleanup returned Err(… webhook_deliveries deletes refused …)`.
  - The throwaway was reverted. Only the three test files are staged.

## Phase 5 Feature Implementor (F, fresh run after the amendment) - 2026-11-22T21:36:00-08:00
- **Decided:** implementation done, with T's RED now GREEN (`docs/handoffs/222/handoff-F.md`).
  - `RetentionTable` gets `FleetHealthHourly`, `FleetHostHourly` and `FleetHealthSnapshots`, each with its own `const` batch statement ordered by its cutoff column (`hour`, `hour`, `recorded_at`); `ALL` is `[Self; 8]`.
  - `cleanup_retention` computes `fleet_health_cutoff` with the other cutoffs and gives the three variants one arm; #221's three unbatched `.await?` deletes are removed. No separate fleet-health delete path.
  - The brief's two mutation checks (the first diff gate's B-1) are recorded in the handoff: removing 0051 fails the plan test, and restoring `?` on a prune fails the isolation test.
- **Assumed:** none beyond the cycle's (the bundled SQLite plans as the tests observe through sqlx).
- **Hedged:** `archChanged` is reported `true`, because a public enum in core gains variants, even though A approved exactly this extension; a false positive costs one A pass.
- **Evidence:**
  - `cargo fmt --all -- --check` exit 0; `cargo test -p gridops-core -p gridops-reconciler` exit 0 (24 binaries ok, core lib 323, reconciler 136); clippy 1.98.1 `--all-targets -D warnings` exit 0.
  - Mutations: M1 gives `SCAN host_samples USING COVERING INDEX host_samples_host_received_idx`; M5 gives `cleanup returned Err(… webhook_deliveries deletes refused by the test`. Both restored, and both tests green again.
  - `evidence.md`: call-site lines updated after the merge (`main.rs:678-681`, `:737`), and the three fleet-health primary keys and the 90-day constant appended.

## T Phase 7 (GREEN, eight-table cycle)
- **Decided:** PASS. Suite green (core retention 7, fleet_schema 17, reconciler 136), fmt and clippy clean; rewrote handoff-T-green.md because the earlier file described the first (five-table, 0050) cycle.
- **Assumed:** none.
- **Hedged:** M5 was not re-run this cycle; the production line it guards did not change and F's run recorded it failing.
- **Evidence:** ran M1 (0051 moved away) and a new M-fleet (`fleet_host_hourly` `ORDER BY rowid`); both turned the plan test red, then reverted with a clean `git status`.

## A Phase 7 anti-duplication gate (eight-table cycle) - 2026-11-22
- **Decided:** PASS (`docs/handoffs/222/handoff-A-dup.md`). One batch loop (`retention::delete_in_batches`) shared by `RetentionTable::prune` and `prune_inventory_snapshots`; all eight retention deletes go through the enum; no literal retention DELETE left in `cleanup_retention`.
- **Assumed:** none.
- **Hedged:** `fleet_agent_logs` and `log_streams` deletes remain out of scope per the brief; warn only, follow-up issue still owed by O.
- **Evidence:** diff of `crates/` vs origin/master read in full; grep of non-test `DELETE FROM` in crates.
