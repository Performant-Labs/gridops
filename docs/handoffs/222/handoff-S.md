# Handoff-S: Phase 8 - Lock storm: DELETE FROM host_samples scans 628k rows inside the write lock (#222)

**Date:** 2026-11-22
**Branch:** claude/fix-222-retention-batches (head f74df68; origin/master 36dbc1a is an ancestor, re-checked after `git fetch`)
**Issue:** #222
**Brief reviewed:** docs/handoffs/222-brief.md (amended in 20df2d2)
**Handoff-T reviewed:** docs/handoffs/222/handoff-T-green.md (and handoff-T-red.md)
**Handoff-A reviewed:** docs/handoffs/222/handoff-A.md (plan review, PASS) and handoff-A-dup.md (anti-duplication gate, PASS)
**Handoff-F reviewed:** docs/handoffs/222/handoff-F.md
**Operator-facing report:** N/A (no UI, no rendered output; a non-visual audit)

## A precondition
Confirmed. A returned **PASS** at Phase 3 (0 block, 4 warn; the three earlier BLOCK findings are resolved) and the anti-duplication gate returned **PASS** (0 block, one info pair and one out-of-scope warn).

## T precondition
Confirmed. T-green reports **zero blocking issues**. I re-ran the evidence myself (below) rather than relying on T's report.

## Visual-diff-tool and browser-tool preconditions
N/A. The brief states "No UI change, no API change, no new setting." No visual surface exists, so no screenshots, pixel diffs or HTML report are owed.

## Preview / spec sanity check
No defect in the spec that would justify `ADVISORY-HOLD`. One inaccuracy in the brief's prose, already noted by A and harmless: "the other seven already have an index on their cutoff column (0034 and 0001)". `reconciler_passes_started_idx` is from 0036 and the three fleet-health indexes are primary-key autoindexes from 0050. The tests pin the real index names, so nothing depends on the prose. F's implementation matches the brief, not the prose error.

## Independent verification (run by S on head f74df68)
Environment: `CARGO_BUILD_JOBS=2 RUST_TEST_THREADS=2`, `CARGO_TARGET_DIR` unset (shared target from the parent `.cargo/config.toml`), source files touched before each build.

| Check | Result |
|---|---|
| `cargo fmt --all -- --check` | clean |
| `cargo test -p gridops-core --test retention --test fleet_health_schema` | `retention` 7 passed, `fleet_health_schema` 5 passed |
| `cargo test -p gridops-reconciler --bin gridops-reconciler -- snapshot_retention_tests fleet_health_wiring_tests` | 8 passed, 0 failed (12.7 s) |
| `cargo +1.98.1 clippy -p gridops-core -p gridops-reconciler --all-targets --locked -- -D warnings -A clippy::unused_async_trait_impl` | Finished, no warnings |
| **Mutation M1** (brief-required): move `migrations/0051_host_samples_received_idx.sql` away | `every_retention_batch_finds_its_rows_through_its_cutoff_index` **FAILED**: `the host_samples batch must search it through host_samples_received_idx: ["SEARCH host_samples USING INTEGER PRIMARY KEY (rowid=?)", "LIST SUBQUERY 1", "SCAN host_samples USING COVERING INDEX host_samples_host_received_idx", "USE TEMP B-TREE FOR ORDER BY"]`. The other six core tests pass. |
| **Mutation M5** (brief-required): `prune_retained_table(app, table, cutoff).await;` replaced by `retention::prune(...).await?;` | `a_failing_table_prune_does_not_stop_the_other_tables_or_the_pass` **FAILED**: `with webhook_deliveries refusing deletes: cleanup returned Err(... (code: 1811) webhook_deliveries deletes refused by the test`, with every other table's row left behind. |

Both mutations were applied alone and reverted. `git status --short` was empty afterwards, and `main.rs` was restored byte for byte from a scratchpad copy. The brief's two required mutation checks are therefore reproduced by S, not only reported by F and T.

