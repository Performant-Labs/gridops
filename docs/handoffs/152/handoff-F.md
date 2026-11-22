# Handoff-F: Phase 6 - Prune fleet_inventory_snapshots: it grows without bound

**Date:** 2026-11-22
**Branch:** claude/fix-152-prune-snapshots (base 6abac67, T's RED)
**Issue:** #152

## What was done
- `crates/gridops-core/src/fleet/registry/snapshot_retention.rs` (new): the prune.
  - `PRUNE_BATCH` is the brief's statement, verbatim in shape. It binds `(cutoff_millis, batch_size)` and orders by `snap.received_at`.
  - The batch consts are `BATCH_SIZE` (500), `MAX_BATCHES` (40) and `BATCH_PAUSE` (25 ms).
  - The setting consts are `RETENTION_DAYS_KEY` (`"inventorySnapshotRetentionDays"`), `DEFAULT_RETENTION_DAYS` (7) and `RETENTION_DAYS` (`1..=3_650`).
  - `cutoff_millis(now, days)` clamps the days, then subtracts.
  - `prune_inventory_snapshots(pool, cutoff_millis, batch_size, max_batches) -> Result<u64, sqlx::Error>` runs the batches.
- `crates/gridops-core/src/fleet/registry/mod.rs`: declares `pub mod snapshot_retention;`.
- `migrations/0049_fleet_inventory_snapshot_retention.sql` (new): has a header in the 0034 style and exactly `CREATE INDEX fleet_inventory_snapshots_received_idx ON fleet_inventory_snapshots(received_at);`.
- `crates/gridops-reconciler/src/main.rs`: `cleanup_retention` calls a new `prune_inventory_snapshots(app, now)` as its last step.
  - That function reads the setting with `setting_i64` and the default of 7.
  - It calls the core prune with `BATCH_SIZE` and `MAX_BATCHES`.
  - It logs a non-zero count at `info` and an error at `warn`, and never returns the error.
- `crates/gridops-api/src/resources.rs`: the settings API gets the setting.
  - `SystemSettings` gets an optional `inventory_snapshot_retention_days` (`#[serde(default)] Option<i64>`), following `macos_vm_slots`.
  - GET returns `stored_i64(&stored, RETENTION_DAYS_KEY, 7)`.
  - Save refuses a value outside `1..=3650` with the existing range 400.
  - Save stores the value only when it is given, and adds it to the `settings.updated` audit JSON.
- `docs/fleet-api.md`: a new "Inventory snapshot retention" section after "Limits", covering the setting, default, range, the rule, the batching and the fact that the file does not shrink.
- `CHANGELOG.md`, under Unreleased:
  - two Upgrading entries: the 0049 index build, and the backlog drain with no shrink and `VACUUM` only while stopped;
  - one Changed entry for the prune and the setting.
- `docs/handoffs/152/evidence.md` (new): the unchanged-code facts the change relies on.

## Design decisions
- **The setting's key, default and range are core consts, shared by the API and the reconciler.**
  - The precedent is `gridops_core::tart_host::{VM_SLOTS_KEY, DEFAULT_VM_SLOTS, MAX_VM_SLOTS}`, which both crates use the same way.
  - The alternative was repeating the literal `"inventorySnapshotRetentionDays"`/`7`/`1..=3650` in two crates, the way the older retention settings do. I rejected it because the reconciler's clamp and the API's validation must agree, and a shared const guarantees that.
  - What goes over the wire is exactly what the brief specifies.
- **`cutoff_millis` is a small pure helper in core.** It keeps the clamp next to the range it clamps to. The reconciler test exercises it end to end through `cleanup_retention`.
- **A `batch_size` below 1 returns `Ok(0)` without touching the database.**
  - SQLite treats a negative `LIMIT` as no limit, so a `-1` would turn one batch into the unbounded delete this issue exists to prevent.
  - Only callers with a bad argument can reach this guard: the reconciler passes `BATCH_SIZE`.
- **The loop stops when `rows < batch_size`.** That covers both the last partial batch and an empty one. `BATCH_PAUSE` is slept only between batches, never before the first or after the last, as T's notes ask.
- **The prune runs last in `cleanup_retention`.**
  - So a pass with up to 40 batches (about 1 s of wall time including the pauses, see the timing below) runs after the cheap single-statement deletes.
  - Its error is swallowed, so nothing after it can be skipped.
  - The existing deletes before it keep their `?`. That is unchanged behaviour, and the brief puts batching them out of scope.
- **One API range message.** An out-of-range snapshot-retention value gets the existing "Retention or reconciliation settings are outside the supported range." 400, like the other retention days. No new message was needed.

## Reuse / extend-vs-new
- **Extended:**
  - `cleanup_retention` and `setting_i64` (reconciler);
  - `SystemSettings`, `settings` (GET) and `save_settings` (API), following the `macos_vm_slots` optional-field precedent;
  - the `fleet::registry` module tree;
  - the migrations sequence, in the 0034 style.
- **New: `fleet::registry::snapshot_retention`.** The brief and A's handoff justify it explicitly: not `inventory.rs`, which is the private, transactional and audited ingest path. It follows the `fleet::targets` free-fn and `registry::logs` precedents, and the #179 placement (`retry::LAST_FAILURE`).

## Architecture notes for A
- **Layers:**
  - core: a new public module, plus a migration;
  - reconciler: wiring;
  - API: a settings field.
- **No new dependencies.** `tokio::time` comes from the workspace's `full` features.
- **Public surface:** the new `gridops_core::fleet::registry::snapshot_retention` module. Its items are listed under "What was done".
- **Schema:** one new index, `fleet_inventory_snapshots_received_idx`. No table changes.
- **HTTP contract:**
  - `GET /api/v1/settings` gains `data.settings.inventorySnapshotRetentionDays`.
  - `PUT` accepts an optional `inventorySnapshotRetentionDays` and leaves it alone when the field is omitted.
  - No route is added, so the fleet-api.md "Who may call what" table is unchanged.
- **Dependency direction is unchanged:** API → core and reconciler → core.

## Deviations from spec / wireframe
None.
- The brief's GET expression is `stored_i64(&stored, "inventorySnapshotRetentionDays", 7)`. The code passes the same key and default through the core consts.
- No doc listed the retention settings, so the docs went into `docs/fleet-api.md`, which the brief allows.

## Tier 1 self-check (incl. tests now GREEN)
All runs used `CARGO_BUILD_JOBS=2 RUST_TEST_THREADS=2` and the shared target from the untracked `.cargo/config.toml`. `CARGO_TARGET_DIR` was never set. I ran `find crates -name '*.rs' -exec touch {} +` before each build because of the shared-target hazard T describes, and I hit that hazard twice (see Known issues).

**T's tests, now GREEN:**
```
cargo test -p gridops-core --lib -- snapshot_retention a_snapshot_prune_batch
test ...snapshot_retention_tests::a_pass_is_bounded_to_forty_batches_of_five_hundred_with_a_pause_between ... ok
test ...query_plan_tests::a_snapshot_prune_batch_reads_only_rows_older_than_the_cutoff ... ok
test ...snapshot_retention_tests::one_call_deletes_at_most_batch_size_times_max_batches_oldest_first ... ok
test ...snapshot_retention_tests::the_prune_keeps_each_open_sessions_current_snapshot_and_everything_recent ... ok
test result: ok. 4 passed; 0 failed

cargo test -p gridops-reconciler --bin gridops-reconciler -- snapshot_retention_tests
test snapshot_retention_tests::a_failing_snapshot_prune_does_not_stop_the_rest_of_the_cleanup ... ok
test snapshot_retention_tests::retention_keeps_the_configured_days_of_snapshots_seven_by_default ... ok
test result: ok. 2 passed; 0 failed

cargo test -p gridops-api --test fleet_http -- snapshot_retention_setting macos_vm_slots
test snapshot_retention_setting::snapshot_retention_days_are_a_setting_left_alone_when_a_save_omits_them ... ok
test result: ok. 11 passed; 0 failed   (including all macos_vm_slots tests, the optional-field precedent)
```

**Full suites:**
```
cargo test -p gridops-core -p gridops-reconciler          (exit 0)
  gridops-core lib: 315 passed; every integration binary ok, incl. fleet_schema 17, fleet_contracts 4
                    (parses docs/fleet-api.md), fleet_inventory 2, retention 1
  gridops-reconciler: 127 passed
cargo test -p gridops-api --no-fail-fast
  lib 130 passed; control_plane_contracts 39 passed; fleet_http: see FLEET_HTTP below
```
```
cargo test -p gridops-api --test fleet_http --no-fail-fast      (rerun right after a fresh build)
test release_preflight::the_release_preflight_fails_early_without_a_macos_arm64_machine ... FAILED
test result: FAILED. 150 passed; 1 failed      (the one failure predates this change, see Known issues)
```

**Lint and format:**
```
cargo +1.98.1 clippy -p gridops-core -p gridops-api -p gridops-reconciler --all-targets --locked -- -D warnings -A clippy::unused_async_trait_impl
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 35.33s     (no warnings)
cargo fmt --all -- --check   -> exit 0
```

**Timing estimate (required by the brief).** This used the sqlite3 3.54.0 CLI, not the app's bundled 3.51.3, on a file-backed WAL database in the scratch dir (`…/scratchpad/f152/`, not the repo).
- **The seed:** all 48 migrations, then 8 hosts with one open session each and 2,500 snapshots per host. That is 20,000 rows averaging 3,910 bytes of `snapshot_json`, and the database is 85 MB.
- **Drain** (every row except the 8 current ones is older than the cutoff): one call's worth of the `PRUNE_BATCH` statement, 40 batches of 500 rows, deleted 19,992 rows and kept the 8 current snapshots.
  - Per-batch lock hold: **median 2.3 ms, p90 4.4 ms, max 14.0 ms** (the first, colder batch). The minimum was 1.8 ms.
  - Total statement time: **123 ms**. Add the 39 × 25 ms pauses and the pass takes about **1.1 s** of wall time, of which about 0.12 s holds the lock.
- **Steady state** (20,000 rows, 5 eligible): one batch, **1.7 ms**, **62 page-cache misses** (cold connection), **0 fullscan steps**, 0 sort operations. It deleted 5.
- **Building the 0049 index:** **0.10 s** on the 85 MB, 20k-row seed, and 0.13 s on the second seed.
- **Extrapolation to prod (Mars):**
  - **First prune.** About 150k to 170k eligible rows at 20,000 per pass gives **about 8 to 9 passes**, roughly 4 to 5 minutes at the 30 s reconcile interval.
  - **Per-batch lock time.** A few ms warm, and plausibly tens of ms on a cold page cache: 500 rows are about 520 pages, versus 5 s of `busy_timeout`.
  - **Steady state.** After the backlog clears, each pass deletes about 7,000 rows a day / 2,880 passes a day ≈ 2 to 3 rows. That is one batch of a few ms, reading only rows older than the cutoff.
  - **Index build at deploy.** The table is 711 MB and 181k rows, about 8.4× the seed. That is about **1 s with a warm cache**, plus the time to read 711 MB if the cache is cold, so expect about **1 to 3 s**. It runs once, under the write lock, during the migration when the services start.
- **For the PR body (O):**
  - The deploy's migration builds this index once, under the write lock, in about 1 to 3 s.
  - The file stops growing but does not shrink.
  - The code never runs `VACUUM`. To get the space back, run `VACUUM` while the services are stopped.

## Evidence appendix
`docs/handoffs/152/evidence.md`. It covers:
- the `setting_i64` fallback;
- `load_current_session` requiring `ended_at IS NULL`, which confirms the brief's open question: the resend path can't reach an ended session, so the current snapshot of an ended session may be pruned;
- the resend reading only the current sequence;
- the four other readers, each joined on the current sequence with `ended_at IS NULL`;
- no DELETE trigger on the snapshots table;
- the 5 s `busy_timeout`.

**Reader list verified.** `grep fleet_inventory_snapshots crates src`, excluding tests, finds the same readers as the brief: `registry/mod.rs:1291`, `inventory.rs:98` (resend), `inventory.rs:171` (insert), and `queries.rs:1134`, `:1354` and `:1912`. There is no reader of a non-current row.

## Tests that look wrong (for T)
None.

## Known issues
- **`release_preflight::the_release_preflight_fails_early_without_a_macos_arm64_machine` fails on this Mac, and the failure predates this change.**
  - The test's "Linux machine" reports this Mac's OS, so preflight finds a "macOS ARM64 machine ready". It fails identically with all of my production changes stashed (`git stash push -- resources.rs mod.rs main.rs`, then the same run gives `FAILED`).
  - It should pass on CI's Linux runners. It is unrelated to #152.
- **The shared cargo target produced two spurious failures.**
  - One full API run showed the new settings test with `left: Null`, and `recovery::an_api_restart_leaves_a_running_runner_alone` with "fleet API did not start".
  - `fleet_http` launches `env!("CARGO_BIN_EXE_gridops-api")` from the shared target dir, and another worktree's build replaced that binary mid-run.
  - The rerun after a fresh build is recorded above.
- **The timing comes from the sqlite3 3.54.0 CLI with a warm OS page cache, not from the app's bundled 3.51.3 or from prod hardware.** The plan is the same on both versions: test 5 pins it on 3.51.3 and passes. Cold-cache per-batch times on Mars will be higher than the local medians.

## Files changed
- `crates/gridops-core/src/fleet/registry/snapshot_retention.rs` (new)
- `crates/gridops-core/src/fleet/registry/mod.rs`
- `migrations/0049_fleet_inventory_snapshot_retention.sql` (new)
- `crates/gridops-reconciler/src/main.rs`
- `crates/gridops-api/src/resources.rs`
- `docs/fleet-api.md`
- `CHANGELOG.md`
