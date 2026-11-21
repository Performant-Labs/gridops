# Handoff-T-green: Phase 7 - GitLab demand matching, scale-down and a pending-jobs poll (#148)

**Date:** 2026-11-20
**Branch:** claude/gitlab-148-scaling-71c514 (head 8fea0f5)
**Issue:** #148
**Handoff-F reviewed:** docs/handoffs/148/handoff-F.md
**Handoff-T-red:** docs/handoffs/148/handoff-T-red.md

## GREEN confirmation

| Command | Result |
|---|---|
| `cargo test -p gridops-core --lib -- gitlab::jobs_tests` | 9 passed, 0 failed |
| `cargo test -p gridops-core --lib -- autoscaling gitlab` | 53 passed, 0 failed |
| `cargo test -p gridops-core --lib` | 281 passed, 0 failed |
| `cargo test -p gridops-reconciler` | 84 passed, 0 failed (includes 16 `gitlab_poll_tests`) |

No test was changed in this phase. F flagged no wrong tests, and none failed.

**Spot-check that the tests pin behavior (two mutations, each reverted with `git checkout`, tree clean after):**
- `jobs.rs` `status_write_applies` changed to `incoming_tier >= stored_tier` (drops the strictly-newer rule): 2 of 9 D18 tests fail (the 363-case tier matrix, and the runner-id test).
- `gitlab_poll.rs:322` `set_gitlab_runner_busy(.., false, ..)` changed to `true`: `concluding_a_running_row_clears_its_runners_busy_flag_and_records_one_runner_event` fails.

## Tier 1 results

| Check | Result |
|---|---|
| `cargo fmt --all -- --check` | exit 0, PASS |
| `cargo +1.98.1 clippy --workspace --all-targets --locked -- -D warnings -A clippy::unused_async_trait_impl` | exit 0, PASS |
| Build / compile (all targets via clippy) | PASS |
| Existing tests (gridops-core lib 281, reconciler 84, gridops-api: 143+32+60 passed) | PASS, except below |
| `gridops-api` `fleet_http::release_preflight::the_release_preflight_fails_early_without_a_macos_arm64_machine` | FAIL, host-environment: fails at `release_preflight.rs:234` exactly as F reported against base `b22c451`. This host is macOS ARM64, so the expected FAIL never happens. Not touched by #148. |
| `gridops-manager` bin | 94 passed, 11 failed, all "path must be shorter than SUN_LEN". Same count and error F reported on base. Not touched by #148. |
| Server start / API smoke | N/A: no endpoint or route changed. The poll is a background worker, and its path is covered by `poll_once` tests on SQLite with `FakeGitLab`. |

These 12 failures are pre-existing host-environment failures, not caused by this change. I did not re-run base myself; F's base run and my identical failure signatures agree.

## Tier 2 results

- **Test coverage per criterion:** PASS (table below).
- **Test quality (§7):** PASS. Each test names a behavior. D18 is tested at unit tier against a real migrated SQLite database (the rules are SQL), demand through the public `autoscaling` API, and the poll through `poll_once` with the fake client. Mutation checks show failure for the right reason. No redundant tests found. The 363-case matrix is one test, so the suite is proportionate to the change.
- **Type safety:** PASS. No `any`-style escapes. The only integer casts are checked (`i64::try_from`, `u64::try_from`) and return `sqlx::Error::Encode`.
- **Error handling:** PASS. `RateLimited`, `NotFound`, a failed `pending_jobs`, a failed `group_projects` and an unknown status all have tests. Retry exhaustion in `record_gitlab_job` returns `sqlx::Error::Protocol`. The 30 s timeout path is not driven (known hole, declared in T-red, covered by reading the code).
- **Data integrity:** PASS. D18's guarded upsert holds every rule in one statement, and the `WHERE` also requires the status and `status_changed_at` the caller read, so a stale outcome is never reported. A terminal row never changes. Runner-id fill is `COALESCE`. The busy flag is matched by `base_url`, so it never crosses instances.
- **Read-through check:** The poll's only direct SQL on `gitlab_jobs` is the one-column `UPDATE gitlab_jobs SET updated_at=? WHERE id=?` check mark (`gitlab_poll.rs:269`, with the D8 status guard). All status writes go through `record_gitlab_job`, and the busy clear through `set_gitlab_runner_busy`, as D18 requires. I confirmed this with `grep`: there is no other `UPDATE`/`INSERT` on `gitlab_jobs`.
- **API contract:** PASS. `jobs.rs` public items match the brief's D18 signatures. The `autoscaling` public signatures are unchanged, and `assigned_queued_jobs` counts both platforms as A-B2 requires.
- **Security:** PASS. Credentials are unsealed through `runtime_secret` and held as `secrecy` values. All SQL is parameterised, with no string-built values.
- **Migration safety:** N/A. No schema change and no new dependency.
- **Playwright:** N/A (no UI).
- **Evidence appendix:** `docs/handoffs/148/evidence.md` already has 10 F entries covering the unchanged-code facts the tests rely on (`gitlab_jobs` columns and identity index, `runner_events` columns, `retry_when_busy`, `pool_platforms!()`, the fake's pending filter and others). I added none, because the tests rely on nothing else outside the diff.
- **Cross-check of F's results:** My results match F's, including the 12 host failures.

## Acceptance criteria status

| Criterion | Status | Backing test(s) |
|---|---|---|
| GitLab demand matches by tags, connection linkage, untagged/`run_untagged`, configured labels only, platform gate | PASS | `autoscaling_gitlab_tests` (11 tests) |
| Display tiers and D8 statuses, ordering of GitHub and GitLab in one list, saturated-pool handoff | PASS | `autoscaling_gitlab_tests` |
| D18 dedupe across a project and its group connection | PASS | `one_gitlab_job_recorded_under_a_project_and_its_group_connection_is_one_job` |
| D18 writer rules (tiers, terminal immutability, runner and tag fill, unknown, base_url busy match) | PASS | `gitlab::jobs_tests` (9 tests) |
| Poll records jobs, polls groups, concludes through `job`, backs off on rate limit, bounded and ordered checks, no resurrection | PASS | `gitlab_poll_tests` (16 tests) |
| Conclusion frees the runner and writes exactly one `runner_events` row | PASS | `concluding_a_running_row_clears_its_runners_busy_flag_and_records_one_runner_event` |
| Failed project's rows not concluded | PASS | `rows_of_a_project_whose_pending_jobs_failed_are_not_concluded` |
| Credentials and placeholder client | PASS | `gitlab_poll_tests` |
| Scale-down | PASS | `maybe_scale_down` poll test |
| Existing GitHub autoscaling unchanged | PASS | the 24 existing autoscaling tests |

## Blocking issues

None.

## Advisory notes

- **Duplicate `runner_events` row** is possible when one running job is recorded under both a project and its group connection (A's round-4 warning 5, part 4; F's "Known issues"). The runner's state is right and only an extra info row appears. The fix belongs to #140's D18 semantics.
- **Production no-op until the real client is wired:** `main()` wires the `UnavailableGitLab` placeholder, so the poll logs and skips every connection in production until #139 or a follow-up wires #146's client. That is the brief's stated plan.
- **Late conclusion exposure** for a non-ephemeral runner (F's "Known issues"): the same exposure as #140's webhook; ephemeral runners are not affected.
- **Linux CI** has not run the 12 host-only failures.
- **No UI surface:** U is N/A.
