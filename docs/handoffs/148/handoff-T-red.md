# Handoff-T-red: Phase 4 - GitLab demand matching, scale-down and a pending-jobs poll (#148)

**Date:** 2026-11-20
**Branch:** claude/gitlab-148-scaling-71c514 (head 99c5a07)
**Brief / wireframe reviewed:** docs/handoffs/148-brief.md (all 233 lines, both amendment rounds); wireframe N/A (no UI surface)

## A precondition

Confirmed: A returned **PASS** on the plan in round 3 (`docs/handoffs/148/handoff-A.md`, commit 99c5a07), with four warnings. The brief was not amended after that, so no test pins a warning's suggested fix:

- **Warning 1** (a shared `gridops-core` write helper): it is about where the code lives. The tests drive the poll through `poll_once`, so they pass wherever the guarded write lives.
- **Warning 2** (an identical status leaves `status_changed_at` alone): not adopted. No test asserts `status_changed_at` on a row whose status the poll leaves unchanged. The two invariant tests assert it only where the rank forbids any write.
- **Warning 3** (a failed `group_projects` skips the conclusion step): not adopted, so not pinned. A2-W6's per-project rule is pinned (see below).
- **Warning 4** (pin the interval; the A2-W4 column name): `worker`'s loop and interval aren't unit-testable through `poll_once`. I check them by reading the code in Phase 6. The busy-clear test matches on `runners.gitlab_runner_id`, `runners.gitlab_connection_id = gitlab_jobs.connection_id` and `deleted_at IS NULL`, as A's wording fixes it.

## Tests authored

The tests sit at two tiers:
- **Demand matching** is tested against a real SQLite database through the public `autoscaling` API. That is the cheapest tier where the queue SQL runs.
- **The poll** is tested against a real SQLite database and `FakeGitLab`, through `gitlab_poll::poll_once`. That is the cheapest tier where the API calls, the status rule and the conclusion step all run together.

There are no e2e tests, because there is no UI.

### `crates/gridops-core/src/autoscaling_gitlab_tests.rs` (new, 10 tests)

Mounted from `autoscaling.rs` with `#[cfg(test)] #[path = "autoscaling_gitlab_tests.rs"] mod gitlab_tests;`, which is already in the staged diff. It has its own fixture: installation 1, repository 10, run 20, and connections `conn-a` and `conn-b`.

