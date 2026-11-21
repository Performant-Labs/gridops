# Handoff-S: Phase 8 - GitLab demand matching, scale-down and a pending-jobs poll (#148)

**Date:** 2026-11-20
**Branch:** claude/gitlab-148-scaling-71c514 (audited at head 6fa0732, on master b22c451)
**Issue:** #148
**Brief:** docs/handoffs/148-brief.md (all three amendment rounds)
**Handoff-T reviewed:** docs/handoffs/148/handoff-T-green.md (and handoff-T-red.md, round 2)
**Handoff-A reviewed:** docs/handoffs/148/handoff-A.md (round 4, PASS) and handoff-A-dup.md (PASS)
**Handoff-F reviewed:** docs/handoffs/148/handoff-F.md
**Operator-facing report:** N/A (no visual surface)

## A precondition
Confirmed: A returned PASS on the plan (round 4, 6 warnings, no block) and PASS on the anti-duplication gate (`handoff-A-dup.md`).

## T precondition
Confirmed: T-green reported zero blocking issues. Its only non-passing items are the 12 host-environment failures, which I re-checked (see Quality audit, Performance/Existing tests).

## Visual-diff-tool precondition
N/A. This is a Rust backend change (a worker, SQL, a shared writer). It has no rendered surface, so there is no Tier 3 visual audit and no HTML report. The browser and visual-diff preconditions do not apply.

## Preview / spec sanity check
The brief is the source of truth. I found no convention violation in it. Its three amendment rounds are internally consistent, and A-W3 / A2-W2/W3 are explicitly superseded by round 3 (D18), so nothing contradicts itself. No ADVISORY-HOLD.

## Visual diff results
N/A (non-visual).

## Spec compliance

Checked against the brief's Scope, A-B1 to A-W4, A2-B1 to A2-W7, round 3 (D18) and Done-when. Evidence is the diff `b22c451..HEAD`, my own re-runs, and a query-plan check.

