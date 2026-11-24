# Mutation tests on #192 and #193 (#246)

`cargo-mutants` 27.1.0 on the scoring engine (`score.rs`, `concerns.rs`, #192) and the speed
index (`speed.rs`, #193). Run on Jupiter, 2026-11-24, from master at `529cefe`.

## How it was run

```sh
CARGO_BUILD_JOBS=2 RUST_TEST_THREADS=2 cargo mutants -p gridops-core \
  --file crates/gridops-core/src/fleet_health/score.rs \
  --file crates/gridops-core/src/fleet_health/concerns.rs \
  --file crates/gridops-core/src/fleet_health/speed.rs \
  --timeout 300 --build-timeout 1200 -j 1 --no-shuffle \
  -- --lib fleet_health::
```

- The default test tool (`cargo test`), one mutant at a time, a 300 s timeout per mutant (the
  unmutated baseline is a 40 s build and a 7 s test run).
- The tests run are the library's `fleet_health::` tests: the modules' own tests, #197's
  contract tests (`score_contract_tests.rs`, `speed_contract_tests.rs`, including the 50,000-job
  week) and, after, `mutation_tests.rs`. Nothing outside `fleet_health` in `gridops-core` calls
  these three files, and no integration test under `crates/gridops-core/tests/` calls
  `score_fleet`, `concerns::windowed` or `speed_indexes`, so the filter only drops tests that
  cannot catch a mutant here. If anything, it counts more mutants as survivors, not fewer.
- The after-run is the same command with `--iterate --timeout 150`, which re-tests only the
  mutants the first run did not catch (missed or timed out). Each new test was written against a surviving mutant
  and passes on the unmutated code; the after-run is where each one is seen failing on its
  mutant (31 of the 31 targeted mutants caught, in 32 min).

## Counts

| | Generated | Caught | Missed | Timed out | Unviable | Score |
|---|---|---|---|---|---|---|
| Before | 541 | 464 | 35 | 8 | 34 | 93.1% |
| After | 541 | 495 | 4 | 8 | 34 | 99.2% |

Score = (caught + timed out) ÷ viable (generated − unviable): a mutant that makes the tests hang
is detected. Counting caught alone, it was 464 ÷ 507 = 91.5% before and 495 ÷ 507 = 97.6% after. The 4 left missed are the 4 equivalent mutants below: every non-equivalent mutant is now caught or hangs.

By file, before: `concerns.rs` 56 generated, 8 missed; `score.rs` 318 generated, 25 missed,
6 timed out; `speed.rs` 167 generated, 2 missed, 2 timed out.

## Every survivor and its disposition

(a) is a gap a new test in `crates/gridops-core/src/fleet_health/mutation_tests.rs` now kills,
(b) an equivalent mutant (nothing observable changes), (c) code that could be simpler.

### `concerns.rs`

| Mutant | Disposition |
|---|---|
| 48–52: delete each of the five title arms in `fleet_component` | (a) Nothing checked a fleet concern's title. `each_fleet_component_concern_has_its_own_title` |
| 100: `<` → `<=` in `cpus` | (b) The value is `cpu_millis ÷ 1000`, so its distance from a whole number is a multiple of 0.001 (up to rounding error far below 1e-9). It is never exactly 1e-9, where `<` and `<=` differ. |
| 217: `now - window` → `now / window` in `windowed` | (a) The only test ran at `now = 1 h`, where both give a start before every row. `the_other_share_reads_the_window_as_of_a_real_now` (at a real epoch `now`, with a row an hour before the window) |
| 230: `total > 0` → `>=` in `windowed` | (a) A window with no job would divide 0 by 0, and `NaN <= 0.2` is false, so the concern fires and reads `NaN%`. `a_window_without_jobs_raises_no_other_concern` |

### `score.rs`

| Mutant | Disposition |
|---|---|
| 25: `good - bad` → `good + bad`, `good / bad` in `higher`; 26: `>=` → `<` | (a) Nothing tested a target whose "100 at" equals its "0 at", which the settings allow. `a_target_with_equal_good_and_bad_values_is_a_step_not_a_division_by_zero` |
| 33: `good - bad` → `good + bad` in `lower`; 34: `<=` → `>` | (a) The same test, the less-is-better side. |
| 61: `<` → `<=` in `whole_percent` | (b) The test is `\|share × 100 − round\| < 1e-9`. That difference is a multiple of the ulp of a value of up to 100, and 1e-9 is not one, so the difference is never exactly 1e-9. |
| 71: `<` → `<=` in `gib` | (b) The same argument: a byte count over 2³⁰ minus a whole number is never exactly 0.05. |
| 427: `memory < disk` → `<=` in `measure_host_headroom` | (a) On a tie the sentence chose was untested; the score is the same, the deduction is not. `headroom_names_the_disk_when_disk_and_memory_tie` pins the disk sentence on a tie. |
| 511: guard `scored_weight > 0.0` → `true`; 511: `>` → `>=` | (a) They differ only when components are scored but weigh 0 in total: the weight became `0 × 100 ÷ 0 = NaN`. `scored_components_that_weigh_nothing_give_no_score_and_no_nan_weight` |
| 512: guard `scored_weight == 0.0` → `false` | (a) With nothing scored, the components must keep their table weights. `every_subset_of_missing_fleet_components_reweights_to_100` and `…_machine_…`, which check all 32 subsets of each table |
| 546: `count > 0` → `>=` in `percentile` | (a) It differs only at quantile 0 (or below) with empty leading buckets: it interpolated into an empty bucket, `0 ÷ 0`. `the_0th_percentile_skips_empty_leading_buckets` |
| 562: `attempts > 0` → `>=` in `start_success` | (a) It differs only when `min_jobs` is 0 (a setting) and nothing was started. `a_min_jobs_of_0_does_not_score_0_starts_or_0_jobs` (it also covers infrastructure failures' `total_jobs > 0`) |
| 611: `budget > 0.0` → `>=` in `fleet_online_share` | (a) A fleet with no CPU budget at all scored availability from `0 ÷ 0`. `a_fleet_without_cpu_budget_has_no_availability_score` |
| 627: `now - first` → `now + first`; 631: `first + index × 1 h` → `−`; 640: `>= 60_000` → `<`; 650: `row.hour == hour` → `!=`; 660: `used < budget / 2` → `<=` (all in `capacity_misfit`) | (a) The one test ran with the window starting at epoch 0 and its only misfit in the window's first hour, so each of these gave the same count. `capacity_fit_counts_each_hour_of_a_real_window_on_its_own_rows`: a real `now`, the misfit in hour 6, idle hours whose jobs waited under a minute, and an hour at exactly half the budget. |
| 660: `budget > 0.0` → `>=` in `capacity_misfit` | (b) and (c) `used` is a sum of `u32` p50s, so it is never negative. With a budget of 0, `used < budget / 2` is already false, so the `budget > 0.0 &&` guard never changes the result and could be removed. Left as it is: no production change without a defect. |
| 664: `counted > 0` → `>=` in `capacity_misfit` | (a) On 1h off the hour, no hour of the window has ended, so the share was `0 ÷ 0`. `capacity_fit_with_no_ended_hour_in_the_window_has_no_score` |
| 757: `total_bytes > 0` → `==`, `<`, `>=` in `score_fleet` | (a) No test had a machine whose disk was the lower half of its headroom, so dropping every disk went unseen, and none had a root disk of size 0. `headroom_reads_the_least_free_disk_and_ignores_a_0_size_or_0_budget` |
| 766: `memory_budget_mib > 0` → `>=` in `score_fleet` | (a) A memory budget of 0 scored memory at `p95 ÷ 0`. The same test. |

### `speed.rs`

| Mutant | Disposition |
|---|---|
| 71: `digits(shard) && digits(of)` → `\|\|` in `split_shard_counter` | (a) No test had a counter with digits on one side only (`unit (a/4)`). `a_counter_with_digits_on_one_side_only_is_part_of_the_name` |
| 133: `fleet_runs - host_runs < MIN_RUNS` → `<=` | (a) The rest of the fleet was never at exactly 5 runs. `five_runs_on_each_side_qualify_a_key_and_four_do_not` |

### Timed out (detected: the mutant hangs the tests)

`score.rs` 368 (`<` → `<=`), 369 (`/` → `%`, `-` → `+`) and 371 (`+` → `-`, `+` → `*`) in
`disk_floors::first_without`, 627 (`/` → `*`, a loop over about 3 × 10¹⁴ hours) in `capacity_misfit`,
and `speed.rs` 102 (`<` → `<=`) and 103 (`+` → `-`) in the hour-slice loop of `speed_indexes`.
Each turns a loop into one that never ends. They are caught by the timeout, not missed, so no
test is added for them.

## Defects found

None. Every survivor was a gap in the tests or an equivalent mutant; no production code
changed. Most survivors were at an edge the default targets never reach: a degenerate target, a
`min_jobs` of 0, a zero-weight or zero-budget input, quantile 0. The code already handled each
of those, and the new tests now pin that handling. The window-arithmetic survivors
(`concerns.rs:217`, `capacity_misfit`) were the most worth closing: tests run with `now` at
epoch 0 or 1 h cannot tell `now − W` from other arithmetic, so the new tests read as of a real
`now`.

## The edges the issue names

- Bands (`concerns::band`, the Rust side of poor / fair / good): every edge and one either side,
  0, 48–51, 68–71 and 100, for both the fleet-component and the machine concern
  (`the_concern_bands_hold_at_every_edge_and_one_either_side`). The page's good / fair / poor
  colours are `scoreBand` in `src/components/charts/chart-theme.ts`, outside these files.
- The 0 and 100 clamps: the contract tests already cover both ends of every component, and the
  reweighting tests use scores of 0 and 100.
- Reweighting: all 32 subsets of missing components, for the fleet and the machine tables. The
  weights sum to 100, a missing component weighs 0, the score is rounded once, and with nothing
  scored the table weights are kept.
- The gate's rounding: the busy-share gate deducts at a queue-wait score of 99, and not at 100 or
  with no queue score (`the_busy_share_gate_deducts_at_99_and_not_at_100_or_without_a_queue_score`).
- The speed index's minimum run counts: exactly 5 on each side qualify a key; 4 on either side
  do not. The `ratio > 1.0` filter: a key at exactly 1.0 is left out of `topKeys` and one a
  millisecond slower is kept.
- Every concern threshold one unit either side: `host-slow` at 1.3, `host-idle-while-queued` at
  30% and a queue score of 70, `category-other` at 20%
  (`each_concern_threshold_holds_one_unit_either_side`).
