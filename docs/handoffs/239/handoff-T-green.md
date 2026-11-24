# Handoff-T-green: Phase 7 - Fix scoring, rollup and speed-index defects from the wave-1 review (#239)

**Date:** 2026-11-24
**Branch:** claude/fh-239 (head a8e55f3, F's implementation; code base 73de941)
**Issue:** #239
**Handoff-F reviewed:** `docs/handoffs/239/handoff-F.md`
**Handoff-T-red:** `docs/handoffs/239/handoff-T-red.md`

## GREEN confirmation
All runs used `CARGO_BUILD_JOBS=2 RUST_TEST_THREADS=2` and the shared target. Host load average was 11-20.
```
cargo test -p gridops-core --features test-support --lib
test result: ok. 365 passed; 0 failed; 3 ignored        (T-red: 353 passed, 12 failed)

cargo test -p gridops-reconciler
test result: ok. 151 passed; 0 failed; 3 ignored        (T-red: 146 passed, 5 failed)
```
No test needed repair: F's "Tests that look wrong" is "None", and I found none either. F's two reported timing flakes (the week write-transaction checks) did not occur in my runs.

### Mutation checks (brief Done-when: items 1, 2, 3 and 6)
Each fix was reverted in the working tree alone, the named suite re-run, and the file restored with `git checkout`. The tree is clean at a8e55f3 again, and the suites were re-run green afterwards.

| Item | Mutation | Tests that failed |
|---|---|---|
| 1 | `covered_millis` returns `window.millis()` (full-window denominators) | `availability_on_1h_divides_by_the_covered_half_hour_not_the_full_window`, `availability_on_24h_at_ten_past_counts_the_covered_23_hours_and_10_minutes`, `availability_divides_by_whole_covered_minutes_and_none_covered_is_no_data`, `busy_share_on_1h_divides_by_the_covered_seconds` (4 failed) |
| 2 | infra share divides by `completed_count` (successes) again | `infra_failures_divide_by_every_completed_job_so_an_outage_scores_0`, `score_fleet_measures_every_fleet_component_from_the_window` (2 failed) |
| 3 | machine availability `None` when `online_minutes == 0` again | `a_machine_offline_all_window_scores_availability_0_not_no_data` (1 failed) |
| 6 | the earlier-attempt hours are no longer inserted into `fleet_health_recompute` | `a_re_run_whose_attempts_fall_in_different_hours_counts_once` (1 failed) |

Every item's test fails when its behaviour is removed, so the tests pin behaviour, not the diff.

## Tier 1 results
| Check | Command | Result |
|---|---|---|
| Core integration suites | `cargo test -p gridops-core --features test-support --test '*'` (includes `fleet_health_contract`) | every suite ok, PASS |
| API health | `cargo test -p gridops-api health` | 18, 1 and 2 passed, PASS |
| Format | `cargo fmt -p gridops-core -p gridops-reconciler --check` | clean, PASS |
| Lint | `cargo +1.98.1 clippy -p gridops-core -p gridops-reconciler --all-targets --locked -- -D warnings -A clippy::unused_async_trait_impl` | Finished, no warnings, PASS |
| Build / server start | covered by the test builds; no server or route changed, so no curl smoke applies | PASS |

F's reported results match mine: no discrepancy.

## Tier 2 results
- **Coverage per criterion:** each of items 1-11 and A's warns 1, 2 and 6 has a test that failed before the fix (T-red) and passes now. PASS.
- **Test quality:** each test names one behaviour and sits at the cheapest tier: unit for the pure scorer, SQLite integration only where SQL or pass timing is the behaviour. The one preservation guard (`the_busy_share_gate_reads_the_rounded_queue_wait_score`) is green by design and is not redundant. No redundant test found. PASS.
- **Type safety:** no new `unwrap` or `any`-style casts outside tests; clippy clean. The `as u8` roundings are the existing pattern. PASS.
- **Error handling / edge cases:** the 0-covered-minute case, budget-0 machines, an hour with no rows, and `earlier` overflowing the 500-row batch (empty batch of its own) are handled. PASS.
- **Data integrity:** `INSERT … ON CONFLICT(hour) DO NOTHING` on the primary key; the listing and the hour's own `DELETE` share the last transaction; the read runs before the write transactions (indexed per F's EXPLAIN). PASS.
- **API contract:** the wire format (`ScoreComponent`, public signatures) is unchanged; the only new public item is `ROLLUP_GRACE_MILLIS`. The fixture-guard test stays green. PASS.
- **Security / migrations:** no input surface or schema change (`0050` untouched). N/A.
- **Design doc:** rules (a)-(i) are present in `docs/design/fleet-health.md`; the earlier prose that said both rollups read the queue is updated (lines 119-128, 185-190). S verifies the wording. PASS.
- **Playwright:** no UI surface, none run. N/A.
- **Evidence appendix:** F's `evidence.md` covers the unchanged-source facts the tests rely on; I added the `first_hour` fact (positive covered time). PASS.

## Acceptance criteria status
1. Each item has a test failing before the fix: PASS (T-red RED log, 12 + 5 failures).
2. Mutation check for items 1, 2, 3, 6: PASS (table above).
3. Design doc states rules (a)-(i): PASS by inspection (S to confirm).
4. SQLite discipline: PASS. The week write-transaction tests (`seven_days_of_*_with_no_long_write_transaction`, the 100 ms speed check) are in the green runs.

## Blocking issues
None.

## Advisory notes
- The two week write-transaction timing tests flaked for F under load average 22+; they passed for me. They are existing tests, not this diff's; only a CI flake would justify revisiting them.
- No UI surface: U is N/A.
