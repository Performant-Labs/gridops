# Evidence: #181 fleet GitLab pools scale on pending jobs

Facts the diff relies on that live in unchanged code.

- **Fact:** a pool linked to GitLab places a fleet runner for its GitLab connection, registered to no repository, before any GitHub repository is considered. So on the fleet, a `None` spare runner in a pool linked to GitLab is a GitLab runner, which is why `attribute_queued_jobs` can let any `None` spare cover a GitLab job.
  **Source:** `crates/gridops-reconciler/src/fleet.rs:161-168`
  **Verbatim excerpt:**
  > ```
  >     // A GitLab pool's runner is placed for its selected connection's own
  >     // target, never the pool's first target (#139).
  >     let gitlab_connection = gitlab_lifecycle::next_connection(app, &pool.id).await?;
  >     let (target_id, repository_id) = if let Some(connection) = &gitlab_connection {
  >         (
  >             gitlab_lifecycle::fleet_target(app, pool, profile, connection).await?,
  >             None,
  >         )
  > ```

- **Fact:** GitHub candidates (and so owners, for a GitHub-only pool) are ordered by `wj.created_at, wj.id`, the same order the shared helper's SQL uses (`ORDER BY wj.created_at,wj.id`). So `fleet_queued_jobs` walking owner order gives GitHub-only pools the same job order `queued_job_assignments` gave `attribute_queued_jobs` before.
  **Source:** `crates/gridops-core/src/autoscaling.rs:616-618`
  **Verbatim excerpt:**
  > ```
  >           WHERE (wj.status='queued' OR (? AND wj.status IN ('waiting','pending','requested')))
  >             AND wr.status!='completed'
  >           ORDER BY wj.created_at,wj.id,
  > ```

- **Fact:** GitHub and GitLab candidates are merged and re-sorted only when there are GitLab candidates, so with no GitLab jobs the GitHub SQL order stands.
  **Source:** `crates/gridops-core/src/autoscaling.rs:563-572`
  **Verbatim excerpt:**
  > ```
  >     let gitlab = gitlab_job_candidates(database, every_waiting_status).await?;
  >     if !gitlab.is_empty() {
  >         candidates.extend(gitlab);
  >         // Stable, so each job's pools keep their order and stay together.
  >         candidates.sort_by(|left, right| {
  >             left.queued_at
  >                 .cmp(&right.queued_at)
  >                 .then_with(|| left.job.cmp(&right.job))
  >         });
  >     }
  > ```

- **Fact:** the scaler's owners (which `fleet_queued_jobs` reads) include GitLab jobs only at tier 1 (`pending`; `queued_job_owners` calls the candidates with `every_waiting_status=false`), so `created` and `manual` jobs are never fleet demand.
  **Source:** `crates/gridops-core/src/autoscaling.rs:689`
  **Verbatim excerpt:**
  > ```
  >           WHERE job.tier=1 OR (? AND job.tier=0)
  > ```

- **Fact:** in `choose_platform`'s deficit, a non-busy runner with no repository always counts as active, so a starting GitLab runner covers the GitLab job counted in `queued[index]` and the platform stops being behind.
  **Source:** `crates/gridops-core/src/fleet/targets.rs:1539-1543`
  **Verbatim excerpt:**
  > ```
  >             let is_busy = row.get::<bool, _>("busy");
  >             let useful = is_busy
  >                 || row
  >                     .get::<Option<i64>, _>("target_repository_id")
  >                     .is_none_or(|repository| waiting[index].contains(&repository));
  > ```

- **Fact:** the reconciler's `provision` refuses a fleet runner when the platform is not behind and the pool is at `min_count` (the `NoDemand` guard this fix leaves unchanged; its `behind` input is what changed).
  **Source:** `crates/gridops-reconciler/src/main.rs:2442-2451`
  **Verbatim excerpt:**
  > ```
  >     if !behind && placement == Placement::Machines {
  >         let active: i64 = sqlx::query_scalar(
  >             "SELECT COUNT(*) FROM runners WHERE pool_id=? AND deleted_at IS NULL
  >                AND status IN ('starting','online','idle','busy','paused','stopped')",
  >         )
  >         .bind(&pool.id)
  >         .fetch_one(&app.database)
  >         .await?;
  >         if active >= pool.min_count {
  >             return Ok(ProvisionAttempt::NoDemand);
  > ```