| Requirement | Compliant | How I checked / delta |
|---|---|---|
| Active `gitlab_jobs` feed `queued_job_candidates` beside `workflow_jobs`, one ordered list (A-W4) | YES | `autoscaling.rs`: `gitlab_job_candidates` merged with `github_job_candidates`, stable sort by `(queued_at, QueuedJobKey)`; `Github` declared first so GitHub sorts before GitLab. Pool order tail is identical to the GitHub arm (`scope='repository'` first, `created_at`, `id`). |
| Candidate pools = pools linked through `runner_pool_gitlab_connections` | YES | The `EXISTS` over `link.connection_id IN json_each(job.connections)`. Test: `a_gitlab_job_is_counted_only_by_pools_linked_to_its_connection` (project id plays no part). |
| Tag match: every job tag among the pool's configured `labels`, case-insensitive (A-B4) | YES | `lower()` both sides, no `pool_platforms!()` label terms. Test: `a_pending_gitlab_job_matches_a_pool_only_when_the_pool_configures_every_tag`. |
| Untagged job matches only `run_untagged=1` pools | YES | `CASE WHEN json_array_length(tag_list)=0 THEN candidate.run_untagged=1`. Test: `an_untagged_gitlab_job_matches_only_pools_that_run_untagged` (both directions). |
| Exclusive pools respected (A-B4); `self-hosted`, `self-hosted-linux`, `linux`, arch tags never implied | YES | `gitlab_tags_match_configured_labels_only_never_implied_system_or_catch_all_labels`: four labels, exclusive and non-exclusive pool, plus an exclusive pool that does match its own configured tag. |
| Platform gate kept (A2-W5) | YES | Bare `EXISTS (SELECT 1 FROM (pool_platforms!()) platform)`. Test: `a_pool_with_no_runnable_platform_never_owns_a_gitlab_job`. |
| Status tiers (A-B3): only `pending` is demand; `created`/`waiting_for_resource`/`preparing` display only; `running`, `manual`, `scheduled`, `unknown`, terminal never demand | YES | SQL tier filter `job.tier=1 OR (? AND job.tier=0)`. Tests: `only_pending_gitlab_jobs_are_scaler_demand` (11 statuses), `waiting_gitlab_jobs_take_display_room_but_other_unready_statuses_take_none`. |
| D8: the demand scan reads only the active statuses and uses `gitlab_jobs_active_created_idx` | YES | The CTE has the D8 `IN` list verbatim. I ran `EXPLAIN QUERY PLAN` on a migrated database with 20,000 mostly-terminal rows and no table statistics (the codebase never runs `ANALYZE`): `SCAN gj USING INDEX gitlab_jobs_active_created_idx`. (With a synthetic `ANALYZE` SQLite prefers the per-connection unique index; production has no statistics, so that case does not apply.) |
| Job identity (A-B2): GitLab ids never collide with GitHub ids or across instances | YES | Private `QueuedJobKey::{Github(i64), Gitlab{base_url, job_id}}`. Test: `a_github_job_and_gitlab_jobs_with_the_same_numeric_id_never_collide` (count 3). Note the key is `base_url`, not `connection_id`: A's round-4 warning 1, which the D18 dedupe needs. |
| Public GitHub-keyed outputs stay GitHub-only; only `assigned_queued_jobs` counts GitLab jobs; GitLab jobs still take room | YES | `KeyedJobOwner::github()` filters. No public signature changed. Test: `github_and_gitlab_jobs_share_one_ordered_queue_and_only_github_jobs_are_listed`. |
| Saturated-pool handoff to the next compatible GitLab pool; paused/manual rules | YES | `a_saturated_gitlab_pool_hands_pending_jobs_to_the_next_compatible_gitlab_pool` (also covers the job returning when a runner frees). |
| D18 cross-connection dedupe on `(base_url, job_id)`, counted once | YES | Dedupe in the arm's CTE. Test: `one_gitlab_job_recorded_under_a_project_and_its_group_connection_is_one_job` (both shapes). |
| Scale-down of an idle GitLab pool, none while it owns a GitLab job; Bitbucket guard untouched | YES | `maybe_scale_down` is not in the diff. Test: `an_idle_gitlab_pool_scales_down_only_once_it_owns_no_gitlab_job`. |
| Poll is its own worker, never in `reconcile()` (A-B1); interval from `reconcileIntervalSeconds` | YES | `tokio::spawn(gitlab_poll::worker(..))` next to `github_sync_worker`; `setting_i64(.., "reconcileIntervalSeconds", 30).clamp(5, 3_600)`, the pass's own read. `poll_once(app, &mut backoff, now)` is the test seam. |
| Group connection polled through `group_projects`, then `pending_jobs` per project; job recorded under the group's `connection_id` with the project's id | YES | Test: `a_group_connection_is_polled_through_its_projects`. |
| Records an unseen pending job (UUID v4 `id`, contract `tag_list`, `JobStatus::as_str()`) and scales the pool; no double-record | YES | Test: `a_pending_job_with_no_webhook_row_is_recorded_once_and_scales_its_pool`. |
| Status invariants: older status never over a newer webhook status; no resurrection of a terminal row | YES | Tests: `the_poll_never_moves_a_running_row_back_to_pending` (later and earlier `status_changed_at`), `the_poll_never_resurrects_a_terminal_row` (no candidate, no demand). All writes go through D18. |
| Conclusion step only through `GitLabApi::job` (terminal status as given, `NotFound` -> `canceled`) | YES | Test: `a_stale_recorded_job_is_concluded_through_each_job_outcome`. `grep` confirms no other write to `gitlab_jobs` than D18 and the one-column check mark. |
| Bounded conclusion step (A2-B1): at most 10, only rows older than 5 min, pending first then waiting then running, check mark before the request | YES | Tests: `the_conclusion_step_checks_at_most_the_limit_and_only_rows_older_than_the_age`, `stale_pending_rows_are_checked_first_and_running_rows_last`, `a_checked_row_is_not_asked_about_again_within_the_age_whatever_the_answer`. |
| Rows of a project whose `pending_jobs` failed are not concluded (A2-W6) | YES | Test: `rows_of_a_project_whose_pending_jobs_failed_are_not_concluded`. Per-request `tokio::time::timeout(GITLAB_REQUEST_TIMEOUT)`, no per-connection budget. |
| Concluding a `running` row frees its runner (A2-W4) and writes one `runner_events` row | YES | Test: `concluding_a_running_row_clears_its_runners_busy_flag_and_records_one_runner_event` (other instance, other id, deleted runner untouched; no event without a runner id). T-green mutated the `busy` flag and the test failed. |
| `RateLimited` backs off per connection with `retry_after` or a documented default, without blocking others; other errors logged and do not fail the pass | YES | `HashMap<String, Instant>` in the worker's state, default 60 s, capped at 1 h. Test: `a_rate_limited_connection_is_skipped_until_its_retry_after_while_others_are_polled`. |
| A `manual`/`scheduled` row that GitLab now lists as pending becomes `pending` under the same `(connection_id, job_id)` | YES | Test: `a_manual_or_scheduled_row_gitlab_now_lists_as_pending_becomes_pending_demand`. |
| D18 shared writer: exact API, rules, unit tests, `pub mod jobs;` | YES | `gitlab/jobs.rs` public items match the brief's signatures. 9 tests in `jobs_tests.rs` cover each Done-when item. One implementation deviation is under Advisory notes (note 2). |
| `Reconciler.gitlab: Arc<dyn GitLabApi>`, `UnavailableGitLab` placeholder, `gitlab_credentials` next to `runtime_secret` (A-W1, A-W2, A2-W7) | YES | `main.rs` edits are exactly the allowed ones. `gitlab_credentials` unseals through `runtime_secret` and keeps the token as a `SecretString`. Test: `gitlab_credentials_reads_the_base_url_and_unseals_the_access_token`. |
| Do not touch `gitlab_lifecycle.rs`, `0047_gitlab.sql`, or existing contract types | YES | None is in `git diff --name-only`. `gitlab/mod.rs` gained only `pub mod jobs;` and the test mount. |
| `cargo fmt --check`, `clippy -D warnings`, `cargo test` for the workspace | YES, with the pre-existing host failures | See Quality audit. |

