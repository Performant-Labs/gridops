# Handoff-T-red: Phase 4 - Fix scoring, rollup and speed-index defects from the wave-1 review (#239)

**Date:** 2026-11-24
**Branch:** claude/fh-239 (head c41abfa; code base 73de941)
**Brief / wireframe reviewed:** `docs/handoffs/239-brief.md` (amended, blob f4a2e56), `docs/handoffs/239/handoff-A.md` (round 2). Wireframe: N/A, no UI.

## A precondition
Confirmed. A returned **PASS** on the plan in round 2 (commit c41abfa, `chore(#239): a -- verdict: PASS`), with 0 block and 8 warn findings. Warns 1, 2 and 5-7 shape the tests below.

## Tests authored
Every test sits in the owning module's own `#[cfg(test)] mod tests` (A warn 7), except item 8's. Item 8 is the one #197 contract test the brief puts in scope. Each test uses the cheapest tier that shows its behaviour:
- **Unit tests** for the pure scorer (`score_fleet` over hand-built rows).
- **SQLite integration tests** through `testdata` wherever the behaviour lives in SQL or in a pass's timing.

None of the tests reference symbols that don't exist yet. In particular, `ROLLUP_GRACE_MILLIS` is written as the literal `15 * MINUTE`, so the whole suite compiles and each test fails only on its assertion.