| Test | What it pins |
|---|---|
| `a_pending_gitlab_job_matches_a_pool_only_when_the_pool_configures_every_tag` | Tag matching: all tags configured → match; tags in another case → match; one tag missing → no match. Done-when 1. |
| `a_gitlab_job_is_counted_only_by_pools_linked_to_its_connection` | Candidates come through `runner_pool_gitlab_connections`. An older pool linked to another connection never counts the job, and `project_id` plays no part (NV-3). |
| `an_untagged_gitlab_job_matches_only_pools_that_run_untagged` | `tag_list = []` matches only `run_untagged = 1`, whatever the pool's labels. An older tagged-only pool with room is skipped. With no such pool, the job has no owner. |
| `gitlab_tags_match_configured_labels_only_never_implied_system_or_catch_all_labels` | A-B4. For `self-hosted`, `self-hosted-linux`, `linux` and the host's arch label, a plain pool and an exclusive pool that don't configure the label never match. A non-exclusive pool that does configure it matches. An exclusive pool matches a job whose tags it configures. |
| `a_pool_with_no_runnable_platform_never_owns_a_gitlab_job` | A2-W5. An older pool with `providers='[]'` (no platform), room and the tag configured is passed over, and the next pool owns the job. |
| `only_pending_gitlab_jobs_are_scaler_demand` | A-B3 and the W-5 Done-when. Eleven rows match the pool's tags: `created`, `waiting_for_resource`, `preparing`, `running`, `manual`, `scheduled`, `unknown`, `success`, `failed`, `canceled` and `skipped`. They give `assigned_queued_jobs = 0` and an empty `queued_job_ownership`. Adding one `pending` row gives 1. |
| `waiting_gitlab_jobs_take_display_room_but_other_unready_statuses_take_none` | A-B3's display tier, read with A-B2 ("a GitLab job still consumes room ahead of a later GitHub job"). An older `created`, `waiting_for_resource` or `preparing` GitLab job takes the room of `shared` (max 1), so GitHub's `waiting` job 100 is shown under `overflow` in `waiting_job_ownership`. With an older `running`, `manual`, `scheduled`, `unknown` or `success` GitLab job, job 100 stays under `shared`. None of these is scaler demand. |
| `github_and_gitlab_jobs_share_one_ordered_queue_and_only_github_jobs_are_listed` | A-W4 and A-B2. An older GitLab job takes `shared`'s room, so the GitHub job overflows; `assigned_queued_jobs` counts both. `queued_job_ownership` and `queued_job_assignments` list the GitHub job only, with `repository_id` 10. When the two jobs are queued at the same time, GitHub goes first. |
| `a_github_job_and_gitlab_jobs_with_the_same_numeric_id_never_collide` | A-B2's key. GitHub job 100 and GitLab job 100 on each of two connections, all with the same `created_at` so they sit next to each other in the candidate list, are **3** jobs for the pool. Ownership lists `(100, both, false)` once, and `queued_job_assignments` lists one job, from repository 10. An `i64` key collapses them to 1. |
| `a_saturated_gitlab_pool_hands_pending_jobs_to_the_next_compatible_gitlab_pool` | The handoff. `older` is full (2 busy of max 2), and the `paused` and `manual` pools never count, so `newer` owns all 3 jobs. When one runner of `older` frees, it takes back the oldest job (1 and 2). |

### `crates/gridops-reconciler/src/gitlab_poll_tests.rs` (new, 16 tests)

Mounted from `main.rs` with `#[cfg(test)] mod gitlab_poll_tests;`, next to `start_tests`. That line is in the staged diff.

`Harness::new` builds the fixture:
- installation 1;
- four connections, each on its own base URL, with the token sealed in `runtime_secrets` under `access_token_key`: `conn-a` (project 11), `conn-b` (project 12), `conn-g` (group 7) and `conn-z` (project 13, which no pool targets);
- a `Reconciler` whose `gitlab` is `Arc::new(FakeGitLab)`.

