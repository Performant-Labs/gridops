# Handoff-T-red: Phase 4 - Prune fleet_inventory_snapshots: it grows without bound (#152)

**Date:** 2026-11-22
**Branch:** claude/fix-152-prune-snapshots (base dbc9e80; only test files are staged)
**Brief / wireframe reviewed:** docs/handoffs/152-brief.md (amended), docs/handoffs/152/handoff-A.md (re-review, PASS). Wireframe: N/A (no UI).

## A precondition
Confirmed: A returned **PASS** on the plan (Phase 3 re-review, dbc9e80), with 0 blocks and 1 warn.
- **Warn 1 is applied in test 5.** The plan test matches the sessions step by its alias: some step starts `SEARCH s USING ` and contains `INDEX`.
- It names no sessions index. The no-`SCAN` assertion is the real guard.
- The `snap` step is asserted exactly as the brief writes it.

## Contract the tests fix (F must provide these names)
**Core: a new `pub mod snapshot_retention` in `crates/gridops-core/src/fleet/registry/`, declared in `registry/mod.rs`.**
It must provide:
- `pub const PRUNE_BATCH: &str`: the one batch statement the function executes, with binds `(cutoff_millis, batch_size)` in that order, shaped as the brief gives it.
- `pub const BATCH_SIZE: i64` (500), `pub const MAX_BATCHES: u32` (40), and `pub const BATCH_PAUSE: std::time::Duration` (at least 25 ms).
- `pub async fn prune_inventory_snapshots(pool: &SqlitePool, cutoff_millis: i64, batch_size: i64, max_batches: u32) -> Result<u64, sqlx::Error>`. It returns the number of rows deleted.

**Migration `migrations/0049_fleet_inventory_snapshot_retention.sql`** creates `fleet_inventory_snapshots_received_idx`. The plan test checks for it by name.

**Reconciler and API:** no new names. The tests go through `cleanup_retention(&Reconciler)` and `PUT`/`GET /api/v1/settings`.

