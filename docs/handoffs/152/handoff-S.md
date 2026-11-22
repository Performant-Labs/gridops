# Handoff-S: Phase 8 - Prune fleet_inventory_snapshots: it grows without bound (#152)

**Date:** 2026-11-22
**Branch:** claude/fix-152-prune-snapshots (HEAD b2e58b5; code last changed at 5b32f1e)
**Issue:** #152
**Brief reviewed:** docs/handoffs/152-brief.md (amended)
**Handoff-T reviewed:** docs/handoffs/152/handoff-T-green.md (and handoff-T-red.md)
**Handoff-A reviewed:** docs/handoffs/152/handoff-A.md (plan review, PASS) and handoff-A-dup.md (anti-duplication, PASS)
**Handoff-F reviewed:** docs/handoffs/152/handoff-F.md
**Operator-facing report:** N/A (no UI surface; no visual audit)

## A precondition
Confirmed: A returned PASS on the plan (handoff-A.md, 0 block, 1 warn, applied in test 5) and PASS on the anti-duplication gate (handoff-A-dup.md, 2 non-blocking warns).

## T precondition
Confirmed: T-green reports zero blocking issues. The one failing test in the API run,
`release_preflight::the_release_preflight_fails_early_without_a_macos_arm64_machine`, is macOS-host dependent, is not in the diff, and fails identically with F's changes stashed.

## Visual-diff-tool precondition
N/A. The change has no visual surface (no UI change, per the brief). Browser and pixel-diff preconditions do not apply and no HTML report is produced.

## Visual diff results
N/A (no UI).

## Preview / spec sanity check
Done before auditing; no defect in the source of truth. The brief's one earlier defect (the unbounded `SCAN snap`) was fixed in the amended brief and its sessions-step wording was corrected by A. The 25 ms pause is above F's measured per-batch maximum (14 ms). No ADVISORY-HOLD.

## Independent checks I ran (S, at HEAD)
- Reader list. `grep fleet_inventory_snapshots` over `crates`, `src` and `migrations`, excluding tests: the readers are `registry/mod.rs:1291`, `inventory.rs:98` (resend) and `:171` (insert), and `queries.rs:1134`, `:1354`, `:1912`. Each reader joins on the session's current sequence and requires `ended_at IS NULL`. There is no reader of a non-current row, and no DELETE trigger or foreign key on the table. This matches the brief.
- Resend path. `load_current_session` (`inventory.rs:404-411`) selects `... AND ended_at IS NULL`, so a resend can never reach an ended session. Pruning an ended session's current snapshot is therefore safe, as the brief decided.
- `received_at` is written from `now` in milliseconds (`inventory.rs:171-185`), the same unit as the cutoff.
- `cleanup_retention` runs on every reconcile pass (`main.rs:640`, `:697`), so the "drains over 8-9 passes" estimate holds.
- Web client. `settings.general.tsx` builds its PUT body from named form fields and only adds `macosVmSlots` when present, so omitting the new optional field keeps the stored value. `operations.functions.ts` types the GET loosely; the extra GET field is harmless.
- Tests at HEAD: `cargo test -p gridops-core --lib -- snapshot_retention a_snapshot_prune_batch` gave 4 passed (`CARGO_BUILD_JOBS=2 RUST_TEST_THREADS=2`, after touching sources for the shared-target hazard).
- Mutation spot checks, both reverted (`git status` clean; `snapshot_retention.rs` byte-identical to the original):

| Mutant (throwaway, `snapshot_retention.rs`) | Core suite result |
|---|---|
| P: delete the `tokio::time::sleep(BATCH_PAUSE)` between batches | all 4 tests PASS (mutant survives) |
| G: delete the `batch_size < 1` guard (a negative `LIMIT` is no limit in SQLite) | all 4 tests PASS (mutant survives) |

