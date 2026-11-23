# Evidence: #197 (F, Phase 5)

Source facts in unchanged code that the diff and T's tests rely on.

- **Fact:** the job-category handlers' admin check is the session routes' existing one: a non-admin gets `ApiError::Forbidden` (403).
  **Source:** `crates/gridops-api/src/auth.rs:185-191`
  **Verbatim excerpt:**
  > pub fn require_system_admin(user: &AuthUser) -> ApiResult<()> {
  >     if user.role == "admin" {
  >         Ok(())
  >     } else {
  >         Err(ApiError::Forbidden)
  >     }
  > }

- **Fact:** every route in `session_routes()` (where the three job-category routes are registered) refuses a bearer token before the handler runs.
  **Source:** `crates/gridops-api/src/main.rs:416`
  **Verbatim excerpt:**
  > .route_layer(middleware::from_fn(access::refuse_bearer))

- **Fact:** the health state's default clock is the registry's real-time clock.
  **Source:** `crates/gridops-core/src/fleet/registry/mod.rs:78-80`
  **Verbatim excerpt:**
  > pub fn system_clock() -> std::sync::Arc<dyn RegistryClock> {
  >     std::sync::Arc::new(SystemClock)
  > }

- **Fact:** the Overview's queued-jobs clause, which `fleet_queued_jobs!` repeats without the `user_installations` join and its user binding.
  **Source:** `crates/gridops-api/src/resources.rs:80-88`
  **Verbatim excerpt:**
  > macro_rules! visible_queued_jobs {
  >     () => {
  >         "FROM workflow_jobs wj
  >           JOIN workflow_runs wr ON wr.id=wj.run_id
  >           JOIN repositories repo ON repo.id=wr.repository_id
  >           JOIN user_installations visible ON visible.installation_id=repo.installation_id
  >           WHERE visible.user_id=? AND wj.status IN ('queued','waiting','pending','requested') AND wr.status!='completed'"
  >     };
  > }

- **Fact:** `RequestContext::error` takes the context by value and is `const`, so the health handler's error closure copies it (`RequestContext` is `Copy`, as `require_admin(&state, &context, …)` followed by `context.error(…)` already relies on).
  **Source:** `crates/gridops-api/src/fleet/error.rs:29-35`
  **Verbatim excerpt:**
  > pub(super) const fn error(self, code: ErrorCode) -> FleetApiError {
  >     FleetApiError {
  >         code,
  >         context: self,
  >         details: None,
  >     }
  > }

- **Fact:** the speed check builds on the existing slow-statement log: every pool logs a statement of 100 ms or more at WARN.
  **Source:** `crates/gridops-core/src/db.rs:20` and `:40`
  **Verbatim excerpt:**
  > const SLOW_STATEMENT: Duration = Duration::from_millis(100);
  >
  > .log_slow_statements(log::LevelFilter::Warn, SLOW_STATEMENT);

- **Fact:** inserting a runner pool already maps it to its installation, so `testdata::seed_fleet` uses `INSERT OR IGNORE INTO runner_pool_installations`.
  **Source:** `migrations/0014_runner_pool_installations.sql:19-24`
  **Verbatim excerpt:**
  > CREATE TRIGGER `runner_pool_installation_after_pool_insert`
  > AFTER INSERT ON `runner_pools`
  > BEGIN
  > 	INSERT OR IGNORE INTO `runner_pool_installations` (`pool_id`,`installation_id`)
  > 	VALUES (NEW.`id`,NEW.`installation_id`);
  > END;

- **Fact:** an existing Overview test renders the route with no `QueryClientProvider`, which is why `useFleetHealth` keeps its own state (the workload chart's pattern) instead of using react-query.
  **Source:** `tests/dashboard-pool-scope.test.ts:89`
  **Verbatim excerpt:**
  > act(() => root.render(createElement(TooltipProvider, null, createElement(page))));

- **Fact:** the GitLab webhook test payload carries `build_name: "rspec"`, so a later `pending` event fills the name of a row first written without one (the name fill W-16 requires), which bumps `updated_at` (see "Tests that look wrong" in `handoff-F.md`).
  **Source:** `crates/gridops-api/src/webhooks/gitlab_tests.rs:163-170`
  **Verbatim excerpt:**
  > fn job_event(job_id: u64, status: &str, at: Option<i64>) -> Value {
  >     json!({
  >         "object_kind": "build",
  >         "ref": "main",
  >         "tag": false,
  >         "retries_count": 0,
  >         "build_id": job_id,
  >         "build_name": "rspec",