## Quality audit

| Area | Result | Notes |
|------|--------|-------|
| API consistency | N/A | No HTTP route changed. Public `autoscaling` signatures are unchanged; `jobs.rs` public items match the D18 signatures in the brief. |
| Error handling | PASS | `RateLimited`, `NotFound`, a failed `pending_jobs`, a failed `group_projects`, an unknown status, an invalid scope id or base URL each have a defined result. Errors are logged per connection and never fail `poll_once`. `record_gitlab_job` returns `sqlx::Error::Protocol` after 5 lost races rather than spinning. The 30 s timeout path is not driven by any test (declared by T-red); the code is a plain `tokio::time::timeout` mapped to `Upstream`. |
| UI/UX match to spec | N/A | No UI. |
| Accessibility | N/A | No UI. |
| Architecture gate | PASS | A: PASS (round 4) and A-dup: PASS. I found nothing in the diff that conflicts with either. |
| Code organization | PASS | New code sits where the analogues are: SQL in a domain module (`gitlab::jobs`, like `autoscaling.rs`), the worker beside `github_sync_worker`. No dead code, no TODO, no `unwrap`/`dbg!`/`println!`/`#[allow]` in the added lines (grep). |
| Security | PASS | All SQL is bound (the JSON id lists are bound parameters). The token is a `SecretString`, unsealed through `runtime_secret`; no token reaches a log line or a `runner_events` row. |
| Performance | PASS | Demand scan uses the partial index (plan check above). The GitLab arm costs nothing when no GitLab job exists (empty CTE, and the sort is skipped when the GitLab list is empty). Conclusion step is bounded at 10 requests per connection per poll. Poll writes are one transaction per project. |
| Visual regression | N/A | No UI. |
| Naming consistency | PASS | `gitlab_poll`, `GITLAB_JOB_CHECK_AGE_MS`/`_LIMIT`, `GITLAB_REQUEST_TIMEOUT`, `GITLAB_RATE_LIMIT_BACKOFF`, `UnavailableGitLab` all match the brief's names. |
| Test quality (`testing/test-quality.md` §7) | PASS | See below. |
| Existing tests / hygiene | PASS, with 12 pre-existing host failures | I re-ran `cargo test -p gridops-core --lib -- autoscaling gitlab` (53 pass), `cargo test -p gridops-reconciler` (84 pass), `cargo fmt --all -- --check` (exit 0), `cargo +1.98.1 clippy --workspace --all-targets --locked -- -D warnings -A clippy::unused_async_trait_impl` (exit 0). I also re-ran `gridops-api` `fleet_http release_preflight`: it fails because this macOS ARM64 host satisfies the very condition the test needs to be absent. That file is untouched by #148, so the failure is the host's, as F and T state. The 11 `gridops-manager` failures are the macOS unix-socket path limit (`SUN_LEN`) under this worktree's long path; also untouched code. |