## Spec compliance
| Requirement (brief) | Compliant | Notes |
|---|---|---|
| Retention rule: delete only when old AND not the current snapshot of a session with `ended_at IS NULL` | YES | `PRUNE_BATCH` is the brief's statement verbatim in shape; strict `<` cutoff; test 1 covers old non-current, old open-current (kept), recent, old ended-current (deleted), and a row exactly at the cutoff (kept). |
| No filtering on `receipt_state`; no change-detection history | YES | Not present. |
| Setting `inventorySnapshotRetentionDays`, default 7, read with `setting_i64`, clamped 1..=3650 at use | YES | `cutoff_millis` clamps; reconciler test covers default, stored 30, 0 to 1, 5000 to 3650. Key/default/range are core consts shared with the API (F's documented choice, follows `tart_host::VM_SLOTS_KEY`). |
| Settings API: optional field, `macos_vm_slots` precedent, validated 1..=3650, GET `stored_i64(.., 7)`, in the `settings.updated` audit JSON, omitted leaves it alone, no web UI change | YES | `resources.rs` diff matches; API test covers default, store, audit, omit keeps, 0/-1/3651 refused with 400, 1/3650 accepted. |
| Documented with default and range; CHANGELOG | YES | No doc listed the retention settings, so a section was added to `docs/fleet-api.md` (allowed by the brief); two Upgrading entries and one Changed entry in CHANGELOG. `fleet_contracts` (parses the doc) passes. |
| New `pub mod snapshot_retention` in `fleet::registry`, free fn with the brief's signature, `pub const`s; called from `cleanup_retention` | YES | Not in `inventory.rs`. Signature matches. |
| Migration 0049: exactly `CREATE INDEX fleet_inventory_snapshots_received_idx ON fleet_inventory_snapshots(received_at);`, no `IF NOT EXISTS`, 0034-style header | YES | Verified; 0049 is the next free number. |
| One autocommit statement per batch, no `BEGIN`, no transaction across batches | YES | One `.execute(pool)` per batch. |
| Batch 500, pause const >= measured hold via `tokio::time::sleep`, stop on short batch or 40 batches | YES in code | `BATCH_SIZE=500`, `BATCH_PAUSE=25ms` slept only between batches, `MAX_BATCHES=40`. **The pause and the negative-LIMIT guard have no behavioural test (see finding 1 and 2).** |
| `info` log when non-zero; prune error at `warn` and does not fail the pass | YES | Guard test (`BEFORE DELETE` trigger) proves the error is swallowed. The `info` log is not tested (observability; accepted by T). |
| Do not run VACUUM from code; say so in the PR body | YES (code); PR body is O's | Code never runs VACUUM. See "Notes for O". |
| Tests 1 to 6 | YES | 1: core test 1. 2: core test 2. 3: API test. 4: reconciler test. 5: plan test (alias `s` per A). 6: `invariants_tests` and 17 `fleet_schema` tests pass. |
| Timing estimate in the result handoff | YES | handoff-F: per-batch median 2.3 ms, max 14.0 ms, total 123 ms for 20k rows; steady state 1 batch, 1.7 ms, 62 page misses, no scan; index build 0.10 s on the 85 MB seed; extrapolated to prod 8-9 passes, 1-3 s index build. Seed in the scratch dir. Not re-measured by me; the plan behind it is pinned by test 5. |
| Out of scope respected | YES | No `webhook_deliveries`, other retention deletes, `host_commands`, UI, deploy or prod touch. |

## Quality audit
| Area | Result | Notes |
|------|--------|-------|
| API consistency | PASS | No new route; one optional field on the existing settings contract. |
| Error handling | PASS | Prune errors logged at `warn` and swallowed; out-of-range values 400 with the existing message. |
| UI/UX match to spec | N/A | No UI. |
| Accessibility | N/A | No UI. |
| Architecture gate | PASS | A: PASS; A-dup: PASS. No finding of mine conflicts with A. |
| Code organization | PASS | Small, documented module; no dead code or TODOs. A-dup warn 2 (reconciler wrapper shares the core fn's name, aliased) is a non-blocking readability note. |
| Security | PASS | No new endpoint; existing settings auth path unchanged; bound parameters only. |
| Performance | PASS | Index-bounded batches (plan pinned), 500-row batches, pause between batches; steady state one cheap batch. |
| Visual regression | N/A | No UI. |
| Naming consistency | PASS | Matches the brief's names and the `macosVmSlots` precedent. |
| Test quality (`testing/test-quality.md` section 7) | FAIL | See findings 1 and 2 below. Six of the seven new tests pass the rubric. |

### Test-quality audit (per test)
| Test | Verdict | Notes |
|---|---|---|
| core `the_prune_keeps_each_open_sessions_current_snapshot_and_everything_recent` | PASS | One rule; fails on a dropped `ended_at`, a dropped `session_id` match, or `<=` (T-red M1, M2, M4). Two sessions share sequence 2 so only the session id separates them. |
| core `one_call_deletes_at_most_batch_size_times_max_batches_oldest_first` | PASS | One behaviour (the bound and the order), fails on ignoring `max_batches` or on `ORDER BY rowid` (M3, M5). |
| core `a_snapshot_prune_batch_reads_only_rows_older_than_the_cutoff` (plan) | PASS | Pins the plan on the app's bundled SQLite; the only place the plan is visible. |
| reconciler `retention_keeps_the_configured_days_of_snapshots_seven_by_default` | PASS | Four stages, each a distinct branch (fallback, stored, lower clamp, upper clamp); also proves the 400-day-old open-session current row survives a naive delete (M9). |
| reconciler `a_failing_snapshot_prune_does_not_stop_the_rest_of_the_cleanup` | PASS | Fails if the error is propagated with `?` (M8). Passes without the prune wired, which is why the sibling test owns wiring; acknowledged as a guard. |
| API `snapshot_retention_days_are_a_setting_left_alone_when_a_save_omits_them` | PASS | One contract. Nit (advisory): `-1` flows through the same lower-bound branch as `0`; harmless. |
| core `a_pass_is_bounded_to_forty_batches_of_five_hundred_with_a_pause_between` | **FAIL** | Finding 1. |

## Findings (blocking: REWORK, test-only)

1. **Delete or rewrite `a_pass_is_bounded_to_forty_batches_of_five_hundred_with_a_pause_between` (rubric 2, 4; smell: tautological change-detector).**
   - File: `crates/gridops-core/src/fleet/registry/snapshot_retention_tests.rs`, the last test.
   - It asserts `BATCH_SIZE == 500`, `MAX_BATCHES == 40` and `BATCH_PAUSE >= 25 ms` against the very constants the code defines. Its name claims a pause "between" batches, but it never observes one.
   - Breaking the code it names does not turn it red: with the `tokio::time::sleep(BATCH_PAUSE)` line deleted (mutant P), all four core tests, this one included, still pass.
   - The pause is the brief's cross-process lock-safety measure (the API's writers are in another process and SQLite's busy handler polls with 1 to 100 ms sleeps). It is the one load-bearing behaviour of the lock-safety requirement with no tripwire.
   - **Required:** replace it with a test that the pause is actually taken. Seed enough eligible rows for three full batches with a small `batch_size` (for example 2) and call with `max_batches = 3`. Measure `Instant::now()` around the call and assert only a lower bound: elapsed >= `2 * BATCH_PAUSE`. `tokio::time::sleep` never returns early, so a lower bound is deterministic and not flaky-by-design; do not assert an upper bound. The test must fail when the sleep is removed (re-run mutant P to confirm). If T wants to keep the brief's decided numbers visible, `assert!(BATCH_PAUSE >= Duration::from_millis(25))` may stay inside that same test; the 500 and 40 literal equalities add no independent signal and should go.

2. **Add a test for the `batch_size < 1` guard (rubric section 3: one test per branch added).**
   - File: same test file.
   - `prune_inventory_snapshots` returns `Ok(0)` without touching the database when `batch_size < 1`, because a negative `LIMIT` is no limit in SQLite and would turn a batch into the unbounded delete this issue exists to prevent. The branch has no test: with the guard removed (mutant G) every core test still passes.
   - **Required:** one test that seeds a few eligible rows, calls with `batch_size = 0` and `batch_size = -1` (with `max_batches > 0`), and asserts `Ok(0)` and that every row remains. It must fail when the guard is removed (re-run mutant G to confirm).

**TEST-ONLY: no `src/` change required.** Both findings are confined to `snapshot_retention_tests.rs`. The production code is correct and matches the brief; I found no production defect.

## Scope check
F delivered exactly the brief's scope: the core module and 0049, the reconciler wiring, the optional settings field, the docs and CHANGELOG, and the evidence appendix. No over-delivery (no history, no `receipt_state` filtering, no VACUUM, no UI, no other retention changes) and no under-delivery.

## Verdict

REWORK (test-only) — spec compliance is complete and the production code is correct, but the test suite has two gaps found by the test-quality audit, both confirmed by surviving mutants.

1. Replace the constant-pin test with a deterministic lower-bound test that the inter-batch pause is slept (finding 1).
2. Add a test that `batch_size < 1` deletes nothing (finding 2).

After these two test edits, with the rest of the suite unchanged and green, this is ready for O to commit. The rest of the audit stands: all other criteria are PASS and need no re-check beyond re-running the core retention tests and the two mutants.

## Advisory notes
- **Notes for O (PR body, per the brief and F's handoff):**
  - Migration 0049 builds the `received_at` index once, under the write lock, when the services start on the new version. F's estimate is about 1 to 3 s for the 711 MB table (extrapolated from 0.10 s on an 85 MB seed with a warm cache). A cold cache has to read the whole table, so say "seconds", not a guarantee. The CHANGELOG already says "a second or two".
  - The first prune on Mars (about 150k to 170k eligible rows) drains over about 8 to 9 reconcile passes at 20,000 rows a pass, roughly 4 to 5 minutes at the 30 s interval.
  - The database file stops growing but does not shrink. The code never runs `VACUUM`; to reclaim space run it while GridOps is stopped.
  - Timing came from the sqlite3 3.54.0 CLI on a warm page cache, not the app's bundled 3.51.3 or Mars hardware; cold-cache per-batch times on Mars will be higher than the local medians (still far below the 5 s `busy_timeout`).
  - `release_preflight::the_release_preflight_fails_early_without_a_macos_arm64_machine` fails on a macOS dev host with or without this change; it should pass on Linux CI.
- No test covers the `info` log of rows deleted or lock release under a concurrent writer (timing-dependent). Accepted by T; covered by code review (one `execute` per batch, no `begin()`) and F's timings.
- A-dup warn 2: the reconciler wrapper `prune_inventory_snapshots` shares its name with the core fn it calls (aliased as `retention`). An optional rename such as `prune_snapshots_step` would help grep.
