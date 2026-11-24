# Evidence: #239 (unchanged source the diff relies on)

## Phase 6 (F)

- **Fact:** a machine with no score raises no `host-score` concern, so item 4 needs no change in `concerns.rs`.
  **Source:** `crates/gridops-core/src/fleet_health/concerns.rs:68-70`
  **Verbatim excerpt:**
  > pub fn host_score(host: &ScoredHost) -> Option<Concern> {
  >     let score = host.score?;
  >     let severity = band(score)?;

- **Fact:** a machine with no busy share (a CPU budget of 0) raises no `host-idle-while-queued` concern.
  **Source:** `crates/gridops-core/src/fleet_health/concerns.rs:146-147`
  **Verbatim excerpt:**
  > pub fn host_idle_while_queued(host: &ScoredHost, fleet_queue: Option<u8>) -> Option<Concern> {
  >     let busy = host.busy_share?;

- **Fact:** the scorer keeps only rows whose hour is in `[now − W, now)`, so the first row it can read is the hour `ceil_hour(now − W)`, which the covered-time denominator starts from.
  **Source:** `crates/gridops-core/src/fleet_health/score.rs:668-669`
  **Verbatim excerpt:**
  > let start = inputs.now - inputs.window.millis();
  > let in_window = |hour: i64| hour >= start && hour < inputs.now;

- **Fact:** #191's job rollup picks the newest `runners` row and that row's newest placement. `speed.rs` now copies both subqueries.
  **Source:** `crates/gridops-reconciler/src/fleet_health_rollup.rs:55-61`
  **Verbatim excerpt:**
  >   LEFT JOIN runners r ON r.id=(SELECT cand.id FROM runners cand
  >           WHERE cand.github_runner_id=wj.runner_id
  >           ORDER BY cand.created_at DESC,cand.id DESC LIMIT 1)
  >   LEFT JOIN workload_placements p ON p.id=(
  >          SELECT latest.id FROM workload_placements latest
  >           WHERE latest.runner_id=r.id
  >           ORDER BY latest.generation DESC,latest.updated_at DESC,latest.id DESC LIMIT 1)

- **Fact:** the earlier-attempt lookup's join on the run goes through `workflow_jobs_run_status_idx` on `(run_id, status)`.
  **Source:** `migrations/0001_initial.sql:238`
  **Verbatim excerpt:**
  > CREATE INDEX `workflow_jobs_run_status_idx` ON `workflow_jobs` (`run_id`,`status`);--> statement-breakpoint

- **Fact:** `fleet_health_recompute.hour` is the primary key, so `INSERT … ON CONFLICT(hour) DO NOTHING` is valid, and a listed hour that is already there is left alone.
  **Source:** `migrations/0050_fleet_health.sql:100-103`
  **Verbatim excerpt:**
  > CREATE TABLE fleet_health_recompute (
  >   hour INTEGER PRIMARY KEY NOT NULL CHECK (hour % 3600000 = 0),
  >   requested_at INTEGER NOT NULL
  > ) STRICT;

- **Fact:** a job's attempt lives in `workflow_job_sync.run_attempt`, keyed by `job_id`, and is nullable. The lookup reads an unknown attempt as 1.
  **Source:** `migrations/0043_workflow_job_attempts.sql:19-24`
  **Verbatim excerpt:**
  > CREATE TABLE workflow_job_sync (
  >   job_id INTEGER PRIMARY KEY NOT NULL REFERENCES workflow_jobs(id) ON DELETE CASCADE,
  >   run_attempt INTEGER,
  >   confirmed_at INTEGER,
  >   checked_at INTEGER
  > );

- **Fact:** the reconcile interval defaults to 30 s, which is the pass lag the design doc states for a past-grace hour that has not been written yet.
  **Source:** `crates/gridops-reconciler/src/main.rs:508-510`
  **Verbatim excerpt:**
  >         let interval = setting_i64(&reconciler.database, "reconcileIntervalSeconds", 30)
  >             .await
  >             .clamp(5, 3_600);

- **Fact:** the synthetic week has no re-runs, so the week write-transaction test does not exercise the new recompute listing. The only cost it adds there is one indexed read per hour, outside the transaction.
  **Source:** `crates/gridops-core/src/fleet_health/testdata.rs:903-905`
  **Verbatim excerpt:**
  > /// 50,000 successful GitHub jobs, each its own run at attempt 1, spread over
  > /// the four machines with a CPU budget, and a root sample every 30 s for all
  > /// five machines.
