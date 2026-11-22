# Handoff-T-green: Phase 7 - Prune fleet_inventory_snapshots (#152)

**Date:** 2026-11-22
**Branch:** claude/fix-152-prune-snapshots (HEAD 5b32f1e, F's implementation)
**Issue:** #152
**Handoff-F reviewed:** docs/handoffs/152/handoff-F.md
**Handoff-T-red:** docs/handoffs/152/handoff-T-red.md

## GREEN confirmation
Ran with `CARGO_BUILD_JOBS=2 RUST_TEST_THREADS=2`, after `find crates -name '*.rs' -exec touch {} +` (shared-target hazard).
- `cargo test -p gridops-core -p gridops-reconciler`: exit 0. core lib 315 passed; every core integration binary ok (fleet_schema 17, fleet_contracts 4, fleet_inventory 2, retention 1); reconciler 127 passed.
- `cargo test -p gridops-api --no-fail-fast`: lib 130 passed, control_plane_contracts 39 passed, fleet_http 150 passed / 1 failed. The one failure is `release_preflight::the_release_preflight_fails_early_without_a_macos_arm64_machine`: the test's "Linux machine" reports this Mac, so preflight prints `PASS macos-machine`. The test and its fixture are not in the diff, F showed it fails with F's changes stashed, and it is environment-dependent (macOS host). It is unrelated to #152 and should pass on Linux CI. Not blocking.
- All authored tests pass: core `snapshot_retention_tests` (3), `query_plan_tests::a_snapshot_prune_batch_reads_only_rows_older_than_the_cutoff`, reconciler `snapshot_retention_tests` (2), API `snapshot_retention_setting::snapshot_retention_days_are_a_setting_left_alone_when_a_save_omits_them`. The `macos_vm_slots` precedent tests pass.
- Spot-check that the tests pin behavior: temporarily replaced `AND s.ended_at IS NULL` with `AND 1=1` in F's `PRUNE_BATCH`; `the_prune_keeps_each_open_sessions_current_snapshot_and_everything_recent` FAILED. Reverted (git status clean). T-red's mutations M1-M12 (ended_at, session_id, ORDER BY rowid, `<=`, ignore max_batches, no clamp, default 30, `?` propagation, naive DELETE, no range check, omitted resets, no audit) cover the rest against equivalent code.

## Tier 1 results
- `cargo fmt --all -- --check`: exit 0. PASS.
- `cargo +1.98.1 clippy -p gridops-core -p gridops-api -p gridops-reconciler --all-targets --locked -- -D warnings -A clippy::unused_async_trait_impl`: finished, no warnings. PASS.
- Build/compile of all three crates and the API binary (fleet_http launches it): PASS. API smoke via fleet_http (GET/PUT /api/v1/settings): PASS.
- Existing tests that delete snapshot rows (`invariants_tests`, 17 `fleet_schema` upgrade tests, which run migration 0049): PASS.

## Tier 2 results
- Coverage per criterion: PASS (see below).
- Test quality: each test names a behavior and sits at the cheapest sufficient tier (SQL rule and plan: core integration/unit against real SQLite; wiring: reconciler through `cleanup_retention`; contract: HTTP). No duplicates; the const-pin test is a unit-level pin of the lock-safety numbers. Proportionate. PASS.
- Lock-safety review (no deterministic test, as stated in T-red): `prune_inventory_snapshots` runs one `.execute(pool)` per batch, no `begin()`; `BATCH_PAUSE` slept only between batches; `batch_size < 1` returns `Ok(0)` (negative LIMIT guard); loop stops on `rows < batch_size` or `max_batches`. PASS.
- Type safety: no `any`-style casts; `i64`/`u64` conversion via `try_from`. PASS.
- Error handling: prune error logged at `warn` and swallowed (guard test); out-of-range API values 400 (0, -1, 3651; 1 and 3650 accepted). PASS.
- Data integrity: the rule keeps every open session's current snapshot; resend path cannot reach an ended session (`load_current_session` requires `ended_at IS NULL`, inventory.rs:404-411, in evidence.md). Reader list re-verified against code; no non-current reader. PASS.
- API contract: optional field, omitted keeps the stored value, GET default 7, audit JSON carries it; no new route. PASS.
- Migration safety: 0049 is exactly `CREATE INDEX fleet_inventory_snapshots_received_idx ON fleet_inventory_snapshots(received_at);` with a 0034-style header; additive, reversible by `DROP INDEX`, no data change. PASS.
- Security: no new endpoint; the existing settings auth path is unchanged. PASS.
- Docs/CHANGELOG: `docs/fleet-api.md` section and two Upgrading plus one Changed CHANGELOG entries present (`fleet_contracts`, which parses fleet-api.md, passes). PASS.
- Evidence appendix: checked `docs/handoffs/152/evidence.md` entries against source (setting_i64, load_current_session, db.rs:35 busy_timeout, readers in mod.rs/queries.rs); verbatim and accurate. Nothing to append. PASS.
- Playwright: not used by this project change (no UI). N/A.

## Acceptance criteria status
1. Retention rule (old non-current and old ended-current go; open-session current and recent stay): PASS, `the_prune_keeps_each_open_sessions_current_snapshot_and_everything_recent`.
2. Batch limit (batch_size * max_batches per call, oldest first, max_batches=1 deletes at most batch_size): PASS, `one_call_deletes_at_most_batch_size_times_max_batches_oldest_first`; the pause between batches: `a_pass_pauses_between_its_batches`; the `batch_size < 1` guard: `a_batch_size_below_one_deletes_nothing` (both added in the test-only rework below).
3. Settings API (store, omit keeps, 1..=3650): PASS, `snapshot_retention_days_are_a_setting_left_alone_when_a_save_omits_them`.
4. Reconciler default 7 and clamp, prune error does not fail cleanup: PASS, reconciler `snapshot_retention_tests` (2).
5. Plan test (SEARCH snap via received_idx, SEARCH s via index, no SCAN): PASS, `a_snapshot_prune_batch_reads_only_rows_older_than_the_cutoff`.
6. Existing snapshot-deleting tests: PASS.
7. Timing estimate in F's handoff (median 2.3 ms, max 14 ms per batch; steady 1.7 ms; index build 0.10 s on seed, about 1-3 s on prod): present. Not re-measured by T; the plan is pinned by test 5 on the app's bundled SQLite.

## Blocking issues
None.

## Advisory notes
- No test of the `info` row-count log or of lock release under a concurrent writer (timing-dependent; covered by review and F's timings).
- U: N/A (no UI surface). S: Tier 3 per brief; PR body must say the 0049 index builds once under the write lock (about 1-3 s), the file does not shrink, and VACUUM only while stopped.

## Test-only rework (S REWORK findings 1 and 2, 2026-11-22)
Both S findings were test-quality gaps, so T fixed them. `snapshot_retention.rs` is byte-identical: `git diff` on it is empty.
- **Finding 1:** deleted the constant-pinning `a_pass_is_bounded_to_forty_batches_of_five_hundred_with_a_pause_between` and replaced it with `a_pass_pauses_between_its_batches`. The new test seeds 7 eligible rows and calls with `batch_size` 2 and `max_batches` 3, so three batches are full and the call takes two pauses. It asserts 6 rows deleted and a wall-clock time of at least `2 * BATCH_PAUSE`. It asserts no upper bound, because `tokio::time::sleep` never returns early. It keeps the `BATCH_PAUSE >= 25 ms` check. The paused tokio clock was not used: auto-advance would also fire the sqlx pool's timers. The test does not assert that no pause follows the last batch, because that needs an upper bound on wall time, which can flake.
- **Finding 2:** added `a_batch_size_below_one_deletes_nothing`. With 3 eligible rows, `batch_size` 0 and then -1 (`max_batches = MAX_BATCHES`) each return 0 and leave every row in place.
- **Mutants**, each applied, run and reverted:
  - P (the `tokio::time::sleep(BATCH_PAUSE)` line removed): `a_pass_pauses_between_its_batches` FAILS (`1.144792ms < 2 * 25ms`) and the others pass. Killed.
  - G (the guard replaced by `let full = batch_size as u64;`): `a_batch_size_below_one_deletes_nothing` FAILS at `batch_size -1` (left 3, right 0: the unbounded delete). Killed.
- **Re-run on the real code:** `cargo test -p gridops-core --lib snapshot_retention` gives 4 passed. `cargo fmt -p gridops-core --check` is clean. `cargo +1.98.1 clippy -p gridops-core --all-targets --locked -- -D warnings -A clippy::unused_async_trait_impl` is clean.
- **Shared target-dir note:** the first run reported a stale "Fresh" binary with 0 matching tests, because another checkout had built into the shared target dir. Touching `src/lib.rs` before each run forced a real rebuild, and every result above comes from a rebuilt binary. Check the test count, not just the exit code.