### Test-quality audit (§7), 36 new tests

- **Per test.** Each of the 36 tests names one behaviour, and each runs at the cheapest tier that can prove it: D18 at unit tier on a migrated SQLite database (the rules are SQL), demand through the public `autoscaling` API, the poll through `poll_once` with `FakeGitLab`. Failure is for the right reason: T-green's two mutations went red in the expected tests (the tier rule and the busy flag), and T-red showed a no-behaviour stub failing 20 of 24 and 14 of 16. Assertions are on effects (rows, runner state, demand counts, `runner_events`). The only call-count assertions are `FakeMethod::Job` / `PendingJobs` counts for "not asked again" and "skipped while rate-limited", where the call is the only observable effect (§2 allows this).
- **Per suite.** 36 tests for a new module, a new SQL arm and a new worker is proportionate. There is no fan-out of one branch across many tests.
- **Smells.** No assertion-free, tautological or snapshot tests. Two borderline items, neither a delete:
  - `a_status_write_applies_only_upward_or_at_the_same_tier_when_newer_and_never_to_a_terminal_row` runs 363 cases in one test. §3 warns against fanning one branch, but each stored status is a separate arm of the SQL `CASE`, so the cases are not one repeated branch. It is a single test, so the count stays proportionate.
  - `the_conclusion_step_bounds_are_the_briefed_values` pins the constants to 10 and 5 minutes. It is a spec pin (the brief states both values), and the behavioural test uses the constants symbolically, so it would not catch a change to either. Keep it.
- **One coverage gap, not blocking:** see Advisory note 1.

## Scope check

F delivered the briefed scope and stayed inside the file list. The diff is exactly: `autoscaling.rs`, the D18 copy (`gitlab/jobs.rs`) with `pub mod jobs;`, the new `gitlab_poll.rs`, the A-W1 `main.rs` edits, one constructor line in `start_tests.rs`, and tests. Two additions are outside the brief's literal file list and are reasonable:

- the `gridops-core` dev-dependency with `test-support` in `gridops-reconciler/Cargo.toml` (needed for `FakeGitLab`; resolver 3, so dev-dependency features do not reach normal builds; `Cargo.lock` is unchanged);
- the `#[cfg(test)] mod jobs_tests;` line in `gitlab/mod.rs` (test wiring, chosen so the D18 tests keep running against whichever copy of `jobs.rs` lands).

F also went slightly past the brief in two places, both adopting A's round-4 warnings: the highest-tier status rule for a duplicated job, and skipping the conclusion step when `group_projects` fails. Neither changes a pinned behaviour. Nothing was under-delivered.

## Verdict

PASS - all Done-when items are met and covered by passing tests, the implementation matches the brief and all three amendment rounds, A and T gates hold, and the quality audit found no blocking issue. Ready for O to commit.

## Advisory notes

