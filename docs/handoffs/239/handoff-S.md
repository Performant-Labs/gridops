# Handoff-S: Phase 8 - Fix scoring, rollup and speed-index defects from the wave-1 review (#239)

**Date:** 2026-11-24
**Branch:** claude/fh-239 (head 4981976; code base 73de941)
**Issue:** #239
**Brief:** `docs/handoffs/239-brief.md` (amended, blob f4a2e56)
**Handoff-T reviewed:** `docs/handoffs/239/handoff-T-red.md`, `docs/handoffs/239/handoff-T-green.md`
**Handoff-A reviewed:** `docs/handoffs/239/handoff-A.md` (plan review, round 2), `docs/handoffs/239/handoff-A-dup.md` (anti-duplication gate)
**Handoff-F reviewed:** `docs/handoffs/239/handoff-F.md`, with `evidence.md` and `decisions.md`
**Operator-facing report:** N/A (no visual surface)

## A precondition
Confirmed. The plan review (round 2) and the anti-duplication gate both returned **PASS**. Round 1's BLOCK (0b435f8) was resolved by the brief amendment before round 2.

## T precondition
Confirmed. `handoff-T-green.md` reports "Blocking issues: None". T-red proved 12 + 5 failing tests before the fix, and T-green re-ran the suites and the mutation checks.

## Visual-diff-tool precondition
N/A. The diff touches only Rust crates (`gridops-core`, `gridops-reconciler`) and `docs/design/fleet-health.md`. No route, component or rendered output changed, so the browser and pixel-diff preconditions do not apply and no HTML report is produced.

## Spec sanity check (run first)
The brief's own contract is internally consistent, and the design-doc rules it asks for do not contradict one another or the code's existing conventions:
- The grace constant, the covered-time denominator and the hour-by-time rule fit together. A's round-2 warns 1-4 closed the two gaps that remained (the listed-hours loop and the whole-minute unit).
- No schema or convention defect that would justify ADVISORY-HOLD. F's implementation is judged against the brief, not against a defective source of truth.

## Independent verification
Run by S in the worktree (tree clean, head 4981976), with `CARGO_BUILD_JOBS=2 RUST_TEST_THREADS=2` and the shared target; `CARGO_TARGET_DIR` unset. Host load average 14-17.
```
cargo test -p gridops-core --features test-support --lib
test result: ok. 365 passed; 0 failed; 3 ignored

cargo test -p gridops-reconciler
test result: ok. 151 passed; 0 failed; 3 ignored
```
These match F's and T's counts. I did not repeat T's mutation runs (they edit production files); I checked each against the code instead, see "Mutation checks" below.

## Spec compliance

