# Independent review of PR #244 (#240), for #245

Reviewed: master at `529cefe` (PR #244, merged 2026-11-24 09:30 PST) against the text of #240 and its brief `docs/handoffs/240-brief.md`. The reviewer did not write #244. Line numbers are at `529cefe`.

## Summary

All nine items are done. Every fix in #244 has a test that fails when that fix alone is reverted (21 reverts, table below), with one gap PR #244 itself half-noted: the item 4 call-site fix (`host_live_reads` calling `live_counts` instead of `counts`) was caught only by a wall-clock check, which #244 reported as passing without the fix. Its four query-plan tests read `live_counts_sql()` directly, so they pass with the call site reverted.

Two defects found, both fixed in this PR:

1. **The live path still read the whole job history** (item 4's class). `collect_live` calls `fleet_queue_breakdown`, whose queued-jobs clause (`fleet_queued_jobs!`, `crates/gridops-api/src/resources.rs` :92-99) filtered `wj.status IN ('queued','waiting','pending','requested')`. SQLite cannot prove that implies the partial index's `status<>'completed'`, so it planned `SCAN wj`: every `workflow_jobs` row ever recorded, on every health computation. On the synthetic week that was 355,489 of `collect_live`'s 357,807 SQLite VM steps, and 280,000 more with 40,000 more finished jobs. Fix: add `wj.status<>'completed'` (no change to the result) so it reads `workflow_jobs_open_created_idx`. `collect_live` drops to 4,999 VM steps, flat with history.
2. **No deterministic test of item 4.** The speed test now also counts SQLite VM steps (a progress handler on a one-connection pool) for `host_live_reads` and `collect_live` before and after adding io's 40,000 finished placements, and fails if either grows over 5 %. Unlike the 100 ms limit, the count depends only on data and plans, so it fails the same way on a fast or a loaded machine.

Also added: a test that a miss on one window does not wait on another window's computation (item 1 for misses; before it, a single compute lock shared by all windows was only caught indirectly by item 3's live-call count), and a test that `live_counts` gives exactly `counts`' figures except the runner count (#244 claims this; nothing tested it).

## The nine items

| # | Item | Verdict | Where |
|---|---|---|---|
| 1 | A lock per window; a hit never waits on a computation | Done | `crates/gridops-api/src/fleet/health.rs` :118-132 (`WindowCache`: a sync `entry` lock held only to read or write, a per-window `computing` lock), :306-311 (the hit is answered before any computation lock) |
| 2 | 2 s timeout as a failure; 30 s before retrying; stale or 503 meanwhile | Done | `health.rs` :316-321 (`tokio::time::timeout(COMPUTE_TIMEOUT, …)`), :90-115 (`answer`, `after_failure`), :336-337 (the failure time is read after the failure) |
| 3 | Live part read once, shared across windows | Done (per the brief: once per computation, shared by computations in flight, not cached across requests) | `health.rs` :134-194 (`LiveFlight`), :354-357 |
| 4 | Live counts over non-terminal placements through `(host_id,state)` | Done; partly done for the class until this PR (see defect 1) | `crates/gridops-core/src/fleet/registry/queries.rs` :1325-1340 (`live_counts_sql`), :2449 (call site), :1213-1229 (allocation read lists unreleased states) |
| 5 | Disk alarm from the observed disk, only with a heartbeat in 120 s | Done | `crates/gridops-api/src/resources.rs` :5955-5988 (`fleet_root_disks`, shared with the Overview's `disk_alerts`), `health_live.rs` :49-54, :193-196 |
| 6 | Busy GitLab runners count as running | Done | `health_live.rs` :98-106 (`r.current_job_id IS NOT NULL OR (r.status='busy' AND r.busy=1)`; the job row joined only for GitHub runners) |
| 7 | 422 above 200 rules or 1 KiB per pattern; same check in the UI | Done | `crates/gridops-api/src/fleet/job_categories.rs` :145 and :185; `src/features/settings/job-categories-section.tsx` :51-61 (limits), :179-180 and :279-280 (Add rule and Save disabled) |
| 8 | `job_categories.updated` audit with before and after, same transaction | Done | `job_categories.rs` :263-291 (`DELETE … RETURNING` for "before", `audit_in` on the save's transaction); `crates/gridops-api/src/auth.rs` :457 |
| 9 | Compile message only for a 422, on its row; `AbortController` | Done | `job-categories-section.tsx` :113-150 |

Done-when: the OpenAPI spec is updated for the new 422 details. Done: `docs/openapi/gridops-v1.json` (changed by #244) states the 200-rule and 1 KiB limits on the request body (`maxItems` 200, pattern `maxLength` 1024), the 422 for an empty or oversized pattern, the 422 with empty `details` for a list over 200 rules, the `job_categories.updated` audit event, and the 2 s timeout and 30 s retry rule on the health endpoint's 503.

Notes, not defects:

- Item 1: a request for a window whose response has expired, while that window's own computation is running, waits for it (at most the 2 s timeout). That is the existing "one computation shared by the concurrent misses" rule, not a hit.
- Item 3: a computation that joins a live reading already in flight gets the live part read at the first computation's `now`, at most 2 s earlier.
- Item 6: Bitbucket runners still never count as running. Not a defect of #244, and no follow-up filed: GridOps records no busy signal for a Bitbucket runner at all ("Bitbucket does not publish a per-step busy signal to GridOps", `crates/gridops-reconciler/src/main.rs` :1579), so there is nothing for the live read to count. Counting them needs a new data source, which is a product decision, not a fix.
- Item 7: the 25 MB body limit still applies before `parse_rules` runs; the limits bound the compile and the write, which is what #240 asked.
- The Overview's own queue read (`visible_queued_jobs!`, `resources.rs` :80-88) walks every run of the user's repositories in the covering index `workflow_runs_repository_status_idx (repository_id=?)`. Adding the same `status<>'completed'` term does not change its plan. Not a #244 defect (it is the Overview's read, untouched by #240), and no follow-up filed: on a copy of the synthetic week (90,400 runs, one user seeing the repository) its `COUNT(*)` takes about 4 ms (SQLite CLI, three runs: 5.6, 4.7, 4.0 ms), reading index entries only. It grows linearly with runs, so it is worth a look when #204 times the Mars copy, not before.

## Revert tests

Method: with this PR's tests in place, each fix alone was reverted (a scripted text replacement, `git checkout` between runs), and `live_plan_tests`, `fleet::health*` and `fleet::job_categories::tests` were run. The UI fixes were reverted the same way and `tests/fleet-health-240-job-categories.test.ts` run. Every revert fails at least one test, each for the stated reason.

| Revert | Fails |
|---|---|
| H1a: one lock taken before the hit check (the old shape) | `a_cache_hit_does_not_wait_on_another_windows_computation`, `a_miss_does_not_wait_on_another_windows_computation` (new), `concurrent_misses_on_every_window_share_one_live_reading` |
| H1b: one computation lock for every window, hits unlocked | `a_miss_does_not_wait_on_another_windows_computation` (new); `concurrent_misses_on_every_window_share_one_live_reading` also fails, but only because serialised windows read the live part three times. The hit test passes. |
| H2a: no timeout | `a_hanging_source_times_out_and_is_not_called_again_for_30_seconds`, `the_30_seconds_count_from_the_failure_not_the_start` |
| H2b: failure not remembered | `a_failing_source_is_not_called_again_for_30_seconds`, `a_hanging_source_times_out_…`, `the_30_seconds_count_from_the_failure_not_the_start` |
| H2c: failure time = computation start | `the_30_seconds_count_from_the_failure_not_the_start` |
| H3: live part read per window | `concurrent_misses_on_every_window_share_one_live_reading` |
| L4a: `host_live_reads` calls `counts` again | `the_live_path_passes_the_speed_check_however_long_the_placement_history` (strengthened): "host_live_reads: 3330223 VM steps on the week, 5970223 with io's 52600 placements" (4,396 with the fix). All four `live_plan_tests` pass. #244's original version of the test, run three times here at load average ~13, failed on the 100 ms limit (134-373 ms); #244 reported it passing, so as a detector it depended on the machine. |
| L4b: allocation read back to `state!='released'` | `the_root_allocation_read_seeks_the_unreleased_states` |
| L4c: queued-jobs clause without `status<>'completed'` (this PR's fix) | the strengthened speed test: "collect_live: 357807 VM steps on the week, 637807 with io's 52600 placements". #244's original test passes with it. |
| Q4a: `live_counts` without its allocation branch | `the_live_counts_read_only_live_placements`, `the_live_counts_equal_the_full_counts_but_for_the_runners` (new) |
| Q4b: `retryable` dropped from the unfinished states | `the_spelled_out_states_are_the_schemas_less_the_finished_ones` |
| L5a: disk alarm from the admin budget and the latest measurement | `the_disk_alarm_uses_the_observed_disk_not_the_admin_budget`, `a_machine_with_an_old_heartbeat_raises_no_disk_alarm` |
| L5b: heartbeat condition dropped | `a_machine_with_an_old_heartbeat_raises_no_disk_alarm` |
| L6a: `current_job_id IS NOT NULL` only | `a_busy_gitlab_runner_counts_as_running_once_and_an_online_one_does_not` |
| L6b: job row joined for any platform | the same test (its third step: the GitLab runner took the GitHub job's category) |
| J7a: no rule-count limit | `the_list_and_each_pattern_are_bounded`, `over_the_limits_the_routes_answer_422_and_save_nothing`, `a_save_is_audited_with_the_lists_before_and_after` (its 201-rule save answered 200) |
| J7b: no pattern-length limit | `the_list_and_each_pattern_are_bounded`, `over_the_limits_the_routes_answer_422_and_save_nothing` |
| J8a: no audit event | `a_save_is_audited_with_the_lists_before_and_after`, `a_save_whose_audit_event_fails_saves_nothing` |
| J8b: audit written after the commit | `a_save_whose_audit_event_fails_saves_nothing` |
| UI 9a: any failure treated as a 422 | "does not call a server error or a network failure a compile failure" |
| UI 9b: 422 messages not put on their rows | "shows a blank new row's 422 on that row…", "shows a 422's compile error on the rule it names" |
| UI 9c: no `signal` passed | "aborts the preview in flight when the rules change" |
| UI 9d: no `abort()` on a newer edit | the same test |
| UI 7a: no pattern-size check | "refuses a pattern over 1 KiB on its row and sends nothing" |
| UI 7b: no list-length check | "refuses more than 200 rules and adds no 201st" |
| UI 7c: Add rule not disabled at 200 | the same test |
| UI 7d: pattern size in characters, not UTF-8 bytes | "refuses a pattern over 1 KiB…" |

## The contract test edit (#197's #194 test)

`crates/gridops-api/src/fleet/health_contract_tests.rs` :185-188 (#245's text names `tests/fleet_http/health_contract_tests.rs`; the file is under `src/fleet/`). #244 added `harness.clock.advance(30 * SECOND);` before the "source recovers" request and reworded the comment. Nothing else in the file changed.

Finding: the edit is necessary and is the smallest one item 2 allows.

- Necessary: the step before fails the source at t = 631 s, which item 2 records. A request at the same instant is inside the 30 s and must not call the source, so it is a 503. Reverting the edit (C2) fails the test with 503 for 200.
- Smallest: the retry test is `now - failed_at < 30 s`, so 30 s is the least advance that lets the source be called. With 29 s (C1) the test fails the same way.
- Intent kept: "once the source recovers, fresh data comes back" still holds, on the first allowed try. No assertion was removed or loosened.

But the process was not followed: PR-Agent marked the edit BLOCKING and asked for the owner's sign-off on #240 before merge. #244 dismissed it in a PR comment and merged with the finding still `ACTIVE`. The guard did not stop it because #244 closes no wave-1 issue (#240 has no entry in `docs/design/fleet-health-owners.json`), not because the rule allows it. The sign-off is the owner's; this conclusion is posted on #240 for acceptance or rejection.

## Shared test data (`crates/gridops-core/src/fleet_health/testdata.rs`)

#244 made `seed_fleet` seed each machine's observed root disk (credential, observation session, `fleet_observed_domains` root row at `DISK_BYTES`) and `insert_samples` raise `last_heartbeat_at` to the newest sample's `received_at`.

- Who reads it: the module is behind `gridops-core`'s `test-support` feature, enabled only by `gridops-core`, `gridops-api` and `gridops-reconciler` (their `Cargo.toml` dev-dependencies). Of the production code their testdata suites exercise (the rollups, backfill, speed index, classifier cache, job categories, health live), only `collect_live` and the Overview's `disk_alerts` read `fleet_observed_domains` or `last_heartbeat_at`.
- No expected value moved: #244 changed no assertion outside its own tests except the contract line above.
- No test weakened: the disk contract test (`health_live_contract_tests.rs` :229) asserts the same thresholds on the same 100 GiB, now read from the observed disk at `DISK_BYTES` instead of the budget at `DISK_BYTES`. The fixture now matches production: `heartbeat` (`crates/gridops-core/src/fleet/registry/inventory.rs` :250) inserts the samples (:315) and sets `last_heartbeat_at` (:354) in one transaction.
- Run: with `testdata.rs` put back to its pre-#244 text, `cargo test --no-fail-fast -p gridops-core -p gridops-api -p gridops-reconciler -- fleet health speed classifier category` ran 26 test binaries: 391 passed, and exactly three failed, all disk tests that need the observed disk the change seeds: `health_live::tests::the_disk_alarm_uses_the_observed_disk_not_the_admin_budget`, `health_live::tests::a_machine_with_an_old_heartbeat_raises_no_disk_alarm` (#240's) and `health_live_contract_tests::the_root_disk_alarm_follows_disk_alarm_for_at_each_threshold` (#197's, under item 5's rule). Every other suite passes with either version of the fixture, so none of them depends on it. With #244's version all pass (the workspace run below).

## Speed check on the synthetic week

Measured with `live_path_timings_on_the_synthetic_week` (added here, `#[ignore]`d: a measurement, not a check), release build like #234's numbers, on Jupiter on 2026-11-24 around 12:10 PST. `build_week`: five machines, 50,400 jobs, a sample every 30 s for 7 days. 21 runs each after one warm-up. Jupiter was heavily loaded (load average 30 to 49, other sessions building), so the times are noisy; the VM-step counts below are exact.

| Read | Before #240 (`counts`, full queued-jobs scan) | Master as merged (#244) | This PR |
|---|---|---|---|
| `host_live_reads` | median 118.9 ms, p95 143.4 ms | median 0.49 ms, p95 0.79 ms | median 0.30 ms, p95 0.35 ms |
| `fleet_queue_breakdown` | median 8.8 ms, p95 11.9 ms | median 5.9 ms, p95 6.7 ms | median 0.04 ms, p95 0.05 ms |
| `collect_live` (the whole live half) | median 135.1 ms, p95 150.3 ms | median 6.2 ms, p95 7.7 ms | median 0.47 ms, p95 0.52 ms |
| SQLite VM steps, `host_live_reads` (debug, exact) | 3,330,223 | 4,396 | 4,396 |
| SQLite VM steps, `collect_live` (debug, exact) | not measured | 357,807 | 4,999 |

Before #240, #234 measured `host_live_reads` at about 98-124 ms and `collect_live` at 108-113 ms on the same data; the 118.9 ms here agrees.

The whole endpoint: `GET /api/v1/fleet/health` still serves the fake source (#204 plugs in the database), so its time is the HTTP, admin check and cache overhead alone: cold (a miss every request, the clock moved 31 s) median 0.39 ms, p95 0.72 ms; cached median 0.35 ms, p95 2.5 ms. With #204's source the cold time will be that plus the windowed read plus `collect_live`. #204's targets (p95 under 300 ms cold, 20 ms cached) cannot be measured until then; the live half leaves almost all of the 300 ms to the windowed read.

The speed check itself (`speed_check::check`, no statement over 100 ms, no write lock over 50 ms) passes for `host_live_reads` and `collect_live` on the week plus io's 40,000 extra finished placements, in the debug test build.

## PR-Agent findings dismissed on #244, and "ACTIVE after dismissal"

- **Shared fixture changed** (22c6936e809b, NIT). Reason given: the fixture is test-only, the three crates' suites pass, and it matches production. True by reading the code (previous section).
- **Contract test edited** (6e6c0b88f5b7, BLOCKING). Reason given: the edit is required by item 2, and the guard does not check #240. The first part is true (C1, C2 above). The second is true but is not a permission: the finding asked for the owner's sign-off before merge, and that did not happen.
- **ACTIVE after dismissal.** A reply on the PR does not change a finding's state. In the latest `pr-agent-review-state:v1` both findings are still `ACTIVE` (`last_seen_head_sha` 048bf39), while the three fixed ones are `RESOLVED`. The `pr-agent/review` status was `success` ("Reviewed — score 80"), so nothing mechanical stopped the merge. #244 merged with two ACTIVE findings, one of them BLOCKING, against the rule "merge only when CI is green and no finding is active".

## Checks run for this review

On Jupiter, 2026-11-24 (Pacific), with `CARGO_BUILD_JOBS=2` and `RUST_TEST_THREADS=2`: `cargo fmt --all --check`; `cargo clippy --workspace --all-targets --locked -- -D warnings -A clippy::unused_async_trait_impl` (clean); `cargo test --no-fail-fast -p gridops-core -p gridops-api -p gridops-reconciler --locked` (1,053 passed, 0 failed, 2 ignored, one of them the timing measurement); `npm run lint` (clean); `npm test` (707 passed). No API change, so no OpenAPI change.
