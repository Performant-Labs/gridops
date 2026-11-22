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
