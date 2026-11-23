# Evidence: #222 batch the retention deletes

Source facts in unchanged code that the change relies on.

- **Fact:** `cleanup_retention` is awaited with `?` in the reconcile pass, so an error it returned (as each table delete's `?` did before this change) aborts the rest of that pass; it now returns `Ok` when a table prune fails.
  **Source:** `crates/gridops-reconciler/src/main.rs:700`
  **Verbatim excerpt:**
  > `    cleanup_retention(app).await?;`

- **Fact:** `cleanup_retention` is also called (with `?`) on the path where the runner manager is unavailable, so the batched prunes run there too.
  **Source:** `crates/gridops-reconciler/src/main.rs:641-644`
  **Verbatim excerpt:**
  > `        Err(error) => {`
  > `            tracing::warn!(error = ?error, "runner manager unavailable; skipping runner reconciliation");`
  > `            cleanup_retention(app).await?;`
  > `            return Ok(());`

- **Fact:** before 0050, the only `host_samples` index containing `received_at` leads with `host_id`, so it cannot serve a `received_at < ?` range search.
  **Source:** `migrations/0027_fleet_queries.sql:21-22`
  **Verbatim excerpt:**
  > `CREATE INDEX host_samples_host_received_idx`
  > `ON host_samples(host_id,received_at,id);`

- **Fact:** the other four tables already have an index on their cutoff column (audit_events).
  **Source:** `migrations/0001_initial.sql:14`
  **Verbatim excerpt:**
  > ``CREATE INDEX `audit_events_created_idx` ON `audit_events` (`created_at`);--> statement-breakpoint``

- **Fact:** (webhook_deliveries, capacity_samples).
  **Source:** `migrations/0034_retention_indexes.sql:5-6`
  **Verbatim excerpt:**
  > `CREATE INDEX webhook_deliveries_received_idx ON webhook_deliveries(received_at);`
  > `CREATE INDEX capacity_samples_recorded_idx ON capacity_samples(recorded_at);`

- **Fact:** (reconciler_passes).
  **Source:** `migrations/0036_runner_start_timing.sql:9`
  **Verbatim excerpt:**
  > `CREATE INDEX reconciler_passes_started_idx ON reconciler_passes(started_at);`

- **Fact:** migrations (including 0050's index build) run inside `connect_database`, before the API serves.
  **Source:** `crates/gridops-core/src/db.rs:45-49`
  **Verbatim excerpt:**
  > `    adopt_legacy_drizzle_schema(&pool).await?;`
  > `    MIGRATOR`
  > `        .run(&pool)`
  > `        .await`
  > `        .context("failed to migrate GridOps database")?;`

- **Fact:** the reconciler starts only once the API is healthy, so the index build cannot race the first retention pass.
  **Source:** `compose.yaml:147-149`
  **Verbatim excerpt:**
  > `    depends_on:`
  > `      api:`
  > `        condition: service_healthy`