**The core lib test binary won't compile until the core items exist.** None of core's existing lib tests can run until then. A RED over new Rust items always works this way (#140, #146, #148, #179). The reconciler and API tests compile today and fail at runtime.

## Tests authored

### Core: `crates/gridops-core/src/fleet/registry/snapshot_retention_tests.rs` (new)
Included from `registry/tests.rs` with `#[path]`, as A suggested. It reuses the file-backed `Fixture`: hosts come from real enrollments, and sessions and snapshots are inserted through the real schema and triggers.

| Test | Pins | Tier |
|---|---|---|
| `the_prune_keeps_each_open_sessions_current_snapshot_and_everything_recent` | Brief test 1, the retention rule (details below). | Integration over the real migrated schema. The rule is SQL, so only SQLite shows it. |
| `one_call_deletes_at_most_batch_size_times_max_batches_oldest_first` | Brief test 2 (details below). | Same as above. |
| `a_pass_is_bounded_to_forty_batches_of_five_hundred_with_a_pause_between` | The brief's decided bounds: `BATCH_SIZE == 500`, `MAX_BATCHES == 40`, `BATCH_PAUSE >= 25 ms`. | Unit (const check). It's the cheapest pin on the lock-safety numbers. |

**`the_prune_keeps_each_open_sessions_current_snapshot_and_everything_recent`.** The seed has two hosts and four sessions: two open and two ended.
- **Host a has two sessions, both at current sequence 2.**
  - An old current row of the open session is kept.
  - An old current row of the ended session is deleted.
  - Only the session id tells these two rows apart, so the test catches an implementation that drops the `session_id` match or drops `ended_at IS NULL`.
- **Host b has:**
  - an old non-current row (deleted);
  - a row received exactly at the cutoff (kept, because the comparison is a strict `<`);
  - recent rows, current and non-current, of an open session and of an ended session (all kept).
- **Asserts:**
  - the exact set of surviving `(session, sequence)` rows;
  - the returned count, 4;
  - no session row is touched.

**`one_call_deletes_at_most_batch_size_times_max_batches_oldest_first`.** The seed is 15 eligible rows plus an open session's current row that is older than all of them. Insertion (rowid) order is the reverse of age. With `batch_size = 4`:
- `max_batches = 1` deletes exactly 4, the 4 oldest;
- `max_batches = 2` deletes exactly 8, which is `batch_size * max_batches`, and returns 8;
- the next call deletes the remaining 3 and returns 3;
- after that a call returns 0;
- the current row survives throughout.

### Core: `query_plan_tests.rs`, `a_snapshot_prune_batch_reads_only_rows_older_than_the_cutoff` (added)
This is brief test 5. `EXPLAIN QUERY PLAN PRUNE_BATCH` runs on the migrated schema and must:
- contain the step `SEARCH snap USING INDEX fleet_inventory_snapshots_received_idx (received_at<?)` exactly;
- contain a step starting `SEARCH s USING ` that contains `INDEX`;
- have no step starting `SCAN `.

It's a unit test against the real planner, the only place the plan is visible. It's in the same style as the #179 tests.

### Reconciler: `crates/gridops-reconciler/src/snapshot_retention_tests.rs` (new; `#[cfg(test)] mod` in `main.rs`)
The harness is a `Reconciler` over a fresh migrated database, built as in `gitlab_poll_tests`. It has one host, a credential, an open session at current sequence 100, and that session's current snapshot, received 400 days ago. The tests run through `cleanup_retention`.

| Test | Pins | Tier |
|---|---|---|
| `retention_keeps_the_configured_days_of_snapshots_seven_by_default` | Brief test 4 (stages below). Throughout, the 400-day-old current snapshot of the open session survives. That catches a naive `DELETE … WHERE received_at < ?` in the reconciler. | Integration through `cleanup_retention`. It's the cheapest tier that proves the reconciler reads the setting, clamps it and calls the prune. A unit test of a cutoff helper would not prove that `cleanup_retention` uses it. |
| `a_failing_snapshot_prune_does_not_stop_the_rest_of_the_cleanup` | Brief: "an error in the prune is logged at warn and does not fail the rest of `cleanup_retention`". A test-only `BEFORE DELETE` trigger makes every snapshot delete abort. `cleanup_retention` must still return `Ok`, and an old `reconciler_passes` row must still be deleted. **Guard: green before and after.** | Same. |

The stages of `retention_keeps_the_configured_days_of_snapshots_seven_by_default`:

| Setting | Expected |
|---|---|
| none | a row 7 days + 1 h old goes, one at 7 days − 1 h stays |
| 30 | 8 days old stays, 31 days old goes |
| 0 | clamped to 1: 1 day + 1 h old goes, 1 day − 1 h old stays (an unclamped 0 would take it) |
| 5000 | clamped to 3650: 3,651 days old goes, 3,649 days old stays |

### API: `crates/gridops-api/tests/fleet_http/snapshot_retention_setting.rs` (new; `#[path]` in `fleet_http.rs`, beside `macos_vm_slots`)

| Test | Pins | Tier |
|---|---|---|
| `snapshot_retention_days_are_a_setting_left_alone_when_a_save_omits_them` | Brief test 3 and the settings contract (sequence below). | Integration against the real API binary, like the existing settings tests. The contract is the HTTP shape. |

The test's sequence:
1. GET returns `inventorySnapshotRetentionDays: 7` by default.
2. PUT 30 answers 200. GET then returns 30, the `settings` row holds `"30"` (the JSON number `setting_i64` parses), and the latest `settings.updated` audit metadata carries 30.
3. A PUT without the field answers 200 and keeps 30.
4. PUTs of 0, -1 and 3651 answer 400 each and change nothing.
5. PUTs of 1 and 3650 are accepted.

### Not authored, by design
- **Brief test 6 (existing snapshot-deleting tests keep passing).** It needs no new test.
  - I ran the whole `gridops-core` suite with a throwaway correct implementation and 0049 in place: `invariants_tests` and all 17 `fleet_schema` upgrade tests pass.
  - I re-run it in Phase 6 against F's code.
- **Lock release between batches (autocommit per batch, sleep between batches).** I found no deterministic headless test for this. A concurrent-writer test would be timing-dependent.
  - A review of the code covers it in Phase 6: one `execute` per batch, no `begin()`, and `tokio::time::sleep(BATCH_PAUSE)`.
  - So do F's required timing measurements.
  - The const test pins the numbers.
- **The `info` log of rows deleted per pass.** I did not test it. It's observability, not behaviour, and the `warn` path is pinned by the guard's `Ok`.
- **Docs and CHANGELOG.** They are not testable behaviour. I check them in Phase 6.

## RED confirmation
I ran with `CARGO_BUILD_JOBS=2 RUST_TEST_THREADS=2` on dbc9e80 plus the test files only. `cargo fmt --all -- --check` is clean.

**1. Compile RED (the real tree). Only the missing contract module:**
```
cargo test -p gridops-core --lib --no-run
error[E0432]: unresolved import `crate::fleet::registry::snapshot_retention`
  --> crates/gridops-core/src/fleet/registry/query_plan_tests.rs:12:29
error[E0432]: unresolved import `crate::fleet::registry::snapshot_retention`
  --> crates/gridops-core/src/fleet/registry/snapshot_retention_tests.rs:12:29
```

**2. Runtime RED behind the compile RED.** I used a throwaway stub that reproduces today's behaviour:
- `PRUNE_BATCH` holds the brief's SQL verbatim;
- the function returns `Ok(0)` (nothing is pruned today);
- there is no 0049.

The stub was removed afterwards and never staged.
```
query_plan_tests::a_snapshot_prune_batch_reads_only_rows_older_than_the_cutoff ... FAILED
  the batch must search snapshots by received_at: ["SEARCH fleet_inventory_snapshots USING INTEGER PRIMARY KEY (rowid=?)", "LIST SUBQUERY 2", "SCAN snap", "CORRELATED SCALAR SUBQUERY 1", "SEARCH s USING INDEX sqlite_autoindex_fleet_observation_sessions_2 (host_id=? AND session_id=?)", "USE TEMP B-TREE FOR ORDER BY"]
snapshot_retention_tests::one_call_deletes_at_most_batch_size_times_max_batches_oldest_first ... FAILED
  left: 0   right: 4
snapshot_retention_tests::the_prune_keeps_each_open_sessions_current_snapshot_and_everything_recent ... FAILED
  only old rows that are not an open session's current snapshot go
  left: [("a-ended", 1), ("a-ended", 2), ("a-open", 1), ("a-open", 2), ("b-ended", 1), ("b-ended", 2), ("b-open", 1), ("b-open", 2), ("b-open", 3)]
 right: [("a-open", 2), ("b-ended", 1), ("b-ended", 2), ("b-open", 2), ("b-open", 3)]
snapshot_retention_tests::a_pass_is_bounded_to_forty_batches_of_five_hundred_with_a_pause_between ... ok   (const pin; compile-RED only)
```
All seeding succeeded, so these are assertion failures, not setup failures.

**3. Reconciler RED (the real tree, no stubs):**
```
cargo test -p gridops-reconciler --bin gridops-reconciler -- snapshot_retention_tests
snapshot_retention_tests::retention_keeps_the_configured_days_of_snapshots_seven_by_default ... FAILED
  panicked at crates/gridops-reconciler/src/snapshot_retention_tests.rs:141:5: the default is 7 days
  left: [1, 2, 100]   right: [2, 100]
snapshot_retention_tests::a_failing_snapshot_prune_does_not_stop_the_rest_of_the_cleanup ... ok   (guard)
```

**Correction on the way:** my first reconciler seed used a credential verifier that the `host_credentials_verifier_format_insert` trigger rejects. That was a setup failure, so it was not a valid RED. I fixed the seed to a 43-character base64url verifier and re-ran. The output above is from the fixed seed.

**4. API RED (the real tree, no stubs):**
```
cargo test -p gridops-api --test fleet_http -- snapshot_retention_setting
snapshot_retention_setting::snapshot_retention_days_are_a_setting_left_alone_when_a_save_omits_them ... FAILED
  panicked at crates/gridops-api/tests/fleet_http/snapshot_retention_setting.rs:45:5: seven days by default
  left: Null   right: Number(7)
```

**5. The tests can pass, and they catch mutations.** All of this used throwaway code, reverted afterwards (`git status` shows only test files):

**Core.** A correct loop over `PRUNE_BATCH` plus a throwaway 0049 makes all 4 tests **pass**. Each mutation then fails:

| Mutation | Fails |
|---|---|
| M1: drop `s.ended_at IS NULL` | test 1: `("a-ended", 2)` survives |
| M2: drop the `session_id` match | test 1: `("a-ended", 2)` survives |
| M3: `ORDER BY snap.rowid` | the plan test (`SCAN snap`), and the batch test (the newest rows go first) |
| M4: `<=` cutoff | test 1: the at-cutoff `("b-open", 2)` is deleted |
| M5: ignore `max_batches` | the batch test: `left: 15 right: 4` |

**Reconciler.** Throwaway wiring in `cleanup_retention` makes both tests **pass**. The wiring reads the setting with default 7, clamps it to `1..=3650`, calls the prune with `BATCH_SIZE` and `MAX_BATCHES`, and logs an error at `warn` without returning it. Each mutation then fails:

| Mutation | Fails |
|---|---|
| M6: no clamp | `left: [100] right: [6, 100]` |
| M7: default 30 | `left: [1, 2, 100]` |
| M8: propagate the prune error with `?` | the guard fails: `snapshot deletes refused by the test` |
| M9: naive `DELETE WHERE received_at < ?` | the open session's current row is deleted: `left: [2] right: [2, 100]` |

**API.** A throwaway optional field, GET default, range check, store and audit make the test **pass**. Each mutation then fails:

| Mutation | Fails |
|---|---|
| M10: no range check | `200` where `400` is expected |
| M11: omitted resets to 7 | `left: 7 right: 30` |
| M12: not audited | `left: Null right: 30` |

**Suite and lint.** With the throwaway core implementation and 0049 in place:
- the whole `cargo test -p gridops-core` passes: 315 lib tests, and every integration binary including `fleet_schema` (17) and `retention`;
- CI's lint passes: `cargo +1.98.1 clippy -p gridops-core -p gridops-api -p gridops-reconciler --all-targets --locked -- -D warnings -A clippy::unused_async_trait_impl` exits 0.

## Notes for F
- **Shared target directory hazard.** The worktrees share `~/.cache/cargo-target/gridops`, and Cargo hashes path crates by their workspace-relative path. Another worktree's build of `gridops-core` can replace this worktree's artifact, and Cargo's mtime check then counts it as fresh. The symptom is a spurious `no snapshot_retention in fleet::registry` in the reconciler build while core compiles fine.
  - I hit this twice.
  - `find crates -name '*.rs' -exec touch {} +` before building forces a rebuild from this worktree.
- **The batch test calls the function with `batch_size = 4`.** The function must use its arguments, not the consts. The reconciler passes the consts.
- **`BATCH_PAUSE` is only slept between batches.** The core tests run up to 2 batches per call, so a pause before the first batch, or after the last, only slows them down.
- **The guard's trigger aborts every snapshot DELETE.** The prune's error must be logged at `warn` and swallowed, wherever F places the call in `cleanup_retention`.

## Ready for F
**RED is valid.** Every new behaviour test fails on its feature assertion: compile RED for the missing core contract, then runtime RED behind it, and runtime RED in the reconciler and API. Every one is satisfiable, and each catches the mutations listed. F may implement against these tests.
