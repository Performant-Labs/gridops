# Handoff-F: Phase 6 - Fix scoring, rollup and speed-index defects from the wave-1 review (#239)

**Date:** 2026-11-24
**Branch:** claude/fh-239 (head b112cc1, T-red; code base 73de941)
**Issue:** #239

## What was done
- `crates/gridops-core/src/fleet_health/mod.rs`: adds `pub const ROLLUP_GRACE_MILLIS: i64 = 15 * 60_000` beside `HOUR_MILLIS`, the one copy of the grace rule (A warn 3).
- `crates/gridops-core/src/fleet_health/score.rs`: items 1-5.
  - **Item 1.** Private helpers `first_hour`, `covered_millis` and `covered_minutes`.
    - Availability, fleet and machine, divides by the whole covered minutes, with a `> 0` guard on both paths. 0 covered minutes is `None` (A warn 2).
    - Busy share divides by the covered seconds.
    - `capacity_misfit`'s first hour uses `first_hour`.
  - **Item 2.** The infra share divides by `job_count`.
  - **Item 3.** Machine availability is `None` only when the machine has no rows.
  - **Item 4.** A machine with a CPU budget of 0 has every component `None`, so `score: None` and `busy_share: None`.
  - **Item 5.** `component` becomes a private `measure` that returns `Measured { component, raw }`.
    - Each public component function is a one-line wrapper over a private `measure_*`.
    - `combine` wraps a private `combine_measured`, which rounds only the weighted sum.
    - `score_fleet` uses the `measure_*` functions and `combine_measured`.
- `crates/gridops-core/src/fleet_health/speed.rs`: items 10 and 11.
  - **Item 10.** `qualifying.retain(ratio > 1.0)` runs before `top_keys` is sorted and truncated. The index still uses every qualifying key.
  - **Item 11.** `grouped_durations` joins the newest `runners` row and its newest placement, using #191's two subqueries copied inline (A warn 6).
- `crates/gridops-reconciler/src/fleet_health_rollup.rs`: item 6, item 7 and A warn 1.
  - **Item 6.** A new `EARLIER_ATTEMPT_HOURS` read runs in the read phase through `workflow_jobs_run_status_idx`.
    - It returns the hours of the last 7 days in which the hour's re-runs' earlier attempts completed, the hour itself excluded.
    - `write_hour` takes `earlier: &[i64]` and lists those hours with `INSERT … ON CONFLICT(hour) DO NOTHING`. The inserts go in the last write batch, before that batch's own `DELETE FROM fleet_health_recompute WHERE hour=?`.
  - **Item 7 and A warn 1.** Both the listed loop and the catch-up wait for `hour + HOUR_MILLIS + ROLLUP_GRACE_MILLIS <= now`.
  - The module docs say the job rollup is the only reader of the recompute queue.
- `crates/gridops-reconciler/src/fleet_host_rollup.rs`: items 8 and 9.
  - **Item 8.** The listed loop and the `DELETE FROM fleet_health_recompute` are removed.
  - **Item 9.** The catch-up rolls `next` only when `next < floor_hour(now − ROLLUP_GRACE_MILLIS)`, which is the same as `next + 1 h + grace <= now` for a whole-hour `next`.
  - The module and function docs are updated.
- `docs/design/fleet-health.md`: rules (a)-(i).
  - **The hourly rollups section:**
    - which rollup reads the queue (f);
    - the grace (e), with "an hour has rows only once past its grace";
    - the earlier-attempt listing and its interim double count (d, A warn 4);
    - the `fleet_health_recompute` paragraph (f).
  - **The speed index:** the newest runner row and placement, the fleet median including the machine's own runs (i), and `topKeys` > 1.0 (g).
  - **Scores:**
    - new paragraphs on rounding (h), the covered time (a: the minute unit, the 0-minute case, busy seconds, capacity hours) and the grace-hour contract with #204 (a, including the past-grace-but-unwritten case);
    - the table rows for infra `÷ job_count` (b) and for availability and busy share over covered time;
    - the 0-minutes and budget-0 rules (c).
- `docs/handoffs/239/evidence.md`: new. It holds the unchanged-source facts the diff relies on.