- **Fact:** one reconcile pass calls `provision` once per missing runner up to `desired`, so one pass may place several fleet runners.
  **Source:** `crates/gridops-reconciler/src/main.rs:1077-1079`
  **Verbatim excerpt:**
  > ```
  >     if !provisioning_blocked && !start_ended && active.len() < desired as usize {
  >         for _ in active.len()..desired as usize {
  >             let mut attempt = provision(app, view, pool, builtin_enabled).await?;
  > ```

## Added by T (S test-only rework, R1 and R2)

- **Fact:** `maybe_scale_down` resets `desired_count` to `min_count` once the idle timeout has elapsed since the newest runner update (0 with no runners), and returns early otherwise. With a huge `idle_timeout_minutes` the timeout never elapses, so the desired count stays raised and `provision` runs (R1).
  **Source:** `crates/gridops-reconciler/src/main.rs:1533-1535`, `crates/gridops-reconciler/src/main.rs:1552-1563`, `crates/gridops-reconciler/src/main.rs:4033-4035`
  **Verbatim excerpt:**
  > ```
  > async fn maybe_scale_down(app: &Reconciler, pool: &Pool, runners: &[Runner]) -> Result<()> {
  >     if !pool.autoscaling_enabled || pool.paused || pool.desired_count <= pool.min_count {
  >         return Ok(());
  > ...
  >     let last_activity = runners
  >         .iter()
  >         .map(|runner| runner.updated_at)
  >         .max()
  >         .unwrap_or(0);
  >     if !idle_period_elapsed(now_millis(), pool.idle_timeout_minutes, last_activity) {
  >         return Ok(());
  >     }
  >     sqlx::query("UPDATE runner_pools SET desired_count=?,state='scaling',updated_at=? WHERE id=?")
  >         .bind(pool.min_count)
  > ...
  > fn idle_period_elapsed(now: i64, idle_timeout_minutes: i64, last_activity: i64) -> bool {
  >     now.saturating_sub(last_activity) >= idle_timeout_minutes.saturating_mul(60_000)
  > }
  > ```

- **Fact:** owners are listed oldest job first, so a GitLab job recorded at time 10 is attributed before a GitHub job queued now (R2).
  **Source:** `crates/gridops-core/src/autoscaling.rs:315`
  **Verbatim excerpt:**
  > ```
  > /// The one pool that counts each queued job, oldest job first. Every view of
  > ```

- **Fact:** `attribute_queued_jobs` puts single-platform jobs first (a stable sort, so multi-platform jobs keep owner order), and an uncovered multi-platform job with no room and no machine available counts for the first platform (R2: the uncovered GitLab job lands on x64).
  **Source:** `crates/gridops-core/src/fleet/targets.rs:1444`, `crates/gridops-core/src/fleet/targets.rs:1462-1472`
  **Verbatim excerpt:**
  > ```
  >     jobs.sort_by_key(|(platforms, _)| platforms.len() > 1);
  > ...
  >             let index = if let [only] = platforms.as_slice() {
  >                 *only
  >             } else {
  >                 platforms
  >                     .iter()
  >                     .copied()
  >                     .filter(|index| room.slots(*index) > 0)
  >                     .max_by_key(|index| (room.slots(*index), std::cmp::Reverse(*index)))
  >                     .or_else(|| platforms.iter().copied().find(|index| available[*index]))
  >                     .or_else(|| platforms.first().copied())?
  >             };
  > ```

- **Fact:** in `choose_platform`'s deficit, an idle runner registered to a repository counts as active only when a job of that repository waits on its platform. So on one platform the `behind` flag cannot tell whether a repository runner "covered" a GitLab job, which is why R2 asserts the attribution through `platform_jobs` instead.
  **Source:** `crates/gridops-core/src/fleet/targets.rs:1539-1547`
  **Verbatim excerpt:**
  > ```
  >             let is_busy = row.get::<bool, _>("busy");
  >             let useful = is_busy
  >                 || row
  >                     .get::<Option<i64>, _>("target_repository_id")
  >                     .is_none_or(|repository| waiting[index].contains(&repository));
  >             if useful {
  >                 active += 1;
  >                 busy += i64::from(is_busy);
  >             }
  > ```