### Items 1-11 (checked against the diff, not the handoffs)
| Item | Requirement | Where | Compliant |
|---|---|---|---|
| 1 | Every time denominator is the covered time `now − ceil_hour(now − W)`; test with an off-the-hour `now`; grace-hour test at H + 10 min on 24h | `score.rs`: `first_hour`, `covered_millis`, `covered_minutes`; used by `fleet_online_share`, the machine availability, busy share and `capacity_misfit`. Tests: 1h at 10:30; 24h at H + 10 min; whole minutes at H + 5:29 and H + 0:29; busy share | YES |
| 2 | Infra share ÷ `job_count`; update the pinned test | `score_fleet` uses `total_jobs`; pinned value 77 to 80, deduction "1.0%". Outage test scores 0 and raises the critical concern | YES |
| 3 | Only no rows is missing data; 0 online minutes scores 0 | `online = (!rows.is_empty() && covered_minutes > 0)`. Test covers both a machine with 0-minute rows (scores 0) and one with no rows (`None`) | YES |
| 4 | Budget 0 gives no score and no `host-score` concern | `unbudgeted` over every component when `cpu_budget_millis == 0`; `concerns.rs` unchanged and still reads `host.score?`. Fixture guard `on_the_fixture_the_weak_machines_score_lowest_and_its_concerns_come_out` stays green with no fixture edit | YES |
| 5 | Combine unrounded; private; wire and signatures unchanged | Private `Measured`, `measure_*`, `combine_measured`. `combine(&[ScoreComponent])` and every public component function keep their signatures; `combine` feeds the rounded scores through, as before. Busy-share gate still reads `queue.component.score` (guard test green) | YES |
| 6 | An earlier attempt's hours are listed in `fleet_health_recompute` in the same write transaction | `EARLIER_ATTEMPT_HOURS` read in the read phase (indexed per F's EXPLAIN); `INSERT … ON CONFLICT(hour) DO NOTHING` in the last write batch, before the hour's own `DELETE`; the hour itself is never listed; bounded to 7 days; the extra empty batch keeps every transaction at 500 rows or fewer | YES |
| 7 | Catch-up rolls only once `hour_end + 15 min <= now` | `next + HOUR_MILLIS + ROLLUP_GRACE_MILLIS <= now`; also the listed loop (A warn 1): `hour <= now − HOUR − GRACE` | YES |
| 8 | Machine rollup stops reading and deleting the queue; still skips hours not ended | Listed loop and the `DELETE` removed; `a_pass_leaves_the_recompute_list_to_the_job_rollup` replaces the one contract test the brief put in scope | YES |
| 9 | Same 15-minute grace for the machine catch-up | `next < floor_hour(now − ROLLUP_GRACE_MILLIS)`, which equals `next + 1 h + grace <= now` for a whole-hour `next` (checked: `next + H` is a whole hour, so `floor_hour(x) >= next + H` iff `x >= next + H`) | YES |
| 10 | `topKeys` filter `ratio > 1.0` | `qualifying.retain(ratio > 1.0)` before sort and truncate; the index still uses every qualifying key | YES |
| 11 | Newest-row subqueries | Both #191 subqueries (runner and placement) copied inline into `grouped_durations`; the test seeds both duplicate shapes | YES |

### Done-when
| Criterion | Result |
|---|---|
| Each item has a test that fails before the fix | YES. T-red lists 12 + 5 failures, each on the feature assertion, with the seeds checked first. The one green-by-design test is a preservation guard for rule (h), and it is justified |
| Mutation check shown for items 1, 2, 3, 6 | YES. `handoff-T-green.md` has the table: item 1 (4 tests fail), item 2 (2), item 3 (1), item 6 (1) |
| Design doc states rules (a)-(i) | YES, see below |
| SQLite discipline (50 ms, 500 rows) | YES. Batches stay at 500 rows or fewer, with the listings in an extra batch when the last has no room; the week write-transaction tests and the 100 ms speed check are in the green runs |

### Design-doc rules (the oracle for #204), read in `docs/design/fleet-health.md`
| Rule | Where stated | OK |
|---|---|---|
| (a) covered-time denominators and the grace-hour contract | "The covered time (#239)" and "The grace-hour contract with #204" under Scores: whole covered minutes for availability, the 0-minute case (`None`, not 0), covered seconds for busy share, capacity fit unchanged; hourly tables read only past grace, every other hour live, by time, not by the rollups' markers; the past-grace-but-unwritten case is stated | YES |
| (b) infra ÷ `job_count` | The fleet component table row | YES |
| (c) only no rows is missing; 0 minutes scores 0; budget 0 has no score and no `host-score` concern | Paragraph after the machine table | YES |
| (d) the job rollup lists an earlier attempt's hours | Under `fleet_health_hourly`, with the interim double count A warn 4 asked for | YES |
| (e) 15-minute grace before any hour is rolled | "The grace (#239)" under the rollups | YES |
| (f) only the job rollup reads `fleet_health_recompute` | The rollups paragraph and the `fleet_health_recompute` section | YES |
| (g) `topKeys` only above 1.0 | Speed index, step 6 | YES |
| (h) the gate and the bands read the rounded component score | "Rounding (#239)" | YES |
| (i) the fleet median includes the machine's own runs | Speed index, step 3 | YES |

