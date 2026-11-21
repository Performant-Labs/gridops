# Evidence: #173 (F, Phase 5)

Source facts the diff relies on that live in unchanged code.

- **Fact:** `left_service` runs at most once per runner on the manager path, because `runner_exited` calls it only when its guarded UPDATE took the row out of service; this is what makes a `runner_missing` exit count once across repeated reconcile passes.
  **Source:** `crates/gridops-reconciler/src/gitlab_lifecycle.rs:387-399`
  **Verbatim excerpt:**
  > let left = sqlx::query(
  >     "UPDATE runners SET status='failed',failure_reason=NULL,updated_at=? WHERE id=? AND status NOT IN ('failed','deleted')",
  > )
  > ...
  > if !left {
  >     return Ok(false);
  > }
  > left_service(

- **Fact:** counting a provisioning failure raises the pool's count by one from its current value (it does not reset), and sets a retry time (the backoff).
  **Source:** `crates/gridops-reconciler/src/main.rs:3795-3798`
  **Verbatim excerpt:**
  > async fn record_provision_failure(database: &SqlitePool, pool: &Pool) -> Result<()> {
  >     let attempts = pool.provision_failure_count.saturating_add(1);
  >     let (circuit_open, delay_ms) = provision_failure_policy(attempts);
  >     let retry_at = now_millis().saturating_add(delay_ms);

- **Fact:** the reconciler turns the manager's `exitReason` into an `Exit` through `from_exit_reason` and hands it to `runner_exited`, so adding `runner_missing` to the table and to `from_exit_reason` is enough to route a Tart exit 78 into `left_service`.
  **Source:** `crates/gridops-reconciler/src/main.rs:914-917`
  **Verbatim excerpt:**
  > if runner.ci_platform == "gitlab" && status == "failed" {
  >     let exit = gitlab_lifecycle::Exit::from_exit_reason(view.exit_reason(container_id));
  >     circuit_changed |=
  >         gitlab_lifecycle::runner_exited(app, &pool.id, runner, exit).await?;

- **Fact:** the Tart agent's runner listing names `exitReason` from `gitlab::exit_reason` (the table), only for an exited GitLab runner; extending the table is what makes a 78 exit list as `runner_missing`.
  **Source:** `crates/gridops-tart-agent/src/main.rs:780-785`
  **Verbatim excerpt:**
  > if runtime_state == "exited"
  >     && record.platform.as_deref() == Some("gitlab")
  >     && let Some(reason) = record.exit_code.and_then(gitlab::exit_reason)
  > {
  >     runner["exitReason"] = reason.into();
  > }

- **Fact:** the manager maps a Docker GitLab container's exit code through core's `GITLAB_EXIT_REASONS`, which is why core's own runner script must keep `run-single`'s 78 out of the table (the `@EXIT_RUNNER_MISSING@` guard).
  **Source:** `crates/gridops-manager/src/main.rs:1630-1639`
  **Verbatim excerpt:**
  > let status = docker
  >     .inspect_container(id, None)
  >     .await
  >     .ok()?
  >     .state?
  >     .exit_code?;
  > GITLAB_EXIT_REASONS
  >     .iter()
  >     .find(|(code, _)| i64::from(*code) == status)
  >     .map(|(_, reason)| (*reason).to_owned())

- **Fact:** no fleet host-agent runtime runs Tart (anything other than Docker or a native process is `Unsupported`), so `Exit::from_inspect` needs no `runner_missing` mapping.
  **Source:** `crates/gridops-host-agent/src/runtime.rs:52-58`
  **Verbatim excerpt:**
  > Some(RuntimeKind::Docker) => self.docker.call(request).await,
  > Some(RuntimeKind::NativeProcess) => self.native.call(request).await,
  > _ => HelperResponse::Failed {
  >     code: FailureCode::Unsupported,
  >     retryable: false,
  >     diagnostic: None,
  > },

- **Fact:** a Tart agent test reads the GitLab prep script itself and requires the default runner path and `GRIDOPS_TART_GITLAB_RUNNER_BINARY` in it, so both stay in the entry script (no sourced helper).
  **Source:** `crates/gridops-tart-agent/src/gitlab_tests.rs:204-206`
  **Verbatim excerpt:**
  > let image_script = include_str!("../../../scripts/prepare-tart-gitlab-runner-image.sh");
  > assert!(image_script.contains(DEFAULT_GITLAB_RUNNER_BINARY));
  > assert!(image_script.contains("GRIDOPS_TART_GITLAB_RUNNER_BINARY"));

- **Fact:** the runners list shows a runner's `failureReason`, so the `runner_missing` reason written to the runner row is visible the same way `registration_refused`'s is.
  **Source:** `src/features/runners/runner-row.tsx:128`
  **Verbatim excerpt:**
  > {runner.failureReason ? <span className="hidden min-w-0 max-w-72 truncate text-xs text-danger lg:block" title={runner.failureReason}>{runner.failureReason}</span> : null}
