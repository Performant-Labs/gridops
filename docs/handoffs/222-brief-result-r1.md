## Brief Review (Round 1)

### BLOCK findings
None.

### Needs-verification findings
- **[NV-1]** The claim that the three fleet-health tables' "primary key leads with the cutoff column, so the expected plan is `SEARCH <t> USING COVERING INDEX sqlite_autoindex_<t>_1 (<col><?)`" — and, critically, that those tables *have* a `rowid` at all (the batched statement is `DELETE … WHERE rowid IN (…)`). If the PK is `(hour, …)` and the table is `WITHOUT ROWID`, the shared batched statement shape is invalid for those three tables and the brief's "one arm for all three" collapses. Settle with the `CREATE TABLE` DDL for `fleet_health_hourly`, `fleet_host_hourly` and `fleet_health_snapshots` (migrations, likely 0050_fleet_health.sql).
- **[NV-2]** The claim that #152 found the planner keeps a full scan with `ORDER BY rowid` (hence `ORDER BY <col>` is kept) — cite the #152 / PR #189 analysis or a current `EXPLAIN QUERY PLAN` run on a migrated database.
- **[NV-3]** The claim that the other seven tables already have an index on their cutoff column (0034 and 0001), so their plan tests will pass — cite the DDL/index list for `webhook_deliveries(received_at)`, `audit_events(created_at)`, `capacity_samples(recorded_at)`, `reconciler_passes(started_at)` from migrations 0001/0034.
- **[NV-4]** The claim that migrations run before services start at deploy, making a multi-second index build on 628k rows "acceptable" — cite the migration runner's startup ordering (e.g. `crates/gridops-core/src/db.rs` or service entrypoints).
- **[NV-5]** The claim that `snapshot_retention` (500-row batches, 25 ms sleep, 40-batch cap, autocommit per batch) exists as described at `crates/gridops-core/src/fleet/registry/snapshot_retention.rs` and can be reused — cite that file.

### WARN findings
- **[W-1]** Throughput ceiling vs. backlog: 40 batches × 500 rows = 20,000 rows deleted per pass across *each* table (if the cap is per table). With 628k rows in `host_samples` and no leading `received_at` index today, an existing backlog could take dozens of passes to drain, and the brief sets no acceptance criterion on drain time or cap adequacy. Recommend the brief state the expected steady-state deletion rate vs. insertion rate, or justify that 20k/pass per table is sufficient.
- **[W-2]** Missing test for the logging criterion (Scope item 4): "log rows deleted per table at `info` when non-zero" has no corresponding test in the Tests section, unlike every other criterion. Either add a test or explicitly mark it as handoff-verified.
- **[W-3]** Edge case not addressed: `reconciler_passes` is pruned on `started_at` while a reconciler pass is presumably running — the current pass's own row is newer than the cutoff so is safe, but the brief should state that the in-flight pass row is never eligible (it is, by cutoff, but a one-line note would prevent a future "prune everything" bug).
- **[W-4]** The plan-test criterion requires "no `SCAN <table>`" for all eight statements, but the fleet-health three are asserted (NV-1) to use an *autoindex* plan while the others use explicit indexes; if NV-1 resolves against the brief (WITHOUT ROWID tables), the shared `RetentionTable` shape and its plan-test arms need rework — resolve NV-1 before implementation to avoid mid-PR redesign.

### NIT findings
- **[NIT-1]** Scope item 2 says "the other seven already have an index" — with eight tables listed and `host_samples` the eighth, the count is consistent, but a reader must do the arithmetic; naming the seven (or saying "the other tables except `host_samples`") would be clearer.
- **[NIT-2]** "about lines 390-395" and "around line 3330–3360" are approximate; fine for a brief, but the handoff should pin exact locations.

### Verdict
PASS — no BLOCK findings; implementation may proceed, with NV-1 (rowid existence on the fleet-health tables) resolved first as it shapes the shared batched-statement design.