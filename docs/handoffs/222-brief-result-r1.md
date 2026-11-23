## Brief Review (Round 1)

### BLOCK findings
None.

### Needs-verification findings
- **[NV-1]** The 40-batch cap (40 × 500 = 20,000 rows per pass per table) keeps up with deletion backlog — the brief does not state the retention pass frequency or the per-day row accrual for any of the five tables. If the pass runs less often than backlog/20,000, retention never catches up and the table grows unboundedly. Evidence that would settle it: pass schedule (e.g., a reconciler interval in `crates/gridops-reconciler/src/main.rs`) plus per-table row accrual rates from prod metrics.
- **[NV-2]** "`ORDER BY <col>` avoids the planner falling back to a full scan (vs `ORDER BY rowid`)" — this is a claim about SQLite planner behavior. Evidence: the EXPLAIN QUERY PLAN output cited in #152 / PR #189, or a SQLite documentation/CHANGES excerpt on ORDER BY satisfaction by index.
- **[NV-3]** "Migrations run before the services start" — a deploy-ordering claim; nothing in the brief shows the deploy pipeline. Evidence: the migration runner's startup sequence (file:line in the service entrypoint or deploy config).
- **[NV-4]** The other four tables already have an index leading with their cutoff column (`webhook_deliveries.received_at`, `audit_events.created_at`, `capacity_samples.recorded_at`, `reconciler_passes.started_at`) — the brief attributes this to migrations 0034 and 0001 but shows no schema excerpt. If any is wrong, the plan test will fail late. Evidence: `EXPLAIN QUERY PLAN` output or the index DDL in those migration files.
- **[NV-5]** `snapshot_retention`'s stated shape (500 / 25 ms / 40 batches / autocommit per batch / no spanning transaction) matches `crates/gridops-core/src/fleet/registry/snapshot_retention.rs` — the constants and transaction behavior are asserted, not shown. Evidence: that file's batch loop and constant definitions.

### WARN findings
- **[W-1]** The brief is silent on what happens when the 40-batch cap is hit mid-backlog: there is no log line, metric, or acceptance criterion covering "cap reached, rows remain." If the first post-gap pass deletes 20,000 of 400,000 stale rows, nothing in scope requires that to be observable. Recommend adding: log at `warn`/`info` when the cap is reached with rows remaining.
- **[W-2]** Error-isolation acceptance criterion ("the table was dropped in the test database") is fine, but dropping a table also breaks the plan tests' shared database if they share state. Recommend specifying each test builds its own migrated database, or the isolation test uses its own.
- **[W-3]** `CREATE INDEX` with no `IF NOT EXISTS` will hard-fail if 0050 was ever partially applied without its bookkeeping row. If the migration runner is not transactional per-file, this blocks startup. Either state that the runner records migrations transactionally (evidence would settle NV-3's vicinity) or allow the guard.

### NIT findings
- **[NIT-1]** The plan-test assertion "with no `SCAN <table>`" should also pin the index *name* (`host_samples_received_idx`) for the host_samples case, otherwise a different index satisfying the plan (none exists today, but the test is the regression guard) could pass vacuously.
- **[NIT-2]** "the architecture reviewer decides where" leaves a placement decision open at implementation time; fine, but the handoff should record the choice so the plan tests' module path is predictable.

### Verdict
PASS — no BLOCK findings; implementation may proceed.