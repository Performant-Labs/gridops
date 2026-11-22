# Handoff-S: Phase 8 - Prune fleet_inventory_snapshots: it grows without bound (#152)  (re-audit after the test-only REWORK)

**Date:** 2026-11-22
**Branch:** claude/fix-152-prune-snapshots (HEAD c0fe195; production code last changed at 5b32f1e, tests last changed at 28eb691)
**Issue:** #152
**Brief reviewed:** docs/handoffs/152-brief.md (amended)
**Handoff-T reviewed:** docs/handoffs/152/handoff-T-green.md (including its "Test-only rework" section) and handoff-T-red.md
**Handoff-A reviewed:** docs/handoffs/152/handoff-A.md (plan review, PASS) and handoff-A-dup.md (anti-duplication, re-checked after the rework, PASS)
**Handoff-F reviewed:** docs/handoffs/152/handoff-F.md
**Operator-facing report:** N/A (no UI surface; no visual audit)

This supersedes my earlier REWORK (commit 61d8832). That round found two test-only gaps; both are fixed and I re-verified them with the same two mutants.

## A precondition
Confirmed: A returned PASS on the plan (handoff-A.md: 0 block, 1 warn, applied in test 5) and PASS on the anti-duplication gate, re-run after the rework (handoff-A-dup.md: 2 non-blocking warns, no source file changed since the earlier PASS).

## T precondition
Confirmed: T-green reports zero blocking issues. The one failing test in the API run, `release_preflight::the_release_preflight_fails_early_without_a_macos_arm64_machine`, depends on the macOS host, is not in the diff, and fails identically with F's changes stashed (F's handoff). It should pass on Linux CI.

## Visual-diff-tool precondition
N/A. The change has no visual surface (no UI change, per the brief), so the browser and pixel-diff preconditions do not apply and no HTML report is produced.

## Visual diff results
N/A (no UI).

## Preview / spec sanity check
Done before auditing; no defect in the source of truth. The brief's earlier defect (an unbounded `SCAN snap`) was fixed in the amended brief, and A corrected the sessions-step wording of test 5. F's measured per-batch maximum (14 ms) is below the 25 ms pause. No ADVISORY-HOLD.

## What changed since my last audit
`git diff 5b32f1e..HEAD` touches only `snapshot_retention_tests.rs` and handoff files. **No `src/` file changed.** `snapshot_retention.rs` is byte-identical to the version I audited (sha1 2cbeae21... before and after my mutants).

- Finding 1 (constant-pin test never observed the pause): the test `a_pass_is_bounded_to_forty_batches_of_five_hundred_with_a_pause_between` is deleted. `a_pass_pauses_between_its_batches` replaces it. It seeds 7 eligible rows, calls with `batch_size` 2 and `max_batches` 3, and asserts 6 rows deleted and elapsed >= `2 * BATCH_PAUSE`. There is no upper bound. The `BATCH_PAUSE >= 25 ms` check stays inside it, which I allowed. The 500 and 40 literal equalities are gone.
- Finding 2 (the `batch_size < 1` guard had no test): `a_batch_size_below_one_deletes_nothing` seeds 3 eligible rows, calls with `batch_size` 0 and -1, and asserts `Ok(0)` and that all 3 rows remain.

## Independent checks I ran (S, at HEAD)
- Core: `cargo test -p gridops-core --lib -- snapshot_retention a_snapshot_prune_batch` gave **5 passed** (`CARGO_BUILD_JOBS=2 RUST_TEST_THREADS=2`, sources touched first to avoid the shared-target stale binary; I checked the test count, not just the exit code). The five are: keeps-open-current, one-call-bound, pauses-between-batches, batch-size-below-one, and the plan test.
- Reconciler: `snapshot_retention_tests` gave 2 passed.
- API: `fleet_http -- snapshot_retention_setting macos_vm_slots` gave 11 passed (the new test and all `macos_vm_slots` precedent tests).
- `cargo fmt --all -- --check`: exit 0. `cargo +1.98.1 clippy -p gridops-core -p gridops-reconciler --all-targets --locked -- -D warnings -A clippy::unused_async_trait_impl`: clean.
- **Mutation spot checks, re-run by me, each reverted** (`snapshot_retention.rs` restored from a copy; `git status` and `git diff` empty afterwards):

| Mutant (throwaway, `snapshot_retention.rs`) | Result | Killed by |
|---|---|---|
| P: delete the `tokio::time::sleep(BATCH_PAUSE)` between batches | 1 failed, 4 passed | `a_pass_pauses_between_its_batches`: `637.416µs < 2 * 25ms` |
| G: delete the `batch_size < 1` guard (`let full = batch_size as u64;`) | 1 failed, 4 passed | `a_batch_size_below_one_deletes_nothing`: at `batch_size -1`, left 3, right 0 (the unbounded delete) |

Both mutants that survived last round are now killed by exactly the test written for them, and only that test fails in each case.