## Spec compliance
| Brief requirement | Compliant | Evidence |
|---|---|---|
| **Scope 1.** Migration `0051_host_samples_received_idx.sql`, header comment in the style of 0034/0049, exactly `CREATE INDEX host_samples_received_idx ON host_samples(received_at);`, no `IF NOT EXISTS`; numbered after master's 0050 | YES | File read: 5-line header naming #222 and the failure mode, then the one statement. `migrations/` ends 0050, 0051. M1 proves the plan test depends on it. |
| **Scope 2.** All eight retention deletes batched: `webhook_deliveries`, `audit_events`, `capacity_samples`, `host_samples`, `reconciler_passes`, plus `fleet_health_hourly`, `fleet_host_hourly`, `fleet_health_snapshots` | YES | `RetentionTable::ALL` is `[Self; 8]`. `cleanup_retention` loops over it. `grep "DELETE FROM" main.rs` finds no retention delete left in `cleanup_retention`; #221's three unbatched `.await?` deletes are gone. |
| Fleet-health variants: own `const`, `as_str` arm, plan-test arm; one cutoff arm on `now - FLEET_HEALTH_RETENTION_DAYS * 86_400_000`; the constant stays in the reconciler | YES | `FleetHealthHourly`, `FleetHostHourly`, `FleetHealthSnapshots`. The cutoff is computed beside the others and the single match arm uses it. The constant is unmoved. |
| Reuse `snapshot_retention` shape and constants: 500-row batches, 25 ms sleep, cap 40, one autocommit statement per batch, no transaction across batches | YES | `retention.rs`: `BATCH_SIZE 500`, `MAX_BATCHES 40`, `BATCH_PAUSE 25 ms`; `delete_in_batches` runs `.execute(pool)` per batch with the pause from the second batch on. `snapshot_retention` re-exports the constants. |
| Each batch `DELETE FROM <t> WHERE rowid IN (SELECT rowid FROM <t> WHERE <col> < ? ORDER BY <col> LIMIT ?)`, keeping `ORDER BY <col>` | YES | Eight `const` strings in that shape. M-fleet (T) and M2/M2b (T-red) show `ORDER BY rowid` goes red. |
| Helper in gridops-core, table and column from a closed `const` set, no SQL from runtime input | YES | New `gridops_core::retention` module. `prune_batch` returns `&'static str` chosen by a closed enum, and `delete_in_batches` is `pub(crate)` and takes `&'static str`. Values are bound. |
| Share code with `prune_inventory_snapshots` only if clean; otherwise leave it | YES (shared) | The loop moved to `delete_in_batches`; `prune_inventory_snapshots` keeps its own `PRUNE_BATCH` (different WHERE) and delegates the loop. Signature and tests unchanged. A's plan review and the anti-duplication gate both approved this. |
| `enforce_log_storage_budget` untouched | YES | No diff there. |
| **Scope 3.** A failed prune of one table logs `warn` naming the table and does not stop the other tables or the pass | YES | `prune_retained_table`: `Err` -> `tracing::warn!(table = table.as_str(), ...)`, returns `()`; `cleanup_retention` still returns `Ok`. M5 proves the test catches a restored `?`. |
| **Scope 4.** `info` per table per pass when the count is not zero | YES (by review) | `Ok(0) => {}`, `Ok(deleted) => tracing::info!(table = table.as_str(), deleted, "pruned old rows")`. Matches the snapshot prune. The brief names no test for logging, and T and F declared it a review item. |
| **Tests.** Plan test for each of eight statements: inner select uses the cutoff-column index, no `SCAN`, fails when 0051 is removed | YES | `every_retention_batch_finds_its_rows_through_its_cutoff_index`; M1 reproduced. |
| Fold #221's three hand-written delete plan rows from `fleet_health_schema.rs` into the `ALL` plan test, keep the file's other rows | YES | Diff shows exactly those three rows and their comment removed. The other two rows stay. Test renamed `window_reads_go_through_an_index` and module doc updated (A warn 2). |
| Batching: at most `BATCH_SIZE * MAX_BATCHES` per pass, newer rows never deleted, early stop on a short batch, pause between batches (`a_pass_pauses_between_its_batches` pattern) | YES | Core: `a_tables_prune_takes_only_its_rows_older_than_the_cutoff_oldest_first` (per table, strict cutoff, cutoff row kept), the short-batch stop test, `a_prune_pauses_between_its_batches`. Reconciler: `a_pass_deletes_at_most_batch_size_times_max_batches_rows_of_each_table` (20,001 old rows in each of eight tables, exactly the youngest survives). |
| Error isolation: when one table's prune fails, the others are still pruned | YES | `a_failing_table_prune_does_not_stop_the_other_tables_or_the_pass`: each of the eight refuses its deletes through a test-only trigger in turn. `cleanup_retention` must return `Ok`, every other table is pruned, and the snapshot prune still runs. |
| Mutation checks shown in the handoff (plan test red when 0051 removed; isolation test red when a `?` is restored) | YES | In handoff-F, and reproduced by S above. |
| **Constraints.** `CARGO_BUILD_JOBS=2`, `RUST_TEST_THREADS=2`, shared target, `CARGO_TARGET_DIR` never set; no `VACUUM`; PR against Performant-Labs/gridops | YES | Environment as above. A grep finds no `VACUUM` in the diff. The PR step belongs to O. |

