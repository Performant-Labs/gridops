## Implementation Review (Round 1)

### BLOCK findings

**[B-1] docs/handoffs/222 (Feature Handoff, "Tier 1 self-check") — missing mutation check the brief requires — the brief's Tests section says "Show a mutation check for the plan test and the error-isolation test in the handoff," but the handoff's mutation-check entry only says "T's handoff-T-red already ran M1-M11 against an equivalent throwaway, and running them again is T's Phase 6." No mutation check for `every_retention_batch_finds_its_rows_through_its_cutoff_index` or `a_failing_table_prune_does_not_stop_the_other_tables_or_the_pass` is shown, and the handoff's "Deviations from spec: None" contradicts the brief's explicit deliverable — this blocks because it is an unmet acceptance criterion demonstrable from the prompt itself — remediation: run and record the two mutation checks (e.g. remove migration 0050 for the plan test; remove the `Err` arm / re-add `?` for the error-isolation test) and their red results in the handoff.

### Needs-verification findings

**[NV-1] crates/gridops-core/tests/retention.rs — the plan test asserts the `EXPLAIN QUERY PLAN` detail string ends with exactly `INDEX {index} ({column}<?)` (e.g. `INDEX host_samples_received_idx (received_at<?)`) — settle with the actual SQLite EQP output on the project's SQLite version (e.g. run `EXPLAIN QUERY PLAN` on a migrated database and paste the `detail` strings); the handoff reports the tests green, but the exact detail wording is a runtime claim this prompt cannot check.

**[NV-2] crates/gridops-core/tests/retention.rs (Expected inserts) — the test inserts assume the exact column sets/NOT-NULL constraints of `host_samples` (`host_id,host_epoch,observed_at,received_at,coverage`), `fleet_hosts`, `runner_pools`, and `installations` are as written — settle with the `CREATE TABLE` statements for those tables in migrations (e.g. `migrations/0027_fleet_queries.sql`, `0001_initial.sql`); a NOT NULL column omitted from the insert would make every test in the file fail rather than pass.

**[NV-3] crates/gridops-reconciler/src/snapshot_retention_tests.rs `setting()` — the comment claims storing `value.to_string()` matches "how the settings API does (a JSON number)" — settle with the settings write path in gridops-api (the `settings.value` column type and serialization); a mismatch would mean the test's stored setting is not what production writes, weakening `each_retained_table_keeps_its_own_window`'s settings half.

**[NV-4] crates/gridops-reconciler/src/main.rs:700 / 643 — the claim that these are the only `cleanup_retention(app).await?` call sites — settle with a repository-wide search for `cleanup_retention(`; the evidence appendix cites both, but excerpts are bounded.

### WARN findings

**[W-1] migrations/0050_host_samples_received_idx.sql — migration-number collision with open PR #221's `0050_fleet_health.sql` is acknowledged but deferred to "land time"; PR #221's three new `cleanup_retention` deletes also must become `RetentionTable` variants after this lands — recommendation: coordinate the renumbering and the enum extension in this PR's description as a landing precondition, since whichever merges second silently breaks the other's migration ordering.

**[W-2] crates/gridops-core/src/retention.rs `prune_batch()` — the five `const` SQL strings embed newlines/indentation inside the string literals; harmless for SQLite but makes the strings differ from the single-line form a reviewer might grep for — recommendation: none required, but note the strings are multi-line when writing the mutation checks.

### NIT findings

**[NIT-1] crates/gridops-core/tests/retention.rs `a_prune_stops_at_the_first_short_batch` — asserts `db.remaining(...) == [0_i64; 0]`; `[].to_vec()`-style comparison or `is_empty()` would read clearer, and the `u32::MAX` max_batches relies on the short-batch stop rather than the timeout to terminate.

**[NIT-2] crates/gridops-reconciler/src/main.rs `prune_retained_table` — `Ok(0) => {}` silently skips logging; intentional per brief (log only non-zero), but a comment saying so would prevent a future "fix."

### Verdict

BLOCK — 1 blocking finding; must resolve before testing starts.