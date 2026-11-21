# Evidence: #148 (unchanged source the diff relies on)

## F (Phase 6)

- **Fact:** `maybe_scale_down` returns early only when some runner of the pool is a Bitbucket runner; a pool of GitLab runners falls through to the busy check and to `assigned_queued_jobs`, which now counts the GitLab jobs the pool owns. #148 does not edit this guard.
  **Source:** `crates/gridops-reconciler/src/main.rs:1412-1423`
  **Verbatim excerpt:**
  > // Bitbucket does not publish a per-step busy signal to GridOps. Keep its
  > // persistent runner until an operator explicitly removes or pauses it.
  > if runners
  >     .iter()
  >     .any(|runner| runner.ci_platform == "bitbucket")
  > {
  >     return Ok(());
  > }
  > if runners.iter().any(|runner| runner.busy) {
  >     return Ok(());
  > }
  > let queued = assigned_queued_jobs(&app.database, &pool.id).await?;

- **Fact:** `retry_when_busy`, which wraps every `gitlab_jobs` write of the poll, re-runs the whole closure (so a whole transaction) up to three more times, a second apart, only when the error is `SQLITE_BUSY`; any other result is returned as is.
  **Source:** `crates/gridops-reconciler/src/main.rs:498-511`
  **Verbatim excerpt:**
  > async fn retry_when_busy<T, F, Fut>(mut write: F) -> Result<T>
  > where
  >     F: FnMut() -> Fut,
  >     Fut: std::future::Future<Output = Result<T>>,
  > {
  >     for _ in 0..3 {
  >         match write().await {
  >             Err(error) if database_busy(&error) => {
  >                 tokio::time::sleep(Duration::from_secs(1)).await;
  >             }
  >             result => return result,
  >         }
  >     }
  >     write().await
  > }

- **Fact:** the poll worker sleeps for the same setting, fallback and clamp the pass loop uses between passes.
  **Source:** `crates/gridops-reconciler/src/main.rs:417-419`
  **Verbatim excerpt:**
  > let interval = setting_i64(&reconciler.database, "reconcileIntervalSeconds", 30)
  >     .await
  >     .clamp(5, 3_600);

- **Fact:** `runtime_secret`, which `gitlab_credentials` calls with the connection's `access_token_key`, answers `None` for a missing key and otherwise opens the sealed value with the vault, as for Bitbucket tokens.
  **Source:** `crates/gridops-reconciler/src/main.rs:3758-3764`
  **Verbatim excerpt:**
  > sealed
  >     .map(|value| {
  >         app.vault
  >             .open(&value)
  >             .context("could not decrypt runtime secret")
  >     })
  >     .transpose()

- **Fact:** the active-jobs partial index's predicate is exactly the `IN` list the demand scan and the poll's conclusion step filter on, so both can use it.
  **Source:** `migrations/0047_gitlab.sql:81-82`
  **Verbatim excerpt:**
  > CREATE INDEX `gitlab_jobs_active_created_idx` ON `gitlab_jobs` (`created_at`,`id`)
  >   WHERE `status` IN ('created','waiting_for_resource','preparing','pending','running');

- **Fact:** the poll and the webhook identify a job row by `(connection_id, job_id)`, the conflict target of D18's upsert; `tag_list` defaults to `'[]'`.
  **Source:** `migrations/0047_gitlab.sql:62-77`
  **Verbatim excerpt:**
  > CREATE TABLE `gitlab_jobs` (
  >   `id` text PRIMARY KEY NOT NULL,
  >   `connection_id` text NOT NULL REFERENCES `gitlab_connections`(`id`) ON DELETE cascade,
  >   `project_id` integer NOT NULL,
  >   `job_id` integer NOT NULL,
  >   `pipeline_id` integer NOT NULL,
  >   `status` text NOT NULL,
  >   `tag_list` text NOT NULL DEFAULT '[]',
  >   `status_changed_at` integer NOT NULL,
  >   `retries_count` integer NOT NULL DEFAULT 0,
  >   `gitlab_runner_id` integer,
  >   `pool_id` text REFERENCES `runner_pools`(`id`) ON DELETE set null,
  >   `created_at` integer NOT NULL,
  >   `updated_at` integer NOT NULL
  > );
  > CREATE UNIQUE INDEX `gitlab_jobs_connection_job_unique` ON `gitlab_jobs` (`connection_id`,`job_id`);