### `crates/gridops-core/src/fleet_health/score.rs` (unit)
| Test | Item / rule | What it pins |
|---|---|---|
| `availability_on_1h_divides_by_the_covered_half_hour_not_the_full_window` | 1, rule (a) | At 10:30 on 1h, a machine online all 30 covered minutes scores availability 100, for the fleet and the machine. No `fleet-component:availability` concern fires. |
| `availability_on_24h_at_ten_past_counts_the_covered_23_hours_and_10_minutes` | 1 (the brief's grace-hour test) | At H+10 min on 24h, the 23 ended hours plus 10 live minutes are the denominator (1390 min). Score 100, not 83. |
| `availability_divides_by_whole_covered_minutes_and_none_covered_is_no_data` | 1 (A warn 2) | At H+5 min 29 s, 5 online of 5 whole covered minutes scores 100. At H+29 s, 0 covered minutes is `None` ("not enough data") for both the fleet and the machine, not 0. |
| `busy_share_on_1h_divides_by_the_covered_seconds` | 1 | 1800 CPU-seconds on 2 CPUs over the 1800 s covered gives a busy share of 0.5, not 0.25. |
| `infra_failures_divide_by_every_completed_job_so_an_outage_scores_0` | 2, rule (b) | 40 jobs, 0 successes, 40 infra failures: the component scores 0, not "not enough data", and a critical `fleet-component:infra-failures` concern fires. |
| `score_fleet_measures_every_fleet_component_from_the_window` (**pinned value updated**) | 2 | Infra share is 10 of 1000 jobs (`job_count`), not of the 880 successes: score 80 (was 77), and the deduction now reads "1.0% …: −3." This reverses e71dcf6, as the brief says. |
| `a_machine_offline_all_window_scores_availability_0_not_no_data` | 3, rule (c) | Rows with 0 online minutes score 0, with the deduction "Online 0.0% … −25.". A machine with no rows stays `None`. |
| `a_machine_with_no_cpu_budget_gets_no_score_and_no_host_score_concern` | 4, rule (c) | A budget-0 machine with rows gets `score: None`, `busy_share: None` and every component `None` with a reason. No `host-score:laptop` concern. Today it scores 40, which is critical. |
| `components_combine_unrounded_and_only_the_final_score_is_rounded` | 5 | Components with raw values 49.65, 48.55 and 100, weighted 50/30/20, combine to 59.39, so the score is 59. Rounding first gives 59.7, so 60. The wire components stay rounded (50, 49, 100). |
| `score_fleet_measures_every_machine_component_from_the_window` (**pinned value updated**) | 5 | Host `a` combines unrounded to 55.41, so 55. It was 56 from the rounded parts, because start reliability is a raw 52.63. Without this update the item-5 fix would break a test F may not edit. |
| `the_busy_share_gate_reads_the_rounded_queue_wait_score` | 5, rule (h): **preservation guard, GREEN by design** | A queue-wait raw of 99.6 rounds to 100, so the busy-share gate stays closed and an idle machine's busy share scores 100. It guards the "Decided" constraint that the gate reads the rounded score against item 5's refactor. It passes before the fix on purpose. |

### `crates/gridops-core/src/fleet_health/speed.rs` (SQLite integration, `testdata`)
| Test | Item / rule | What it pins |
|---|---|---|
| `top_keys_hold_only_keys_slower_than_the_fleet` | 10, rule (g) | Keys at 1.2x, 1.0x and 0.8x still give an index, but `top_keys` is only the 1.2x key. |
| `a_job_counts_once_whatever_duplicate_runner_or_placement_rows_it_has` | 11 (A warn 6: both subqueries) | Jupiter's `unit` runners get a second, older `runners` row with the same `github_runner_id` and their own placement. Its `lint` runners get a second, gen-2 placement. Every key must still read 6 machine runs and 24 fleet runs, which is the newest runner row's newest placement, as in #191. |

### `crates/gridops-reconciler/src/fleet_health_rollup.rs` (new `mod tests`, SQLite integration)
| Test | Item / rule | What it pins |
|---|---|---|
| `a_re_run_whose_attempts_fall_in_different_hours_counts_once` | 6, rule (d), A warn 5 | Attempt 1 is rolled in hour A, then attempt 2 is rolled in hour B. Rolling B lists A in `fleet_health_recompute`, and never lists B itself. The next pass re-rolls A, so the run counts once across A and B, and the list ends empty. |
| `the_catch_up_waits_15_minutes_so_a_late_arrival_still_counts` | 7, rule (e) | With the catch-up's marker at the hour that just ended, a pass at hour_end + 14 min 59 s writes no row. A job that completed at 10:59:40 and was stored late is counted by the pass at exactly hour_end + 15 min (`<=`). |
| `a_listed_hour_also_waits_for_the_grace` | A warn 1, rules (a)/(e) | A listed hour that just ended isn't rolled at +5 min and stays listed. It's rolled at +15 min and taken off the list. This gives "has rows ⟺ past grace" for #204. |

### `crates/gridops-reconciler/src/fleet_host_rollup.rs` (existing `mod tests`, SQLite integration)
| Test | Item / rule | What it pins |
|---|---|---|
| `the_catch_up_waits_15_minutes_so_late_samples_still_count` | 9, rule (e) | No machine row exists inside the grace. At hour_end + 15 min the hour is rolled with all 60 minutes, including the last 10 minutes of samples, which arrived after the hour ended. |

### `crates/gridops-reconciler/src/fleet_host_rollup_contract_tests.rs` (#197 contract file, the one in-scope change)
| Test | Item / rule | What it pins |
|---|---|---|
| `a_pass_leaves_the_recompute_list_to_the_job_rollup` (replaces `a_pass_rolls_up_a_listed_hour_and_takes_it_off_the_list`) | 8, rule (f) | Both listed hours, H and the hour in progress, stay listed after a machine pass, and the hour in progress isn't rolled. The old test asserted the behaviour item 8 removes. |

**Deliberately not tested:** the design-doc rules (a)-(i) are prose, which S checks against the doc. Rule (i) documents current behaviour and needs no RED. SQLite discipline (50 ms, 500 rows) is already pinned by the existing week tests, `seven_days_of_*_with_no_long_write_transaction`, and they stay in the GREEN run.

## RED confirmation
Run with `CARGO_BUILD_JOBS=2 RUST_TEST_THREADS=2` and the shared target (`.cargo/config.toml`). `CARGO_TARGET_DIR` was not set.

`cargo test -p gridops-core --features test-support --lib`: **353 passed, 12 failed, 3 ignored.** The 12 failures are exactly the new or updated tests:
```
a_machine_offline_all_window_scores_availability_0_not_no_data        left: Ok(None)      right: Ok(Some(0))
a_machine_with_no_cpu_budget_gets_no_score_and_no_host_score_concern  left: Some(40)      right: None
availability_divides_by_whole_covered_minutes_and_none_covered_is_no_data  "fleet" left: Ok(Some(0)) right: Ok(Some(100))
availability_on_1h_divides_by_the_covered_half_hour_not_the_full_window    "fleet" left: Ok(Some(0)) right: Ok(Some(100))
availability_on_24h_at_ten_past_counts_the_covered_23_hours_and_10_minutes "fleet" left: Ok(Some(83)) right: Ok(Some(100))
busy_share_on_1h_divides_by_the_covered_seconds                       busy share Some(0.25)
components_combine_unrounded_and_only_the_final_score_is_rounded      left: Some(60)      right: Some(59)
infra_failures_divide_by_every_completed_job_so_an_outage_scores_0    left: Ok(None)      right: Ok(Some(0))
score_fleet_measures_every_fleet_component_from_the_window            ("infra-failures", Some(77)) vs Some(80)
score_fleet_measures_every_machine_component_from_the_window          left: Some(56)      right: Some(55)
a_job_counts_once_whatever_duplicate_runner_or_placement_rows_it_has  left: [(e2e,6,24),(lint,12,30),(unit,12,30)] right: [(e2e,6,24),(lint,6,24),(unit,6,24)]
top_keys_hold_only_keys_slower_than_the_fleet                         left: [unit, lint, clippy]  right: [unit]
```
`the_busy_share_gate_reads_the_rounded_queue_wait_score` passes, as a preservation guard should.

`cargo test -p gridops-reconciler`: **146 passed, 5 failed, 3 ignored.** The 5 failures are exactly the new or changed tests:
```
fleet_health_rollup::tests::a_re_run_whose_attempts_fall_in_different_hours_counts_once  "rolling B lists attempt 1's hour, and not B"  left: []  right: [1799989200000]
fleet_health_rollup::tests::the_catch_up_waits_15_minutes_so_a_late_arrival_still_counts "rolled inside the grace"  left: 1  right: 0
fleet_health_rollup::tests::a_listed_hour_also_waits_for_the_grace                       "rolled inside the grace"  left: 1  right: 0
fleet_host_rollup::tests::the_catch_up_waits_15_minutes_so_late_samples_still_count      "rolled inside the grace"  left: 5  right: 0
fleet_host_rollup_contract_tests::a_pass_leaves_the_recompute_list_to_the_job_rollup     "both hours still listed for the job rollup"  left: []  right: [1799992800000, 1800000000000]
```
Every failure is the feature assertion. None comes from a compile, import or seed error: the seeds are checked first, for example the speed test's `"the seed made its duplicates"`, which passes with 12. Every other test in both crates passes, and so do the `gridops-core` integration suites (`--test '*'`, `fleet_health_contract` included).

Hygiene:
- `cargo fmt -p gridops-core -p gridops-reconciler --check` is clean.
- `cargo +1.98.1 clippy -p gridops-core -p gridops-reconciler --all-targets --locked -- -D warnings -A clippy::unused_async_trait_impl` (CI's pinned toolchain) is clean.
- The local stable clippy, 1.99, reports new lints in untouched files (`autoscaling.rs`, `fleet_health_contract.rs`). Those predate this change and CI doesn't run that version.

## Notes for F
- **The grace constant.** Add `pub const ROLLUP_GRACE_MILLIS` in `gridops_core::fleet_health` (A warn 3). The tests spell it as the literal `15 * MINUTE`, so they don't depend on its name.
- **Guards that must stay green, which F keeps green from production data:**
  - `score_contract_tests::on_the_fixture_the_weak_machines_score_lowest_and_its_concerns_come_out` compares the concern keys and severities from `score_fleet(fake::fixture_inputs())` against the fixture JSON.
  - MacBookPro has budget 0 and 0-minute rows. Item 3 alone would give it a critical `host-score`; item 4 is what keeps the fixture right.
  - If the fixture's wording or scores move, change `fake.rs`, `src/features/fleet-health/fixture.json` and the OpenAPI example together (A warn 7). Don't edit the test.
- **Item 6's lookup.** It must run in the read phase, and the insert belongs in the last write batch before the hour's own `DELETE` (A warn 5). The test catches listing H itself (`[a]` exactly) and catches any re-roll that leaves A counted.

## Ready for F
RED is valid. Each of items 1-11, plus A's binding warns 1, 2 and 6, has a test that fails for the right reason before the fix. F may implement against these tests. The handoff's mutation checks for items 1, 2, 3 and 6 are run in Phase 6 (T-green): revert each fix and show its test fails again.