The older prose that said both rollups read the queue, or that rounding happens "only at the end", is corrected (the rollups paragraph, the `fleet_health_recompute` section and the opening of Scores). The doc's numbers agree with the code and the test pins: for example, 1h at H + 59 s is `None`, and on 24h at H + 10 min the covered time is 1390 minutes.

### Constraints
- `migrations/0050_fleet_health.sql` is untouched. No migration was added.
- No new module, table, settings key, scoring entry point or speed query (A-dup confirms). The one new public object is `ROLLUP_GRACE_MILLIS` in `gridops_core::fleet_health`, which A's plan review required so that `gridops-api` (#204) can import it.
- Scope: the files changed are the six production files the plan named, the one in-scope contract test, the design doc, and `docs/handoffs/239/*`.
- Work was headless.

## Quality audit
| Area | Result | Notes |
|------|--------|-------|
| API consistency | N/A | No endpoint changed. `ScoreComponent`, the wire format and every public function signature are unchanged |
| Error handling | PASS | The 0-covered-minute case is `None` rather than NaN (A warn 2), budget-0 and no-row machines are handled, and an `earlier` list that overflows the batch gets its own batch |
| UI/UX match to spec | N/A | No UI |
| Accessibility | N/A | No UI |
| Architecture gate | PASS | A: PASS (plan, round 2) and PASS (anti-duplication). No quality finding conflicts with either |
| Code organization | PASS | One rule per component in `score.rs`; each rollup extends its own `run_pass`/`write_*`; the grace has one definition. See advisory notes for cosmetic nits |
| Security | N/A | No input surface, auth or secret handling touched; the new SQL binds every value |
| Performance | PASS | The new reads use `workflow_jobs_completed_idx`, `workflow_jobs_run_status_idx`, `runners_github_id_idx` and `workload_placements_runner_idx` (F's EXPLAIN); the speed check stays under 100 ms |
| Visual regression | N/A | No rendered output |
| Naming consistency | PASS | `covered_*`, `measure_*`, `ROLLUP_GRACE_MILLIS`, `earlier` agree with the brief's and the doc's terms |
| Test quality (`testing/test-quality.md` section 7) | PASS | See below |

### Test quality, per the section 7 rubric
Tests in the diff: 11 in `score.rs` (8 new, 2 pinned values updated, 1 preservation guard), 2 in `speed.rs`, 3 in `fleet_health_rollup.rs`, 1 in `fleet_host_rollup.rs`, 1 contract test replaced.
- **Names one behaviour and fails for the right reason.** Every new test names one item. T-red's output shows each failing on its feature assertion (`left`/`right` values in the handoff), and T-green's mutations show items 1, 2, 3 and 6 each go red when the fix is removed. The seeds are asserted first where they matter (`"the seed made its duplicates"`).
- **Tier-correct and non-duplicative.** The pure scorer is tested with hand-built rows (unit); SQLite tests are used only where the behaviour is SQL or pass timing (items 6, 7, 8, 9, 10, 11). No test re-proves another's branch: the four covered-time tests pin different things (half-hour on 1h, the 23h10 contract, the whole-minute unit and the 0-minute case, and busy-share seconds).
- **Behavioural, not structural.** Assertions are on scores, counts, listed hours and row contents, not on helper names. The only literal that mirrors production is the grace, spelled `15 * MINUTE` on purpose so that the tests are an independent expectation of `ROLLUP_GRACE_MILLIS`.
- **Proportionate.** 11 items plus A's three binding warns (listed-hour grace, whole-minute unit, both newest-row subqueries) produce 18 new or updated tests, about one per behaviour. Rule (i) documents unchanged behaviour and correctly gets no test; the design-doc rules are prose and are checked here.
- **Smells.** No assertion-free, tautological or unreachable test. The one test that passes before the fix, `the_busy_share_gate_reads_the_rounded_queue_wait_score`, is a deliberate guard for the brief's "decided" constraint that item 5's refactor must not change which score the gate reads. It is not a duplicate signal.
- **No "delete or merge" findings.**

### Mutation checks (read against the code)
- Item 1: `covered_millis` returning the full window reverts every denominator at once; the 1h, 24h, whole-minute and busy-share tests each hinge on it.
- Item 2: dividing by `completed_count` returns "not enough data" in the outage test and 77 in the pinned test.
- Item 3: the `online_minutes == 0` guard returns `None` in the offline-machine test.
- Item 6: dropping the listing leaves `listed()` empty where the test requires `[a]`, and the next pass would leave attempt 1 counted.

## Scope check
Delivered exactly the brief's items 1-11 and the design-doc rules (a)-(i). There was no over-delivery. F's choices where the brief was silent are within A's binding warns: `ROLLUP_GRACE_MILLIS` placement (warn 3), the whole-minute unit (warn 2), the listed-hour grace (warn 1), the read-phase lookup and last-batch insert (warn 5), and both newest-row subqueries (warn 6). Item 4's reason wording is kept unchanged on the fixture's components, so `fake.rs`, `fixture.json` and the OpenAPI example needed no edit. That is the smaller change.

## Verdict

PASS — all acceptance criteria met, spec-compliant, quality acceptable. Ready for O to commit.

## Advisory notes
None of these blocks the PR.
1. **Mislabelled comment, `score.rs`.** In `availability_on_24h_at_ten_past_counts_the_covered_23_hours_and_10_minutes`, the comment "the hour that just ended, as the source computes it live" sits on the row at `T` with 10 minutes. At `now = T + 10 min`, that row is the hour in progress. The hour that just ended is `T − 1 h`, which is one of the 23 full rows. The assertion and the arithmetic are right (23 ended hours plus 10 minutes is 1390), so only the comment needs a fix.
2. **Blank lines.** A new test's doc comment follows the previous test's closing brace with no blank line in `speed.rs` (before `top_keys_hold_only_keys_slower_than_the_fleet`), in `score.rs` (before the `#239's scoring fixes` comment) and in `fleet_host_rollup.rs` (before the item 9 test). `rustfmt` does not enforce this.
3. **Stale module doc owned by #190.** `fleet_health_backfill.rs:4` still says the backfill lists its hours "for the rollups". After item 8, only the job rollup reads the list. The file belongs to #190 (open PR #237), so correcting it here would cross files; whichever lands second can fix the wording.
4. **Long line, design doc.** The `fleet_health_recompute` paragraph runs one line past the doc's usual width where `fleet_health_snapshots` is mentioned. It is cosmetic.
5. **For O, the PR description.** Put no closing keyword before a wave-1 issue number (the brief's constraint). #239 is not a wave-1 issue, so the Fleet health guard passes a PR that closes only #239.
6. **For O, #240 and the design doc.** #240 also edits `docs/design/fleet-health.md`. Whichever of #239 and #240 lands second rebases it.
7. **For O, #204's issue body.** It should match the contract the design doc now states: hourly rows only for hours past grace, everything else computed live, by time, with `ROLLUP_GRACE_MILLIS`. This is A's note from the plan review.
8. **For O, a possible follow-up.** On 1h, busy share is still noisy in the first minutes of each hour, because jobs count at completion over a span of seconds. Whether time shares need a minimum covered span, as the 20-job minimum does for counts, is a spec decision that this issue does not make (A's plan-review note).
9. **Load flakes.** F saw two existing timing tests (the week write-transaction check and the synthetic-week seed check) fail once each under load average 22+. T and S saw neither (load 11-20 and 14-17). They are not this diff's tests or seeds. Only a CI flake would justify revisiting them.