- **Fact:** one GitLab instance (one `base_url`) can hold both a project connection and a group connection, which is why the demand scan deduplicates a job on `(base_url, job_id)` and `set_gitlab_runner_busy` matches runners on `base_url`.
  **Source:** `migrations/0047_gitlab.sql:25-39`
  **Verbatim excerpt:**
  > CREATE TABLE `gitlab_connections` (
  >   `id` text PRIMARY KEY NOT NULL,
  >   `name` text NOT NULL,
  >   `base_url` text NOT NULL,
  >   `scope_kind` text NOT NULL CHECK (`scope_kind` IN ('project','group')),
  >   `scope_id` integer NOT NULL CHECK (`scope_id` > 0),
  > ...
  > CREATE UNIQUE INDEX `gitlab_connections_scope_unique` ON `gitlab_connections` (`base_url`,`scope_kind`,`scope_id`);

- **Fact:** the `runner_events` row the poll writes on freeing a runner uses the table's columns as the GitHub webhook's row does (`runner_id`, `pool_id`, `level`, `event`, `message`, `metadata`).
  **Source:** `migrations/0001_initial.sql:83-91`
  **Verbatim excerpt:**
  > CREATE TABLE `runner_events` (
  > 	`id` text PRIMARY KEY NOT NULL,
  > 	`runner_id` text,
  > 	`pool_id` text,
  > 	`level` text DEFAULT 'info' NOT NULL,
  > 	`event` text NOT NULL,
  > 	`message` text NOT NULL,
  > 	`metadata` text DEFAULT '{}' NOT NULL,
  > 	`created_at` integer NOT NULL,

- **Fact:** `FakeGitLab::pending_jobs` answers only the seeded jobs of that project whose status is `Pending`, so in the poll tests a job seeded `Running` or finished is "no longer pending" and reaches the conclusion step.
  **Source:** `crates/gridops-core/src/gitlab/fake.rs:245`
  **Verbatim excerpt:**
  > .filter(|job| job.project_id == project_id && job.status == JobStatus::Pending)

- **Fact:** the GitLab arm's bare platform check reuses `pool_platforms!()`, which yields no row for a pool with no Docker/Tart provider and no enabled fleet profile, so such a pool never owns a GitLab job (A2-W5).
  **Source:** `crates/gridops-core/src/autoscaling.rs:59-74`
  **Verbatim excerpt:**
  > macro_rules! pool_platforms {
  >     () => {
  >         "
  >                 -- The built-in host's runners, unless the pool runs on machines only.
  >                 SELECT CASE lower(CAST(provider.value AS TEXT)) WHEN 'tart' THEN 'macos' ELSE 'linux' END AS os,
  >                   CASE lower(CAST(provider.value AS TEXT)) WHEN 'tart' THEN 'arm64' ELSE lower(?) END AS arch
  >                 FROM json_each(candidate.providers) provider
  >                 WHERE lower(CAST(provider.value AS TEXT)) IN ('docker','tart')
  >                   AND NOT EXISTS (SELECT 1 FROM pool_execution_profiles machines_only
  >                     WHERE machines_only.pool_id=candidate.id AND machines_only.enabled=1
  >                       AND machines_only.archived_at IS NULL AND machines_only.placement='machines')
  >                 UNION ALL
  >                 -- Each fleet platform the pool runs on.
  >                 SELECT profile.execution_os,profile.architecture FROM pool_execution_profiles profile
  >                 WHERE profile.pool_id=candidate.id AND profile.enabled=1 AND profile.archived_at IS NULL
