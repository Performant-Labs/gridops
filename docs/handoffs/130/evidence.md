# Evidence: #130 cap the time a fleet placement spends in `registering`

Source facts in unchanged code that the change (F) relies on.

- **Fact:** `abandon_unstarted` refuses, changing nothing, once a `start_runner` command exists for the placement, so the cap cannot end a placement that was started (including one started earlier in the same pass).
  **Source:** `crates/gridops-core/src/fleet/placement.rs:386-394`
  **Verbatim excerpt:**
  > let started: bool = sqlx::query_scalar(
  >     "SELECT EXISTS(SELECT 1 FROM host_commands WHERE placement_id=? AND kind='start_runner')",
  > )
  > .bind(&id)
  > .fetch_one(&mut *tx)
  > .await?;
  > if started {
  >     return Ok(false);
  > }

- **Fact:** `abandon_unstarted` returns `true` only when it released at least one capacity allocation, so the give-up warn line in `expire_registering` is logged once per placement (a later call finds nothing left to release).
  **Source:** `crates/gridops-core/src/fleet/placement.rs:418-429`
  **Verbatim excerpt:**
  > let released = sqlx::query(
  >     "UPDATE capacity_allocations SET state='released',released_at=?,updated_at=?
  >       WHERE placement_id=? AND state!='released'",
  > )
  > .bind(now)
  > .bind(now)
  > .bind(&id)
  > .execute(&mut *tx)
  > .await?
  > .rows_affected();
  > tx.commit().await?;
  > Ok(released > 0)

- **Fact:** a placement enters `registering` only when its prepare succeeds, and that write sets `updated_at` to now, so `now - updated_at` is the time spent in `registering` (handoff-A D2).
  **Source:** `crates/gridops-core/src/fleet/registry/commands.rs:348` and `:408-412`
  **Verbatim excerpt:**
  > (CommandOutcome::Succeeded { .. }, "prepare_environment") => "registering",
  >
  > sqlx::query("UPDATE workload_placements SET state=?,updated_at=? WHERE id=?")
  >     .bind(next)
  >     .bind(now)
  >     .bind(placement_id)
  >     .execute(&mut **tx)
  >     .await?;

- **Fact:** in the same pass, `finish` moves the runner row of a failed placement out of `starting` to `failed`.
  **Source:** `crates/gridops-reconciler/src/fleet.rs:712-716`
  **Verbatim excerpt:**
  > let left = sqlx::query(
  >     "UPDATE runners SET status='failed',updated_at=? WHERE id=? AND status NOT IN ('deleted','failed')",
  > )
  > .bind(now_millis())
  > .bind(&placement.workload_id)

- **Fact:** the pool pass sends every failed runner row to `delete_runner`, which is the existing upstream cleanup (D7).
  **Source:** `crates/gridops-reconciler/src/main.rs:958-961`
  **Verbatim excerpt:**
  > .filter(|runner| runner.status == "failed")
  > .collect::<Vec<_>>();
  > for runner in &failed {
  >     delete_runner(app, view, pool, runner).await?;

- **Fact:** `delete_runner` removes a GitHub registration by the recorded `github_runner_id`, or by name when no id was recorded, so a JIT that succeeded upstream before the give-up is still cleaned up.
  **Source:** `crates/gridops-reconciler/src/main.rs:3016-3023`
  **Verbatim excerpt:**
  > let github_runner_id = match runner.github_runner_id {
  >     Some(id) => Some(id),
  >     None => app
  >         .github
  >         .runner_by_name(target, &token, &runner.name)
  >         .await?
  >         .map(|runner| runner.id),
  > };