## Quality audit
| Area | Result | Notes |
|------|--------|-------|
| API consistency | N/A | No endpoint, schema or OpenAPI change. `snapshot_retention` re-exports keep the three constants' paths and values. |
| Error handling | PASS | `warn` with the table name on `Err`; the pass continues. The guarantee is exactly "one table's prune failure does not stop the other tables' prunes, the snapshot prune or the pass" (see Scope check on what is not covered). |
| UI/UX match to spec | N/A | No UI. |
| Accessibility | N/A | No UI. |
| Architecture gate | PASS | A: PASS at Phase 3; anti-duplication: PASS. No conflict between this audit and either. `crate::retention` imports nothing from `fleet_health`; the dependency direction is unchanged. |
| Code organization | PASS | One small module, one enum, one shared loop. No dead code, commented-out code or TODO/FIXME in the diff. Exhaustive `match`es (cutoff arm, test expectations, test table builder) make a ninth table a compile error until it has a cutoff, a plan expectation and test rows. |
| Security | PASS | Statements are `const &'static str` chosen by a closed enum; every value is bound; `u64::try_from(batch_size)` stops a negative `LIMIT` (SQLite treats it as no limit). No secrets, input or auth surface touched. |
| Performance | PASS | The incident's cost, a 628k-row scan inside the write lock, is gone: with 0051 the inner select is a `SEARCH ... USING COVERING INDEX` for all eight tables (plan test). Each batch is 500 rows, with a 25 ms pause, at most 40 per table per pass. The one-off index build is seconds on 628k narrow rows, run at migration time before the services start (`db.rs`, `compose.yaml` `depends_on`), as the brief accepted. |
| Visual regression | N/A | No UI. |
| Naming consistency | PASS | `RetentionTable` variants, `as_str` values and index names match the DDL. The stale name `window_reads_and_retention_deletes_go_through_an_index` and its module-doc clause are corrected. |
| Test quality (`testing/test-quality.md` section 7) | PASS | See below. |

### Test-quality audit (rubric section 7)
Per test, in the diff:
- `every_retention_batch_finds_its_rows_through_its_cutoff_index` names one behavior (each batch reads through its cutoff index), fails for the right reason (M1: the exact `SCAN host_samples` plan), and is the cheapest tier that can see a query plan. It replaces three hand-written plan rows instead of duplicating them. **Pass.**
- `a_tables_prune_takes_only_its_rows_older_than_the_cutoff_oldest_first` is one behavior per table (strict cutoff, oldest first, bounded, own table only). It is per table because each table has its own `const`; the "other tables untouched" cross-check is the "own table only" claim, so it isn't a redundant re-proof. **Pass.**
- `a_prune_pauses_between_its_batches`, the short-batch stop test, `a_prune_with_a_batch_size_below_one_deletes_nothing`: each guards one branch of the shared loop on one table, as the loop is shared. Lower-bound-only timing, so deterministic. **Pass.**
- `a_pass_deletes_at_most_batch_size_times_max_batches_rows_of_each_table`: guards the incident's invariant through `cleanup_retention`, and catches an unbounded delete left next to a correct one (T's M3 shows the plan test passes in that case). About 10 s. **Pass.**
- `a_failing_table_prune_does_not_stop_the_other_tables_or_the_pass`: one behavior, eight cases from one exhaustive list. M5 turns it red. **Pass.**
- `each_retained_table_keeps_its_own_window`: a guard of existing behavior (cutoffs must not move); the fleet-health window has a single home in `retention_keeps_90_days_of_fleet_health_rows`, as A's warn 1 asked, so the new tables are not tested twice. **Pass.**