- Earlier checks that still hold (no production change since): the reader list (`registry/mod.rs:1291`, `inventory.rs:98` and `:171`, `queries.rs:1134`, `:1354`, `:1912`) has no reader of a non-current row, and no DELETE trigger or foreign key exists. `load_current_session` (`inventory.rs:404-411`) requires `ended_at IS NULL`, so a resend can never reach an ended session, which makes pruning an ended session's current snapshot safe. `received_at` is written in milliseconds, the unit of the cutoff. `cleanup_retention` runs every reconcile pass. The web client builds its PUT body from named fields, so omitting the new optional field keeps the stored value.

## Spec compliance
| Requirement (brief) | Compliant | Notes |
|---|---|---|
| Retention rule: delete only when old AND not the current snapshot of a session with `ended_at IS NULL` | YES | `PRUNE_BATCH` is the brief's statement in shape; strict `<` cutoff. Core test 1 covers old non-current (gone), old open-current (kept), recent (kept), old ended-current (gone), and a row exactly at the cutoff (kept). |
| No filtering on `receipt_state`; no change-detection history | YES | Not present. |
| Setting `inventorySnapshotRetentionDays`, default 7, read with `setting_i64`, clamped 1..=3650 at use | YES | `cutoff_millis` clamps; the reconciler test covers default, stored 30, 0 to 1, and 5000 to 3650. Key, default and range are core consts shared by the API and the reconciler (F's documented choice, following `tart_host::VM_SLOTS_KEY`). |
| Settings API: optional field, `macos_vm_slots` precedent, validated 1..=3650, GET `stored_i64(.., 7)`, in the `settings.updated` audit JSON, omitted leaves it alone, no web UI change | YES | `resources.rs` diff matches line for line. API test covers default, store, audit, omit keeps, 0/-1/3651 refused with 400, 1/3650 accepted. |
| Documented with default and range; CHANGELOG | YES | No doc listed the retention settings, so a section was added to `docs/fleet-api.md` (the brief allows this); two Upgrading entries and one Changed entry in CHANGELOG. `fleet_contracts` (parses the doc) passes. |
| New `pub mod snapshot_retention` in `fleet::registry`, free fn with the brief's signature, `pub const`s; called from `cleanup_retention` | YES | Not in `inventory.rs`. Signature `(pool, cutoff_millis, batch_size, max_batches) -> Result<u64, sqlx::Error>` matches. |
| Migration 0049: exactly `CREATE INDEX fleet_inventory_snapshots_received_idx ON fleet_inventory_snapshots(received_at);`, no `IF NOT EXISTS`, 0034-style header | YES | Verified by reading the file; 0049 is the next free number. |
| One autocommit statement per batch, no `BEGIN`, no transaction across batches | YES | One `.execute(pool)` per batch, no `begin()`. |
| Batch 500, pause const of at least the measured hold via `tokio::time::sleep`, stop on a short batch or 40 batches | YES | `BATCH_SIZE=500`, `BATCH_PAUSE=25ms` (above F's 14 ms max) slept only between batches, `MAX_BATCHES=40`. The pause and the negative-LIMIT guard now each have a behavioural test that mutants P and G kill. |
| `info` log when non-zero; prune error at `warn` and does not fail the pass | YES | The guard test (a `BEFORE DELETE` trigger) proves the error is swallowed. The `info` log is not tested (observability; accepted by T). |
| Do not run VACUUM from code; say so in the PR body | YES (code); PR body is O's | Code never runs `VACUUM`. See "Notes for O". |
| Tests 1 to 6 | YES | 1 and 2: core tests. 3: API test. 4: reconciler test. 5: plan test (alias `s`, per A). 6: `invariants_tests` and 17 `fleet_schema` tests pass (T-green). |
| Timing estimate in the result handoff | YES | handoff-F: per batch median 2.3 ms, p90 4.4 ms, max 14.0 ms, total 123 ms for 20k rows; steady state 1 batch, 1.7 ms, 62 page misses, no scan; index build 0.10 s on the 85 MB seed; extrapolated to prod 8 to 9 passes and a 1 to 3 s index build. Seed in the scratch dir. Not re-measured by me; the plan behind it is pinned by test 5 on the bundled SQLite. |
| Out of scope respected | YES | No `webhook_deliveries`, no other retention deletes, no `host_commands`, no UI, no deploy, no prod touch. |

## Quality audit
| Area | Result | Notes |
|------|--------|-------|
| API consistency | PASS | No new route; one optional field on the existing settings contract. |
| Error handling | PASS | Prune errors are logged at `warn` and swallowed; out-of-range values get the existing 400 message. |
| UI/UX match to spec | N/A | No UI. |
| Accessibility | N/A | No UI. |
| Architecture gate | PASS | A: PASS; A-dup: PASS (re-run after the rework). No finding of mine conflicts with A. |
| Code organization | PASS | Small, documented module; no dead code or TODOs. A-dup warn 2 (the reconciler wrapper shares the core fn's name, aliased `retention`) is a non-blocking readability note. |
| Security | PASS | No new endpoint; the settings auth path is unchanged; bound parameters only. |
| Performance | PASS | Index-bounded batches (plan pinned), 500-row batches, a pause between batches; steady state is one cheap batch. |
| Visual regression | N/A | No UI. |
| Naming consistency | PASS | Matches the brief's names and the `macosVmSlots` precedent. |
| Test quality (`testing/test-quality.md` section 7) | PASS | Eight tests, each naming one behaviour; the two earlier gaps are closed and mutation-confirmed. See below. |

### Test-quality audit (per test)
| Test | Verdict | Notes |
|---|---|---|
| core `the_prune_keeps_each_open_sessions_current_snapshot_and_everything_recent` | PASS | One rule; fails on a dropped `ended_at`, a dropped `session_id` match, or `<=` (T-red M1, M2, M4; T-green re-ran the `ended_at` mutant). Two sessions share sequence 2, so only the session id separates them. |
| core `one_call_deletes_at_most_batch_size_times_max_batches_oldest_first` | PASS | One behaviour (the bound and the order); fails on ignoring `max_batches` or on `ORDER BY rowid` (M3, M5). |
| core `a_pass_pauses_between_its_batches` | PASS | New. Asserts the pause is taken with a lower bound only (`tokio::time::sleep` never returns early), so it is deterministic. Kills mutant P (I re-ran it). Costs about 50 ms. Known limit: it cannot prove no pause follows the last batch, because that needs a wall-time upper bound that could flake; harmless, and the code sleeps only when `batch > 0`. |
| core `a_batch_size_below_one_deletes_nothing` | PASS | New. One branch (the guard), two inputs (0 and -1) on the same branch, asserts the return value and that rows remain. Kills mutant G (I re-ran it). |
| core `a_snapshot_prune_batch_reads_only_rows_older_than_the_cutoff` (plan) | PASS | Pins the plan on the app's bundled SQLite, the only place the plan is visible. |
| reconciler `retention_keeps_the_configured_days_of_snapshots_seven_by_default` | PASS | Four stages, each a distinct branch (fallback, stored, lower clamp, upper clamp); also proves the 400-day-old open-session current row survives a naive delete (M9). |
| reconciler `a_failing_snapshot_prune_does_not_stop_the_rest_of_the_cleanup` | PASS | Fails if the error is propagated with `?` (M8). It passes without the prune wired, which is why the sibling test owns wiring; acknowledged as a guard. |
| API `snapshot_retention_days_are_a_setting_left_alone_when_a_save_omits_them` | PASS | One contract. Nit (advisory): `-1` flows through the same lower-bound branch as `0`; harmless. |

Suite proportionality: eight tests for a feature story with a lock-safety requirement, tiered (core against real SQLite, reconciler wiring, HTTP contract), with no duplicate signals and no tautological test. No "delete or merge" finding remains.

## Scope check
F delivered exactly the brief's scope: the core module and 0049, the reconciler wiring, the optional settings field, the docs and CHANGELOG, and the evidence appendix. No over-delivery (no history, no `receipt_state` filtering, no `VACUUM`, no UI, no other retention changes) and no under-delivery. The REWORK round changed only tests.

## Verdict

PASS — all acceptance criteria are met, the implementation is spec-compliant, and quality is acceptable. Both earlier test-quality findings are fixed and confirmed by re-running mutants P and G, which each fail exactly the test written for them. Ready for O to commit and open the pull request.

## Advisory notes
- **Notes for O (PR body, per the brief and F's handoff):**
  - Migration 0049 builds the `received_at` index once, under the write lock, when the services start on the new version. F's estimate is about 1 to 3 s for the 711 MB table (extrapolated from 0.10 s on an 85 MB seed with a warm cache). A cold cache has to read the whole table, so say "seconds", not a guarantee. The CHANGELOG already says "a second or two".
  - The first prune on Mars (about 150k to 170k eligible rows) drains over about 8 to 9 reconcile passes at 20,000 rows a pass, roughly 4 to 5 minutes at the 30 s interval.
  - The database file stops growing but does not shrink. The code never runs `VACUUM`; to reclaim space, run it while GridOps is stopped.
  - Timing came from the sqlite3 3.54.0 CLI on a warm page cache, not the app's bundled 3.51.3 or Mars hardware. Cold-cache per-batch times on Mars will be higher than the local medians (still far below the 5 s `busy_timeout`).
  - `release_preflight::the_release_preflight_fails_early_without_a_macos_arm64_machine` fails on a macOS dev host with or without this change; it should pass on Linux CI.
- No test covers the `info` log of rows deleted or lock release under a concurrent writer (timing-dependent). Accepted by T; covered by code review (one `execute` per batch, no `begin()`) and F's timings.
- A-dup warn 2: the reconciler wrapper `prune_inventory_snapshots` shares its name with the core fn it calls (aliased as `retention`). An optional rename such as `prune_snapshots_step` would help grep.
- Shared cargo target hazard (recorded by F and T): another checkout's build can leave a stale "Fresh" test binary or replace the API binary mid-run. Check the test count, not only the exit code, when re-running.
