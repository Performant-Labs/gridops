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
- Active `gitlab_jobs` rows feed `queued_job_candidates` alongside `workflow_jobs`, with the status tiers and the output surface fixed in **Amendments (A review)** below (A-B2, A-B3): only `pending` is scaler demand, and only `assigned_queued_jobs` (room/scale) sees GitLab jobs; the public GitHub-keyed outputs stay GitHub-only.
- Candidate pools for a GitLab job are the pools linked to its connection through `runner_pool_gitlab_connections`.
- A job matches a pool when every job tag (case-insensitive) is among the pool's **configured** labels (`runner_pools.labels`) — see A-B4. An untagged job (`tag_list` = `[]`) matches only pools with `runner_pools.run_untagged = 1`.
- Exclusive pools (#134, migration 0046 `runner_pool_exclusive_labels`, already on master) must be respected; the rule is fixed in A-B4.
- Ownership and the saturated-pool handoff work exactly as they do for GitHub (same ordering, room, standing, paused/manual rules).
- **Job identity hazard**: candidates and owners are keyed by an `i64` job id today (`workflow_jobs.id`). GitLab job ids are a different id space (and only unique per connection) and must never collide with a GitHub job id or with each other across connections. The key is fixed in A-B2.
- GitLab pools aren't covered by the Bitbucket "never scale down" guard in `maybe_scale_down` (`crates/gridops-reconciler/src/main.rs`): that guard keys on `runner.ci_platform == "bitbucket"`, so a GitLab pool must scale down when idle with no owned GitLab jobs. Prove it with a test; do not edit that guard unless a test shows it is needed.

**Pending-jobs poll** (`crates/gridops-reconciler/src/gitlab_poll.rs`, new, plus the `main.rs` edits listed in A-W1):
- On its own worker schedule (A-B1, not inside the pass), for each GitLab connection a pool targets (`runner_pool_gitlab_connections`), build its `GitLabCredentials` (base URL plus the access token unsealed from `access_token_key` the same way Bitbucket connections' tokens are) and list `pending_jobs`. For a `project` connection that is its `scope_id`; for a `group` connection, list `group_projects` and poll each.
  - How a group-polled job reaches the right pools (brief review R1, NV-3): pools link to a **connection**, never to a project. `runner_pool_gitlab_connections` holds only `(pool_id, connection_id)`, and `gitlab_jobs` holds `connection_id` and `project_id` (excerpts under "Verified source excerpts"). A job found through a group connection's project is recorded with that group connection's `connection_id` and the project's `project_id`, and its candidate pools are the pools linked to that `connection_id`; `project_id` plays no part in pool selection.
- Upsert any job missing from `gitlab_jobs` (new UUID `id`, contract `tag_list` encoding, status `JobStatus::as_str()`), and refresh the status of recorded jobs that GitLab still reports. Never overwrite a newer webhook-written status with an older one (use `status_changed_at`), and never resurrect a terminal row.
- Conclude recorded active jobs of that connection that are no longer pending and never reported a terminal event: call `GitLabApi::job` and write its status; a `NotFound` concludes it as `canceled` (or another documented terminal status).
- Back off on `GitLabError::RateLimited { retry_after }`: skip that connection until the retry-after (or a documented default) has elapsed, without blocking other connections or the rest of the poll. Other errors are logged per connection and do not fail the pass.
- The reconciler needs `gitlab: Arc<dyn GitLabApi>` on `Reconciler`. Per the #139 notes, #139 owns that field next to `bitbucket` and wires a placeholder client returning `GitLabError::Upstream("GitLab client not available yet")` until #146's client lands. If #139 is not on master when this lands, add exactly the same field (name `gitlab`, type `Arc<dyn GitLabApi>`) and placeholder; whichever lands second reconciles to one copy.
- Do not touch `crates/gridops-reconciler/src/gitlab_lifecycle.rs` or its call sites (#139 owns them).

## Constraints

- Six other wave-1 sessions edit nearby files in parallel (#146 client, #138 API, #139 lifecycle, #147 host agent, #140 webhooks, #142 Tart, #141 UI). Keep the diff to the files above plus tests.
- Tests seed `gitlab_jobs` rows directly; the poll tests use `FakeGitLab` (`with_job`, `with_group_project`, `fail_next`). Tests that need two connections isolated must give them different base URLs (the fake records no credential identity).
- No deploy. Mars production stays at `80e6f1b`.

## Done when

- Autoscaling tests cover: tag matching (all tags present → match; one missing → no match; case-insensitive), untagged jobs with and without `run_untagged`, an exclusive pool, the handoff from a saturated pool to the next compatible GitLab pool, no collision between a GitHub job and a GitLab job with the same numeric id, and scale-down of an idle GitLab pool (plus no scale-down while it owns a GitLab job).
- Poll tests (with `FakeGitLab`) show: a pending job with no webhook row is recorded and scales the pool; a group connection is polled through `group_projects`; a stale recorded job is concluded via `job`; backoff on a 429 (`RateLimited`) skips that connection on the next poll and leaves others polled; the conclusion step checks at most `GITLAB_JOB_CHECK_LIMIT` rows and only ones older than `GITLAB_JOB_CHECK_AGE_MS`.
- Poll tests (with `FakeGitLab`) also show the two status invariants from the scope (brief review R1, B-1):
  - **No older status over a newer webhook status**: a `gitlab_jobs` row seeded as a webhook would write it (for example `status = 'running'`) with a `status_changed_at` later than the poll's observation time for that pass, while `FakeGitLab` still reports the same `(project_id, job_id)` as `Pending`. After the pass the row keeps `running` and its `status_changed_at` is unchanged. (The fake carries no timestamps, so the test makes the webhook write the newer one by seeding `status_changed_at` after the pass's start.)
  - **No resurrection of a terminal row**: a `gitlab_jobs` row seeded with a terminal status (one where `JobStatus::is_terminal()` holds, for example `success`) while `FakeGitLab` still reports the same job as `Pending`. After the pass the row's `status` and `status_changed_at` are unchanged, and the job produces no `queued_job_candidates` row, so it scales no pool.
- A poll test concludes a stale recorded active job through each `job` outcome (brief review R1, W-4): one where `FakeGitLab` returns the job with a terminal status, which is written as `JobStatus::as_str()`; and one where the job is not seeded, so `job` returns `GitLabError::NotFound`, which concludes the row as `canceled` (or the other terminal status the implementation documents, asserted by the test).
- An autoscaling test shows the demand scan reads only the D8 active statuses (brief review R1, W-5): `gitlab_jobs` rows with `manual`, `scheduled`, `unknown` and a terminal status (for example `success`), whose tags match an autoscaling pool linked to their connection, produce no candidates (`assigned_queued_jobs` for that pool is 0 and `queued_job_ownership` has no entry for them).
- Tests pin the A-B3 and A-B4 rules: a `running` GitLab job is **not** demand; a `created` (also `waiting_for_resource`, `preparing`) job adds **no** scaler demand; a job tagged `self-hosted`, `self-hosted-linux`, `linux` or an arch label does **not** match a pool (exclusive or not) that does not configure that label, and **does** match a non-exclusive pool that configures it.
- `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings` and `cargo test` pass for the workspace.

## Amendments (A review of the plan, `docs/handoffs/148/handoff-A.md`)

These decisions are binding and replace any earlier text in this brief that differs.

### A-B1. The poll is its own worker, bounded

- `gitlab_poll::worker(app: Reconciler)` is spawned from `main()` with `tokio::spawn`, next to `github_sync_worker`, and never runs inside `reconcile()`. It loops: one poll of every connection, then sleeps for the reconcile interval (the same `reconcileIntervalSeconds` setting `run_passes` uses; if that is not cheaply readable from the worker, a named constant `GITLAB_POLL_INTERVAL` of 30 s, documented).
- Every `gitlab_jobs` write goes through the existing `retry_when_busy` helper, as the GitHub sync's writes do.
- Each connection's poll runs under `tokio::time::timeout(GITLAB_CONNECTION_POLL_BUDGET, ..)` (named constant, 60 s); a timeout is logged for that connection and the worker moves on.
- The conclusion step is bounded like `confirm_jobs`: it only checks rows of that connection whose status is active and not `pending` in GitLab's answer, and whose `status_changed_at` (or `updated_at`) is older than `GITLAB_JOB_CHECK_AGE_MS` (named, 5 minutes, mirroring `JOB_CHECK_AGE_MS`), oldest first, at most `GITLAB_JOB_CHECK_LIMIT` (named, 10, mirroring `JOB_CHECK_LIMIT`) per connection per poll. `running` rows are included (a running job whose terminal webhook was missed must still conclude), but only once they are that old.
- The rate-limit skip-until map (`HashMap<connection_id, Instant>`) lives in the worker's own state, not in `Reconciler` or the database. Expose a single-poll function (e.g. `poll_once(app, &mut backoff, now)`) so tests drive individual polls deterministically without the loop.

### A-B2. GitLab jobs stay out of the GitHub id spaces

- Inside `autoscaling.rs`, candidates are keyed by a **private** enum, e.g. `enum QueuedJobKey { Github(i64), Gitlab { connection_id: String, job_id: i64 } }`, with one total order across platforms (A-W4). No collision is possible between platforms or across connections.
- GitLab jobs take room and are owned by pools exactly like GitHub jobs (so the saturated-pool handoff applies to both), and they **are counted by `assigned_queued_jobs`** (so scale-up and `maybe_scale_down` see them).
- `queued_job_ownership`, `waiting_job_pools`, `waiting_job_ownership`, `queued_job_assignments`, `repository_capacities` and `next_runner_provider` / `next_runner_repository` **keep their public types and emit GitHub jobs only** (GitLab entries are filtered out after ownership is assigned, so a GitLab job still consumes room ahead of a later GitHub job in a shared pool). Their callers in `gridops-api` and fleet placement are untouched. Surfacing GitLab jobs in the queue views is out of scope for #148 (a follow-up for the UI/API issues).
- `QueuedJob.repository_id` and other GitHub repository ids never receive a GitLab value.

### A-B3. GitLab status tiers

- **Scaler demand** (the tier the scaler counts, like GitHub `queued`): `pending` only.
- **Waiting, display tier** (the `every_waiting_status` tier, like GitHub `waiting`/`pending`/`requested`): `created`, `waiting_for_resource`, `preparing`. These never count toward scale-up.
- `running` is **not** demand in either tier: #140 marks the runner busy, and room / `scale_up_target` already count busy runners.
- `manual`, `scheduled`, `unknown` and terminal statuses are never read.
- The SQL filters with the D8 IN-list verbatim (`status IN ('created','waiting_for_resource','preparing','pending','running')`) so the partial index `gitlab_jobs_active_created_idx` applies, then narrows by tier.

### A-B4. Tag matching matches #139's registered tags

- #139 registers each GitLab runner with the pool's configured labels (`runner_pools.labels`) as tags, with no system or catch-all labels. Therefore a GitLab job matches a pool only when every job tag is among the pool's **configured** labels. The GitHub platform/system labels (`self-hosted`, `self-hosted-linux`, `self-hosted-macos`, OS, arch) are never implied for GitLab; `pool_platforms!()` is not used for GitLab matching. A job tagged `linux` matches only a pool that configures `linux`.
- Exclusive pools: since only configured labels match, an exclusive pool is treated exactly like any other pool for GitLab (it can never match through a catch-all label, because no catch-all is implied); a test pins that an exclusive pool matches a job whose tags it configures and not one tagged with an unconfigured catch-all.
- Case: GitLab stores runner/job tags through acts-as-taggable-on with case-insensitive matching, so comparison is case-insensitive (`lower()` on both sides), consistent with the GitHub path. #143 (end-to-end against real GitLab) verifies this; if it shows GitLab is case-sensitive, the comparison changes there.
- An untagged job matches only pools with `run_untagged = 1` (and any such pool linked to its connection, whatever its labels).
- This rule is recorded on epic #136 by the orchestrator.

### A-W1. Allowed `main.rs` edits (#139 is not on master)

`gitlab: Arc<dyn GitLabApi>` on `Reconciler` is #139's field; if #139 is not on master when this lands, #148 adds it. Allowed edits outside `gitlab_poll.rs`: `mod gitlab_poll;` in `main.rs`; the `gitlab` field on `Reconciler`; its value in `main()`'s constructor (the placeholder); the `tokio::spawn(gitlab_poll::worker(..))` line; and the `gitlab` field in the test constructors (`main.rs` near line 4825 and `start_tests.rs` near line 425, and any other `Reconciler { .. }` literal the compiler requires). The placeholder is a private type `UnavailableGitLab` in `gitlab_poll.rs` returning `GitLabError::Upstream("GitLab client not available yet".into())` from every method (#139 keeps its own in `gitlab_lifecycle.rs`; whichever lands second keeps one).

### A-W2. One credentials loader

`gitlab_poll.rs` defines `pub(crate) async fn gitlab_credentials(app: &Reconciler, connection_id: &str) -> Result<Option<GitLabCredentials>>` (reads `base_url` and `access_token_key` from `gitlab_connections`, token via `runtime_secret`, `None` when the connection or secret is gone). First to land adds it; #139 reuses it when it rebases (a note is left for #139 in the shared notes folder).

### A-W3. One status rule for two writers

- A terminal row never changes.
- The poll never moves a row backwards along `created/waiting_for_resource/preparing → pending → running → terminal`: it only writes a status that is later in that order than the stored one (or equal, with no write), and only when the row's `status_changed_at` is older than the poll's observation time, which is taken **before** the `pending_jobs` request.
- A conclusion through `job` writes the terminal status GitLab reports (or `canceled` on `NotFound`) with `status_changed_at` = that observation time.
- New rows inserted by the poll use `INSERT .. ON CONFLICT(connection_id, job_id) DO NOTHING` (then the guarded update), so a concurrent webhook insert is never clobbered. Recorded on epic #136.

### A-W4. One ordered candidate list

GitHub and GitLab candidates form one ordered list (a `UNION ALL` or an in-Rust merge) ordered by job `created_at`, then platform (GitHub before GitLab), then the job key, then the existing pool order (repository-scoped pools first, then `created_at`, `id`). The GitLab arm reuses the same `room` subquery and `PoolStanding::of`. A GitLab pool can therefore also own GitHub jobs with consistent ownership.

## Verified source excerpts (brief review R1, NV-1 to NV-4)

Read on this branch (`31c0035`, on top of master `b22c451`); quoted verbatim.

- **NV-1: the scale-down guard.** `crates/gridops-reconciler/src/main.rs:1396-1407`, `maybe_scale_down`. The guard returns early when **any** runner of the pool has `ci_platform == "bitbucket"`; a pool whose runners are all `gitlab` falls through to the busy check, `assigned_queued_jobs` and the idle timer:

      async fn maybe_scale_down(app: &Reconciler, pool: &Pool, runners: &[Runner]) -> Result<()> {
          if !pool.autoscaling_enabled || pool.paused || pool.desired_count <= pool.min_count {
              return Ok(());
          }
          // Bitbucket does not publish a per-step busy signal to GridOps. Keep its
          // persistent runner until an operator explicitly removes or pauses it.
          if runners
              .iter()
              .any(|runner| runner.ci_platform == "bitbucket")
          {
              return Ok(());
          }
          if runners.iter().any(|runner| runner.busy) {
              return Ok(());
          }
          let queued = assigned_queued_jobs(&app.database, &pool.id).await?;

  `Reconciler` (`main.rs:42-57`) has `bitbucket: BitbucketClient` and no `gitlab` field yet on this branch.

- **NV-2: `GitLabApi`.** `crates/gridops-core/src/gitlab/mod.rs:221-272`. Every one of its seven methods takes `credentials: &GitLabCredentials,` first. The two the poll needs besides `job` (`mod.rs:239-244`), `pending_jobs` (`mod.rs:246-250`) and `group_projects` (`mod.rs:252-258`):

      async fn pending_jobs(
          &self,
          credentials: &GitLabCredentials,
          project_id: ExternalId,
      ) -> Result<Vec<Job>, GitLabError>;

      /// The projects of a group, subgroups included, so a group connection can
      /// be polled project by project (GitLab has no group-level jobs list).
      async fn group_projects(
          &self,
          credentials: &GitLabCredentials,
          group_id: ExternalId,
      ) -> Result<Vec<ExternalId>, GitLabError>;

  `GitLabError` (`mod.rs:203-219`) has `NotFound` and `RateLimited { retry_after: Option<Duration> }`. `JobStatus::is_terminal()` is `Success | Failed | Canceled | Skipped`; `is_active()` is `Created | WaitingForResource | Preparing | Pending | Running`; `GITLAB_ACTIVE_JOB_STATUSES` is `["created", "waiting_for_resource", "preparing", "pending", "running"]`.

- **NV-3: pool-to-connection linkage.** `migrations/0047_gitlab.sql:44-53` and `62-82`:

      CREATE TABLE `runner_pool_gitlab_connections` (
        `pool_id` text NOT NULL,
        `connection_id` text NOT NULL,
        `created_at` integer NOT NULL,
        PRIMARY KEY (`pool_id`,`connection_id`),

      CREATE TABLE `gitlab_jobs` (
        `id` text PRIMARY KEY NOT NULL,
        `connection_id` text NOT NULL REFERENCES `gitlab_connections`(`id`) ON DELETE cascade,
        `project_id` integer NOT NULL,
        `job_id` integer NOT NULL,

  `gitlab_connections` holds `scope_kind` (`'project'` or `'group'`) and `scope_id`. No table links a pool to a GitLab project, so routing is by `connection_id` (see the Scope note).

- **NV-4: `FakeGitLab`.** `crates/gridops-core/src/gitlab/fake.rs`. The helpers exist with these signatures: `with_job(&self, base_url: &Url, job: Job)` (`fake.rs:98`), `with_group_project(&self, base_url: &Url, group_id: ExternalId, project_id: ExternalId)` (`fake.rs:108`), `fail_next(&self, base_url: &Url, method: FakeMethod, error: GitLabError)` (`fake.rs:120`), plus `calls()` and `FakeMethod::{Job, PendingJobs, GroupProjects, ..}`. Behaviour the poll tests rely on:
  - `pending_jobs` returns only seeded jobs of that project whose status is `JobStatus::Pending` (`fake.rs:245`: `.filter(|job| job.project_id == project_id && job.status == JobStatus::Pending)`), so a job seeded `Running` or terminal is "no longer pending" to the poll.
  - `job` returns the seeded job by `(project_id, job_id)` or `GitLabError::NotFound` (`fake.rs:226-233`).
  - A 429 is expressed as `fail_next(&base_url, FakeMethod::PendingJobs, GitLabError::RateLimited { retry_after: Some(..) })` (or `None`); the failure is one-shot, so the next call on that base URL succeeds unless the poll skips it.
  - A group project is expressed as `with_group_project(&base_url, group_id, project_id)` plus `with_job` for that project.
  - All state is keyed by `base_url` alone (`fake.rs:67-76`), hence the different-base-URLs rule in Constraints.

## Source material read

- `docs/handoffs/148-brief.md` (this brief, in full)
- `docs/handoffs/148-brief-result-r1.md` (the round-1 review, in full)
- `migrations/0047_gitlab.sql` (in full)
- `migrations/0046_runner_pool_exclusive_labels.sql` (in full)
- `crates/gridops-core/src/gitlab/mod.rs` (in full)
- `crates/gridops-core/src/gitlab/fake.rs` (in full)
- `crates/gridops-core/src/autoscaling.rs` (outline of every item, and lines 210-600 in full: `assigned_queued_jobs`, `queued_job_owners`, `PoolStanding`, `waiting_job_pools`, `queued_job_candidates`, `pool_platform_labels`, `assign_queued_jobs`)
- `crates/gridops-reconciler/src/main.rs` (`Reconciler` at lines 40-60, `maybe_scale_down` and the scale-down candidate filter at lines 1370-1440, and every `ci_platform` / `bitbucket` site by search)
- `docs/handoffs/137-brief.md` (the FakeGitLab, R2-W3 `gitlab` field ownership and deviation-table sections in full; the rest by search for #148, `status_changed_at`, `NotFound`, rate limiting and the active-jobs index)
- `crates/gridops-reconciler/src/gitlab_poll.rs`: not found (new in this issue)
- `crates/gridops-reconciler/src/gitlab_lifecycle.rs`: not found (#139's, not on this branch)