## Design decisions
- **Item 5's shape.** A private `Measured` struct and private `measure_*` functions, with the public functions as thin wrappers.
  - The alternative was to recompute raw scores inside `score_fleet` from the same `higher`/`lower` closures. That would duplicate every component's scoring rule, so I didn't.
  - `ScoreComponent`, the wire format, and the signatures of `combine` and every public component function are unchanged, as the brief decided.
  - The busy-share gate still reads `queue.component.score`, the rounded score (rule (h); T's guard test stays green).
- **Item 4's wording.** A budget-0 machine's component keeps its own "not enough data" reason when it had nothing to measure anyway. When it did have data, the reason becomes "Not enough data: the machine has no CPU budget." (the `unbudgeted` helper).
  - The fixture's MacBookPro sends no samples, so its five reasons don't change, and `fake.rs`, `fixture.json` and the OpenAPI example need no edit.
  - Using one reason for every component would have meant editing those three files for no behavioural gain.
  - Using the plain no-data reasons would claim "no samples" for a budget-0 machine that does send them.
  - `NO_BUDGET` is now one constant, shared with busy share's existing reason.
- **Item 6's bound and batching.**
  - The earlier-attempt hours are limited to the job rollup's own 7-day reach. A listed hour older than every window would only be rewritten for nothing.
  - #179's 500-row limit is kept: when the last batch has no room for the listings, they get an empty batch of their own, which carries the hour's own recompute delete too. In practice an hour is far below 500 rows, so there is one transaction.
  - The SQL leaves out the hour itself (`completed_at` outside `[hour, hour+1h)`), so H is never listed. A warn 5 and T's `[a]` assertion check this.
- **Item 9's form.** `floor_hour(now − grace)` rather than writing `next + HOUR + GRACE <= now` out. The two are equivalent, and this keeps `floor_hour` used outside its existing unit test; removing it would have meant editing that test.

## Reuse / extend-vs-new
Every item extends the object that owns the behaviour, as the Reuse section of A's handoff says. No survey map exists for this run.
- `score.rs` (#192): the scoring entry point is still `score_fleet`, and `measure`/`combine_measured` are private refactors of `component`/`combine`.
- `speed.rs` (#193): the same `grouped_durations` query.
- `fleet_health_rollup.rs` (#191) and `fleet_host_rollup.rs` (#200): the same `run_pass`, `rollup_hour` and `write_*` functions.

The only new object is `ROLLUP_GRACE_MILLIS`, one constant, as A required. There is no new module, table, settings key or second scoring path.

## Architecture notes for A
- **Public API.** `gridops_core::fleet_health` gains one public constant, `ROLLUP_GRACE_MILLIS`, which #204 (`gridops-api`) needs. Nothing else public changes.
- **Contract change.** The machine rollup no longer reads or writes `fleet_health_recompute`. The job rollup now also *inserts* into it (earlier-attempt hours). The table's schema is unchanged and `0050` is not edited.
- **Layering.** Reads stay outside the write transactions: the new lookup runs next to `hour_facts`. The new read and the speed query's subqueries use indexes (EXPLAIN below).
- **Dependencies.** `the_scoring_engine_does_no_io` still holds. There is no new crate dependency.

## Deviations from spec / wireframe
None. One point A may want to see: rule (a)'s capacity-fit wording. The scorer counts the *ended* hours from `ceil_hour(now − W)`, which is unchanged behaviour, not the covered span. The doc says exactly that, rather than "covered hours".

## Tier 1 self-check (incl. tests now GREEN)
All runs used `CARGO_BUILD_JOBS=2 RUST_TEST_THREADS=2` and the shared target from the parent `.cargo/config.toml`. `CARGO_TARGET_DIR` was never set. Host load average during the runs was 22-33, from other sessions.

```
cargo test -p gridops-core --features test-support --lib
test result: ok. 365 passed; 0 failed; 3 ignored   (T-red: 353 passed, 12 failed)

cargo test -p gridops-reconciler
test result: ok. 151 passed; 0 failed; 3 ignored   (T-red: 146 passed, 5 failed)

cargo test -p gridops-core --features test-support --test '*'
every suite ok (fleet_health_contract 11 passed; fleet_health_speed_check 4 passed; ...)

cargo test -p gridops-api health
every suite ok (18, 11, 2 and 1 passed in the filtered suites)

cargo fmt -p gridops-core -p gridops-reconciler --check        -> clean
cargo +1.98.1 clippy -p gridops-core -p gridops-reconciler --all-targets --locked -- -D warnings -A clippy::unused_async_trait_impl
    Finished `dev` profile                                     -> clean
```

Every one of T's 17 RED tests now passes, and the preservation guard `the_busy_share_gate_reads_the_rounded_queue_wait_score` stays green. So do `on_the_fixture_the_weak_machines_score_lowest_and_its_concerns_come_out`, the speed index's 100 ms week check, and both week write-transaction checks.

**Query plans** for the new SQL, on a database migrated from `migrations/`:
```
EARLIER_ATTEMPT_HOURS: SEARCH wj USING INDEX workflow_jobs_completed_idx; sync by PK;
  SEARCH earlier USING INDEX workflow_jobs_run_status_idx (run_id=? AND status=?); earlier_sync by PK
speed grouped_durations: job via workflow_jobs_completed_idx; cand via runners_github_id_idx;
  latest via workload_placements_runner_idx
```

**Load flakes, recorded for T.** Under load average 22+, two timing checks each failed once and then passed on a rerun.
- `seven_days_of_50000_jobs_roll_up_with_no_long_write_transaction` failed once in a full suite run (no message captured). It passed alone in 12 s, and again in the next full run (151/151).
  - The week seed has no re-runs (`testdata.rs:903`), so this diff only adds one indexed read per hour there, outside the transaction.
- `the_synthetic_week_has_the_fixture_machines_at_realistic_volume` failed twice: "a write transaction held the write lock 71 ms" and then "56 ms (over 50 ms)". It passed on the third run.
  - That test only runs `testdata::build_week` (the seed) under `speed_check`, and this diff doesn't touch either file.

## Evidence appendix
`docs/handoffs/239/evidence.md`

## Tests that look wrong (for T)
None.

## Known issues
- The brief's Done-when asks for mutation checks on items 1, 2, 3 and 6. T-red assigns them to T's GREEN phase ("revert each fix and show its test fails again"), so I didn't run them here.
- Scope cap: this touches 6 production files across scoring, speed and two rollups. The split was already settled up front: A passed the plan and T authored the full suite against it, and each change is small and local to its item. So I didn't propose a split.

## Files changed
- `crates/gridops-core/src/fleet_health/mod.rs`
- `crates/gridops-core/src/fleet_health/score.rs`
- `crates/gridops-core/src/fleet_health/speed.rs`
- `crates/gridops-reconciler/src/fleet_health_rollup.rs`
- `crates/gridops-reconciler/src/fleet_host_rollup.rs`
- `docs/design/fleet-health.md`
- `docs/handoffs/239/evidence.md` (new, handoff material)
- `docs/handoffs/239/handoff-F.md` (this file)
