# Evidence: #222 batch the retention deletes

Source facts in unchanged code that the change relies on.

- **Fact:** `cleanup_retention` is awaited with `?` in the reconcile pass, so an error it returned (as each table delete's `?` did before this change) aborts the rest of that pass; it now returns `Ok` when a table prune fails.
  **Source:** `crates/gridops-reconciler/src/main.rs:737`
  **Verbatim excerpt:**
  > `    cleanup_retention(app).await?;`

- **Fact:** `cleanup_retention` is also called (with `?`) on the path where the runner manager is unavailable, so the batched prunes run there too.
  **Source:** `crates/gridops-reconciler/src/main.rs:678-681`
  **Verbatim excerpt:**
  > `        Err(error) => {`
  > `            tracing::warn!(error = ?error, "runner manager unavailable; skipping runner reconciliation");`
  > `            cleanup_retention(app).await?;`
  > `            return Ok(());`

- **Fact:** before 0051, the only `host_samples` index containing `received_at` leads with `host_id`, so it cannot serve a `received_at < ?` range search.
  **Source:** `migrations/0027_fleet_queries.sql:21-22`
  **Verbatim excerpt:**
  > `CREATE INDEX host_samples_host_received_idx`
  > `ON host_samples(host_id,received_at,id);`

- **Fact:** four of the other seven tables already have a plain index on their cutoff column (audit_events).
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

- **Fact:** migrations (including 0051's index build) run inside `connect_database`, before the API serves.
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

- **Fact:** `fleet_health_hourly` is a rowid (`STRICT`, not `WITHOUT ROWID`) table whose primary key leads with `hour`, so its autoindex `sqlite_autoindex_fleet_health_hourly_1` serves the batch's `hour < ? ORDER BY hour` search and `rowid IN (...)` is valid. No migration declares `WITHOUT ROWID` (`grep -rn "WITHOUT ROWID" migrations` is empty).
  **Source:** `migrations/0050_fleet_health.sql:75-76`
  **Verbatim excerpt:**
  > `  PRIMARY KEY (hour,host_id,pool_id,category)`
  > `) STRICT;`

- **Fact:** `fleet_host_hourly`'s primary key leads with `hour` (rowid table, `STRICT`).
  **Source:** `migrations/0050_fleet_health.sql:95-96`
  **Verbatim excerpt:**
  > `  PRIMARY KEY (hour,host_id)`
  > `) STRICT;`

- **Fact:** `fleet_health_snapshots`'s primary key leads with `recorded_at` (rowid table, `STRICT`).
  **Source:** `migrations/0050_fleet_health.sql:113-114`
  **Verbatim excerpt:**
  > `  PRIMARY KEY (recorded_at,"window")`
  > `) STRICT;`

- **Fact:** the fleet health window that the new cutoff arm uses is the unchanged 90-day constant #221 introduced.
  **Source:** `crates/gridops-reconciler/src/main.rs:3357-3358`
  **Verbatim excerpt:**
  > `/// How long the fleet health rollups and snapshots are kept (#197).`
  > `const FLEET_HEALTH_RETENTION_DAYS: i64 = 90;`