Per suite: proportionate to an eight-table fix. The count is driven by "one `const` per table" and "one branch per loop behavior", not by files touched. T found no redundant test, and I found none that fails alone for the wrong reason. No assertion-free, tautological, mock-shaped or snapshot-everything test. No "delete or merge" finding.

Advisory only (not a finding): `every_prune_is_bounded_to_forty_batches_of_five_hundred_with_a_pause_between` asserts the decided constants (500, 40, at least 25 ms) and that `snapshot_retention` re-exports them. It's a change-detector on numbers rather than a behavior. T's rationale holds: the pause and cap tests read those constants, so without the pin `BATCH_PAUSE = 0` or a huge `MAX_BATCHES` would pass them trivially. The brief decided the numbers explicitly, and the test is cheap (no database). Keep it; a future retune should expect to edit it.

## Scope check
F delivered the brief's scope exactly: migration 0051, the closed `RetentionTable` set and shared loop in `gridops_core::retention`, `prune_inventory_snapshots` delegating its loop, the reconciler's `RetentionTable::ALL` loop and `prune_retained_table`, and #221's three unbatched deletes removed. No over-delivery: no UI, API or setting change, no `VACUUM`, no new dependency, and no change to `enforce_log_storage_budget`. No under-delivery against the amended brief's eight-table scope.

**What the change does not cover (stated so the PR does not over-claim, per A finding 4):** the guarantee is that one table's prune failure does not stop the other tables' prunes, the snapshot prune or the pass. These remain as they were, by the brief's scope:
- the `log_streams` expiry delete, the `UPDATE log_streams` and `enforce_log_storage_budget` ahead of the prune loop, which still end in `?` and skip every prune if they fail;
- `reconcile()`'s `DELETE FROM fleet_agent_logs WHERE received_at < ?` (`main.rs:739`), still one unbounded `.await?`, and the `oauth_states`, `github_app_manifest_states`, `ai_oauth_attempts` and `sessions` expiry deletes after it (`main.rs:743-755`).

## Verdict

PASS — all acceptance criteria met, spec-compliant, quality acceptable. Ready for O to commit.

## Advisory notes
None of these blocks the PR; all are for O.
1. **Open the `fleet_agent_logs` follow-up issue before the PR is opened, and link it from the PR body** (A finding 4, anti-duplication finding 3). `gh issue list --state all --search fleet_agent_logs` still returns nothing as of this audit. In the new design the fix is one more `RetentionTable` variant, but its only index leads with `host_id` (0034), so it needs its own `received_at` index too. The same issue can note the four small expiry deletes at `main.rs:743-755`, and the `?` on the log-stream steps ahead of the prune loop.
2. **The PR body should claim only** "a failed prune of one table does not stop the other tables' prunes or the pass", not "nothing stops the prunes."
3. **CHANGELOG (optional, docs-only).** The brief doesn't scope one, and `docs/releasing.md` says the changelog is written in the version-bump PR, so its absence isn't a defect. Note that #152's brief did scope a CHANGELOG entry, and #221 added an Unreleased "Upgrading" note for migration 0050. If O wants parity, an Unreleased "Upgrading" line is worth adding: migration 0051 builds an index on `host_samples` at start-up (seconds on Mars's 628k rows; the brief's own estimate comes from 0049's 6.6 s on 207k rows). Add a "Fixed" line too: retention deletes now run in batches and one table's failure no longer stops the rest. This can go in during the PR step without another F, T and A cycle.
4. **Deploy expectation.** After deploy, the first retention passes drain any backlog at no more than 20,000 rows per table per pass (160,000 across the eight tables), with a 25 ms pause between batches, so a large backlog empties over several passes instead of one long lock. This is the intended behavior.
