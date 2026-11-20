# [GitLab: demand matching, scale-down and a pending-jobs poll](https://github.com/Performant-Labs/gridops/issues/148)

Part of epic #136. **Wave 1.** Depends only on #137 (merged as `b22c451`: `GitLabApi`, `FakeGitLab`, migration `0047_gitlab.sql`, `gitlab_jobs`). It reads `gitlab_jobs`, which tests seed directly, so it doesn't wait for the webhook in #140.

## Need

Pending GitLab jobs must scale the right pool up, and idle GitLab pools must scale down, as GitHub pools do from `workflow_job`. A missed webhook must not lose a job.

## Contract deviations that replace the issue text (epic #136, "Contract deviations shipped by #137")

Where these differ from the issue text, these win. The contract lives on master; do **not** change shared contract types (the `gitlab` module, `0047_gitlab.sql`, domain types). If the contract truly needs a change, stop and report it.

- **D2**: every `GitLabApi` method takes `&GitLabCredentials { base_url, access_token }` first. A single `Arc<dyn GitLabApi>` serves every connection.
- **D4**: `group_projects(credentials, group_id)` exists so a group connection can be polled: list its projects, then call `pending_jobs` on each.
- **D8 / D13**: the active-jobs partial index is `gitlab_jobs_active_created_idx` `WHERE status IN ('created','waiting_for_resource','preparing','pending','running')` (`GITLAB_ACTIVE_JOB_STATUSES`, `JobStatus::is_active()`). The demand scan must read only those statuses so it uses the index. `manual` and `scheduled` jobs are unconcluded but are not demand. A job is concluded exactly when `JobStatus::is_terminal()`.
- **D12**: `wait_timeout_seconds` default `GITLAB_DEFAULT_WAIT_TIMEOUT_SECS = 600` (registration idle timeout; owned by #139/#147, not here).
- **D14**: GitLab starts belong to the reconciler (#139's module).
- **D16**: the migration is `0047_gitlab.sql`.
- `gitlab_jobs.id` is a UUID v4 string made by whoever records the job first (#140 or this poll). `tag_list` is `serde_json::to_string(&Vec<String>)`, read with `serde_json::from_str::<Vec<String>>`. Identity is `(connection_id, job_id)` (unique index).
- `docs/handoffs/137-brief.md` holds the full contract rationale.

## Scope

**Demand and matching** (`crates/gridops-core/src/autoscaling.rs`, the GitLab branch):
- Active `gitlab_jobs` rows (status in the D8 set — waiting jobs are `created`, `waiting_for_resource`, `preparing`, `pending`; decide and document whether `running` counts as demand the way GitHub's `in_progress` does not) feed `queued_job_candidates` alongside `workflow_jobs`, so `assigned_queued_jobs`, `queued_job_ownership`, `waiting_job_pools`/`waiting_job_ownership`, `queued_job_assignments` and repository/provider capacities see them.
- Candidate pools for a GitLab job are the pools linked to its connection through `runner_pool_gitlab_connections`.
- A job matches a pool when every job tag (case-insensitive) is among the pool's labels. An untagged job (`tag_list` = `[]`) matches only pools with `runner_pools.run_untagged = 1`.
- Exclusive pools (#134, migration 0046 `runner_pool_exclusive_labels`, already on master) must be respected: GitLab tags are matched against the pool's configured labels only — the GitHub catch-all/system labels (`self-hosted`, `self-hosted-linux`, OS/arch) are a GitHub convention; decide and document whether GitLab tags may match them, and if so an exclusive pool never matches the catch-all labels.
- Ownership and the saturated-pool handoff work exactly as they do for GitHub (same ordering, room, standing, paused/manual rules).
- **Job identity hazard**: candidates and owners are keyed by an `i64` job id today (`workflow_jobs.id`). GitLab job ids are a different id space (and are only unique per connection) and must never collide with a GitHub job id or with each other across connections. Choose a key that cannot collide without changing shared contract types; keep the existing public API stable for its callers in `gridops-api` and the reconciler, or update every caller in the same change.
- GitLab pools aren't covered by the Bitbucket "never scale down" guard in `maybe_scale_down` (`crates/gridops-reconciler/src/main.rs`): that guard keys on `runner.ci_platform == "bitbucket"`, so a GitLab pool must scale down when idle with no owned GitLab jobs. Prove it with a test; do not edit that guard unless a test shows it is needed.

**Pending-jobs poll** (`crates/gridops-reconciler/src/gitlab_poll.rs`, new, plus exactly one added call site in `crates/gridops-reconciler/src/main.rs`):
- Every reconciler pass, for each GitLab connection a pool targets (`runner_pool_gitlab_connections`), build its `GitLabCredentials` (base URL plus the access token unsealed from `access_token_key` the same way Bitbucket connections' tokens are) and list `pending_jobs`. For a `project` connection that is its `scope_id`; for a `group` connection, list `group_projects` and poll each.
- Upsert any job missing from `gitlab_jobs` (new UUID `id`, contract `tag_list` encoding, status `JobStatus::as_str()`), and refresh the status of recorded jobs that GitLab still reports. Never overwrite a newer webhook-written status with an older one (use `status_changed_at`), and never resurrect a terminal row.
- Conclude recorded active jobs of that connection that are no longer pending and never reported a terminal event: call `GitLabApi::job` and write its status; a `NotFound` concludes it as `canceled` (or another documented terminal status).
- Back off on `GitLabError::RateLimited { retry_after }`: skip that connection until the retry-after (or a documented default) has elapsed, without blocking other connections or the rest of the pass. Other errors are logged per connection and do not fail the pass.
- The reconciler needs `gitlab: Arc<dyn GitLabApi>` on `Reconciler`. Per the #139 notes, #139 owns that field next to `bitbucket` and wires a placeholder client returning `GitLabError::Upstream("GitLab client not available yet")` until #146's client lands. If #139 is not on master when this lands, add exactly the same field (name `gitlab`, type `Arc<dyn GitLabApi>`) and placeholder; whichever lands second reconciles to one copy.
- Do not touch `crates/gridops-reconciler/src/gitlab_lifecycle.rs` or its call sites (#139 owns them).

## Constraints

- Six other wave-1 sessions edit nearby files in parallel (#146 client, #138 API, #139 lifecycle, #147 host agent, #140 webhooks, #142 Tart, #141 UI). Keep the diff to the files above plus tests.
- Tests seed `gitlab_jobs` rows directly; the poll tests use `FakeGitLab` (`with_job`, `with_group_project`, `fail_next`). Tests that need two connections isolated must give them different base URLs (the fake records no credential identity).
- No deploy. Mars production stays at `80e6f1b`.

## Done when

- Autoscaling tests cover: tag matching (all tags present → match; one missing → no match; case-insensitive), untagged jobs with and without `run_untagged`, an exclusive pool, the handoff from a saturated pool to the next compatible GitLab pool, no collision between a GitHub job and a GitLab job with the same numeric id, and scale-down of an idle GitLab pool (plus no scale-down while it owns a GitLab job).
- Poll tests (with `FakeGitLab`) show: a pending job with no webhook row is recorded and scales the pool; a group connection is polled through `group_projects`; a stale recorded job is concluded via `job`; backoff on a 429 (`RateLimited`) skips that connection on the next pass and leaves others polled.
- `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings` and `cargo test` pass for the workspace.