None of these blocks the commit. They are listed so each is decided, not left open.

1. **The highest-tier rule is untested (decided: not blocking).** `gitlab_job_candidates` makes a duplicated job's status the furthest along of its rows, takes `MIN(created_at)` as its queue time, and takes tags from any row that has some. The brief does not require any of this (F says so), and no Done-when covers it. I checked it: in a scratch copy of HEAD I changed `MAX(CASE gj.status ...)` to `MIN(...)`, and all 11 `autoscaling::gitlab_tests` still passed. So a regression would let a job whose runner is already `running` under one connection count as demand through its still-`pending` duplicate, starting a runner it does not need, until the poll or the webhook catches up (a bounded window, the one A described in round-4 warning 2). I did not make it a REWORK because it is beyond the brief and bounded. Recommended follow-up, one test in `autoscaling_gitlab_tests.rs`: seed the same job under `conn-a` (`running`) and `conn-a-group` (`pending`), same base URL, and assert demand 0; then `pending` + `created` and assert demand 1. Whoever merges #140 second runs the union of tests, so it fits there.
2. **D18 is not "one SQL statement per call".** The brief's D18 rules say one statement. `record_gitlab_job` reads the stored row first (SQLite's `RETURNING` cannot give the previous status), then runs the one guarded upsert, and re-reads up to 5 times if another writer got in between. The upsert holds every D18 rule on its own, and the signature and outcome contract are as briefed. F documents this as design decision 1 but not under "Deviations from spec". When #140's copy replaces this one, it must still report `previous_status` correctly, because the poll's busy-clear depends on `previous_status == Some("running")`.
3. **Deferred transactions.** The poll opens its transactions with `begin()` (deferred) and D18 reads before it writes, so a concurrent commit can raise `SQLITE_BUSY_SNAPSHOT`. `retry_when_busy` retries on any SQLITE_BUSY extended code (`code & 0xff == 5`) and re-runs the whole closure, so it is handled. The fleet modules use `BEGIN IMMEDIATE`; switching would avoid the retry. Not needed now.
4. **The shared notes are stale.** `/Users/andreangelantoni/ClaudeAangel100/side-session-notes/local_e1c0b548-f83a-4d0b-8ba3-069dc9dfc38b/148-notes.md` still says `gitlab_credentials` lives in `gitlab_poll.rs` (A2-W7 moved it to `main.rs`, so #139 would import the wrong path) and describes the old "ON CONFLICT DO NOTHING then guarded update" write. It does not mention that the D18 copy at `gitlab/jobs.rs` will probably land before #140's, the `(base_url, job_id)` dedupe key, or the highest-tier rule. It is outside the repo and I did not edit it. O should refresh it before #139 and #140 rebase.
5. **A persistently failing project is never concluded.** Per A2-W6, rows of a project whose `pending_jobs` call failed are left alone. If a project is deleted or the token loses access, `pending_jobs` fails on every poll, so that project's recorded `pending` rows are never concluded by the poll and keep counting as demand until a webhook ends them. This follows the brief as written. A follow-up could treat a persistent `NotFound` for a project as an answer.
6. **Known and accepted (from F and T):** the production poll logs and skips every connection until #139 or a follow-up wires #146's client in place of `UnavailableGitLab` (a warning per connection per interval until then); a second `runner_events` row is possible when one running job is recorded under both a project and its group connection (A round-4 warning 5, part 4, a D18 semantic that #140 owns); a late conclusion can free a non-ephemeral runner that has already taken another job, the same exposure as #140's webhook.
7. **Second merge.** Whoever merges second takes master's `gitlab/jobs.rs` and keeps `jobs_tests.rs` (mounted from `mod.rs`) and the `autoscaling_gitlab_tests.rs` dedupe test, running both against it. `gitlab/mod.rs` and `gridops-reconciler/Cargo.toml` are shared files where add/add conflicts are possible.
8. **Linux CI** has not run the 12 host-only failures. They are specific to this macOS host, so a green Linux run is expected.