| Test | What it pins |
|---|---|
| `the_conclusion_step_bounds_are_the_briefed_values` | `GITLAB_JOB_CHECK_LIMIT == 10` and `GITLAB_JOB_CHECK_AGE_MS == 300_000`. |
| `a_pending_job_with_no_webhook_row_is_recorded_once_and_scales_its_pool` | A pending job with no row is recorded with these values: a UUID v4 `id`, `pending`, project 11, pipeline 1, and `tag_list` exactly `["docker","Gpu"]` (the contract encoding, in GitLab's order). Its three timestamps fall within the poll. It becomes demand (`assigned_queued_jobs = 1`). `conn-z` (no pool) is never polled. A second poll keeps one row with the same `id`. |
| `a_group_connection_is_polled_through_its_projects` | One `group_projects` call and one `pending_jobs` call per project. Rows are recorded under `conn-g`, with projects 21 and 22. A job of project 23, which is outside the group, is not recorded. Demand is 2. |
| `a_stale_recorded_job_is_concluded_through_each_job_outcome` | W-4. Two stale rows that aren't pending are checked through `job`. A `Success` answer is written as `JobStatus::Success.as_str()`. `NotFound` concludes the row as **`canceled`**. `status_changed_at` falls within the poll. |
| `a_rate_limited_connection_is_skipped_until_its_retry_after_while_others_are_polled` | `RateLimited { Some(60 s) }` on `conn-a` and `RateLimited { None }` on `conn-g`'s `group_projects`. `poll_once` still returns `Ok`, and `conn-b` is recorded. At `now + 1 s`, neither limited connection is called, but `conn-b` is. At `now + 61 s`, `conn-a` is polled and recorded. The backoff is measured on the `now` argument. |
| `the_conclusion_step_checks_at_most_the_limit_and_only_rows_older_than_the_age` | LIMIT + 2 stale rows and one fresh row, none pending on GitLab. Exactly LIMIT `job` calls are made. The LIMIT oldest rows are concluded; the 2 newest stale rows and the fresh row are not. |
| `stale_pending_rows_are_checked_first_and_running_rows_last` | A2-B1's order. LIMIT stale `running` rows are older than a stale `created` row, which is older than a stale `pending` row. `pending` is checked first, then `created`, then the LIMIT − 2 oldest `running` rows; the 2 newest `running` rows wait. |
| `a_checked_row_is_not_asked_about_again_within_the_age_whatever_the_answer` | One row whose answer is unchanged (`Running`) and one whose `job` call fails both get `updated_at` stamped at the check. A second poll asks about neither. |
| `the_poll_never_moves_a_running_row_back_to_pending` | B-1 and A2-W2/W3. One `running` row has `status_changed_at` after the poll and one before. GitLab lists both as pending. Both keep `running` and their `status_changed_at`. A third job with no row is recorded, as a control that proves the poll ran. Demand is 1. |
| `the_poll_never_resurrects_a_terminal_row` | B-1. A `success` row that GitLab lists as pending keeps its `status` and `status_changed_at`, and produces no demand. A control job is recorded. |
| `a_manual_or_scheduled_row_gitlab_now_lists_as_pending_becomes_pending_demand` | A2 Done-when. `manual` and `scheduled` rows become `pending`, the `manual` row keeps its surrogate `id`, no rows are added, and demand goes from 0 to 2. |
| `concluding_a_running_row_clears_its_runners_busy_flag` | A2-W4. Concluding a `running` row with `gitlab_runner_id = 55` frees only the live runner 55 on `conn-a`. Runner 56, runner 55 on `conn-b` and a deleted runner 55 all stay busy. |
| `rows_of_a_project_whose_pending_jobs_failed_are_not_concluded` | A2-W6. `group_projects` lists two projects, each with a stale row, and one `pending_jobs` call fails (`Upstream`). Exactly one row is concluded, with one `job` call, and the other stays `pending`. `poll_once` returns `Ok`. On the next poll, both rows are concluded. |
| `gitlab_credentials_reads_the_base_url_and_unseals_the_access_token` | A-W2/A2-W7. `gitlab_credentials(&app, id)` returns the parsed `base_url` and the unsealed token. It returns `None` for an unknown connection, and `None` when the secret is gone. |
| `the_placeholder_client_refuses_every_call_by_name` | A-W1/A2-W7. All seven `UnavailableGitLab` methods return `GitLabError::Upstream("GitLab client not available yet")`. |
| `an_idle_gitlab_pool_scales_down_only_once_it_owns_no_gitlab_job` | It calls `maybe_scale_down` on a pool with an idle GitLab runner, `desired_count 2` and `min 0`. While the pool owns a pending GitLab job, `desired_count` stays 2. Once the job is `success`, it drops to 0. This proves the Bitbucket guard needs no edit. |

### Test wiring (not production code)

- `crates/gridops-reconciler/Cargo.toml` `[dev-dependencies]` gains `gridops-core = { path = "../gridops-core", features = ["test-support"] }`. `FakeGitLab` is behind `cfg(any(test, feature = "test-support"))`, and `cfg(test)` of another crate doesn't enable it. `Cargo.lock` is unchanged.

**Not tests I author** (Tier 1 / review in Phase 6): `worker`'s spawn from `main()`, its `reconcileIntervalSeconds` interval, `retry_when_busy` around the writes, the D8 IN-list text in the SQL, and the per-request `GITLAB_REQUEST_TIMEOUT`.

**Coverage hole, by design:** the timeout path can't be driven. `FakeGitLab` has no delay knob, and tokio's `test-util` (paused time) isn't enabled in the workspace. The A2-W6 rule ("don't conclude a project whose `pending_jobs` didn't answer") is pinned through an `Upstream` failure instead, the same code path minus the timer.

## RED confirmation

### Runtime RED: `cargo test -p gridops-core --lib autoscaling`

**10 failed / 24 passed.** All 24 existing autoscaling tests are still green. Each new test fails on its feature assertion, because `queued_job_candidates` reads no `gitlab_jobs`:

    a_pending_gitlab_job_matches_..._every_tag                     left: 0  right: 2
    a_gitlab_job_is_counted_only_by_pools_linked_to_its_connection  left: 0  right: 2
    an_untagged_gitlab_job_matches_only_pools_that_run_untagged     left: 0  right: 1
    gitlab_tags_match_configured_labels_only_...                    left: 0  right: 1   (self-hosted, "configures")
    a_pool_with_no_runnable_platform_never_owns_a_gitlab_job        left: 0  right: 1
    only_pending_gitlab_jobs_are_scaler_demand                      left: 0  right: 1
    waiting_gitlab_jobs_take_display_room_...                       created: left: [(100, "shared")]  right: [(100, "overflow")]
    github_and_gitlab_jobs_share_one_ordered_queue_...              left: 0  right: 1
    a_github_job_and_gitlab_jobs_with_the_same_numeric_id_...       left: 1  right: 3
    a_saturated_gitlab_pool_hands_pending_jobs_...                  left: 0  right: 3

### Compile RED: `cargo test -p gridops-reconciler --no-run`

Each error names a missing #148 item; none is a typo or a wrong import of mine:

    E0432 unresolved import `crate::gitlab_poll`                         (gitlab_poll_tests.rs:11)
    E0560 struct `Reconciler` has no field named `gitlab`                (gitlab_poll_tests.rs:79)
    E0425 cannot find function `gitlab_credentials` in this scope        (x3)
    E0277 `!: anyhow::Context` / E0599 no method `is_none` on `_`        (knock-on effects of the missing `gitlab_credentials`)

Consequence F should expect: until `gitlab_poll`, the `gitlab` field and `gitlab_credentials` exist, the reconciler's test binary doesn't compile, so its existing tests can't run either. That is inherent to RED over new items in Rust, as in #137.

### Runtime RED behind the compile RED (throwaway stub, never staged)

To prove the poll tests type-check and fail for the right reason once they compile, I applied a **no-behaviour stub** for one run. It came from the session scratchpad, and the working tree was then restored from the index: `git status` shows only the five staged paths. The stub contained:
- `gitlab_poll.rs` with the constants, an `UnavailableGitLab`, and a `poll_once` that returns `Ok(())`;
- the `gitlab` field, plus its three constructor sites;
- a `gitlab_credentials` that returns `Ok(None)`.

With it, `cargo test -p gridops-reconciler gitlab_poll_tests` gives **14 failed / 2 passed**. Every failure is the feature missing:

    a_pending_job_..._scales_its_pool        Error: the pending job is recorded
    a_group_connection_is_polled_...         left: 0  right: 1        (GroupProjects calls)
    a_stale_recorded_job_is_concluded_...    left: 0  right: 2        (Job calls)
    a_rate_limited_connection_...            panicked at :354  (conn-b not recorded)
    the_conclusion_step_checks_at_most_...   left: 0  right: 10
    stale_pending_rows_are_checked_first_... left: "pending"  right: "canceled"
    a_checked_row_is_not_asked_about_again_  left: 0  right: 2
    the_poll_never_moves_a_running_row_...   Error: conn-a/813 is not recorded   (the control)
    the_poll_never_resurrects_a_terminal_row Error: conn-a/822 is not recorded   (the control)
    a_manual_or_scheduled_row_...            left: "manual"  right: "pending"
    concluding_a_running_row_clears_...      left: "running"  right: "failed"
    rows_of_a_project_whose_pending_jobs_... left: ["pending","pending"]  right: ["canceled","pending"]
    gitlab_credentials_reads_...             Error: conn-a has credentials
    an_idle_gitlab_pool_scales_down_only_... left: 0  right: 2   (scaled down while it owned a pending GitLab job)

The 2 that pass against the stub are `the_conclusion_step_bounds_are_the_briefed_values` and `the_placeholder_client_refuses_every_call_by_name`. They pin contract items whose RED is their absence (the compile RED above), so the stub, which provides them, passes them.

The two invariant tests (no backwards move, no resurrection) each carry a control job that a no-op poll fails. So they aren't vacuous, even though their invariant half would hold for a poll that does nothing.

### Hygiene

- `cargo fmt --all -- --check` is clean.
- `cargo +1.98.1 clippy -p gridops-core --all-targets -- -D warnings -A clippy::unused_async_trait_impl` (CI's toolchain and flags) is clean.
- The reconciler's clippy, run against the stub, reported only the stub's own dead-code and unused-`async` lints, none in `gitlab_poll_tests.rs`.

## Ready for F

**RED is valid. F may implement against these tests.** Where the brief leaves a choice open, the tests take one reading. F implements to these, or tells T if one contradicts the brief:

1. **`gitlab_poll` API** (`crates/gridops-reconciler/src/gitlab_poll.rs`, `mod gitlab_poll;` in `main.rs`):
   - `pub(crate) async fn poll_once(app: &Reconciler, backoff: &mut HashMap<String, std::time::Instant>, now: std::time::Instant) -> anyhow::Result<()>`. The backoff key's type is inferred, so any `HashMap<K, std::time::Instant>` compiles.
   - `poll_once` returns `Ok` when one connection is rate-limited or fails.
   - The skip-until is measured on `now`, not on `Instant::now()`.
   - `RateLimited { retry_after: None }` must still skip the connection one second later. The documented default is F's choice, as long as it is longer than 1 s.
2. **Constants:** `pub(crate) const GITLAB_JOB_CHECK_LIMIT: i64 = 10;` and `pub(crate) const GITLAB_JOB_CHECK_AGE_MS: i64 = 300_000;`, both `i64` like `JOB_CHECK_LIMIT`/`JOB_CHECK_AGE_MS`.
3. **`UnavailableGitLab`** is a `pub(crate)` **unit struct** in `gitlab_poll.rs`, constructed as `UnavailableGitLab`.
4. **`gitlab_credentials(app: &Reconciler, connection_id: &str) -> Result<Option<GitLabCredentials>>`** lives in `main.rs` (the tests call it through `use super::*`).
5. **`Reconciler.gitlab: Arc<dyn GitLabApi>`.** My harness builds a `Reconciler` literal with every current field plus `gitlab`. F adds the field to `main()` and to the two other existing literals (`main.rs` `reconciler_with_github`, `start_tests.rs`), as A-W1 allows.
6. **`NotFound` concludes as `canceled`.**
7. **Status written on conclusion** is `JobStatus::as_str()` of GitLab's answer. A non-terminal answer that ranks higher (for example `Running` for a `pending` row) may be written as well; no test forbids it.
8. **Group routing:** rows are stored with the group connection's `connection_id` and the project's `project_id`.
9. **Busy clear** matches `runners.gitlab_runner_id = gitlab_jobs.gitlab_runner_id AND runners.gitlab_connection_id = gitlab_jobs.connection_id AND runners.deleted_at IS NULL`.
10. **Display tier and A-B2:** in `waiting_job_pools`, a `created`, `waiting_for_resource` or `preparing` GitLab job uses up room before it is filtered out. The tests expect this.
11. **Do not edit the Bitbucket guard in `maybe_scale_down`.** The scale-down test passes without that edit once `assigned_queued_jobs` counts GitLab jobs.
12. **Evidence appendix:** in Phase 6 I'll add entries for the unchanged-code facts these tests rely on: the `maybe_scale_down` guard, and `FakeGitLab`'s pending filter, its `NotFound` and its per-base-URL state.
