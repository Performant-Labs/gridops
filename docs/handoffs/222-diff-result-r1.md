## Implementation Review (Round 1)

### BLOCK findings
None.

### Needs-verification findings
**[NV-1]** crates/gridops-reconciler/src/snapshot_retention_tests.rs (MACHINES / `capacity_parents`) — The claim that `host_samples` rows with `host_id = 'snapshot-retention-host'` insert successfully in the reconciler test harness: `capacity_parents` (shown in the diff) inserts only `installations` and `runner_pools`, no `fleet_hosts` parent, unlike the core-side `Db::new` which does insert a `fleet_hosts` row. If `host_samples.host_id` has an enforced FK, these inserts fail; if the pre-existing `Harness::new` (not shown) already creates a matching host, or FKs are off, it's fine — the green test runs suggest it works, but the harness source isn't in this prompt. Settled by: the `Harness::new` body in `snapshot_retention_tests.rs` and the `host_samples` table definition in `migrations/` (FK clause), plus the PRAGMA foreign_keys setting in `db.rs`.

**[NV-2]** migrations/0051_host_samples_received_idx.sql — The claim (handoff, M1 mutation) that the plan test fails when 0051 is removed is based on a run I cannot reproduce here; also the claim that the SQLite planner produces `SEARCH <t> USING … INDEX <idx> (<col><?)` for all eight statements (including the three autoindex cases) is a runtime claim shown only as reported output. Settled by: re-running `-p gridops-core --test retention` with 0051 removed, and `EXPLAIN QUERY PLAN` output for all eight `prune_batch` statements on a migrated database.

**[NV-3]** crates/gridops-core/src/retention.rs — The claim that each batch statement is a single autocommit statement (no implicit transaction spanning batches under the sqlx SQLite pool) is a runtime-library behavior claim not evidenced in this prompt. Settled by: sqlx SQLite executor source / documentation showing `sqlx::query(...).execute(pool)` on a pool without an explicit transaction runs in autocommit mode.

### WARN findings
**[W-1]** crates/gridops-reconciler/src/main.rs:739 — `DELETE FROM fleet_agent_logs WHERE received_at < ?` remains one unbounded `?`-propagated delete, and its only index leads with `host_id` (0034:7), so on a large `fleet_agent_logs` it can reproduce this exact incident. The handoff declares it out of scope with a follow-up issue, and the brief scopes `cleanup_retention` only — but the same class of defect is being left in the same pass. Recommendation: confirm the follow-up issue exists before merge and covers the `oauth_states` / `github_app_manifest_states` deletes at :743–749 too, which are also unbounded.

**[W-2]** crates/gridops-core/tests/retention.rs (`a_prune_stops_at_the_first_short_batch`) — the test's rationale comment says a non-stopping call "would sleep forever", and it relies on `tokio::time::timeout(10s)` rather than `#[tokio::test(start_paused = true)]`; with `u32::MAX` batches a genuine regression makes the test take up to 10 s (and with real sleeps, far longer than the timeout budget only because the timeout fires first — fine, but flaky-slow under `RUST_TEST_THREADS=2` load on a busy CI host). Recommendation: use paused time or a smaller `max_batches` sentinel.

### NIT findings
**[NIT-1]** crates/gridops-core/tests/retention.rs (`a_tables_prune_takes_only_its_rows_older_than_the_cutoff_oldest_first`) — the nested per-`other`-table assertion loop re-checks already-verified invariants O(n²) times; the `pruned`/`untouched` cross-check adds little beyond the per-table assertions.

**[NIT-2]** migrations/0051_host_samples_received_idx.sql — brief says "header comment in the style of 0034 and 0049"; 0034's header is 4 lines ending with the failure mode, this is 5 lines — substantively equivalent, no action needed.

### Verdict
PASS — no BLOCK findings; testing may proceed.