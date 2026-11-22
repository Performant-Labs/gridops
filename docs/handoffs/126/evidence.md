# Evidence: #126 (unchanged source the diff relies on)

## F (Phase 6)

- **Fact:** the shared runner script forwards the live job log from `${GRIDOPS_RUNNER_ROOT:-$PWD}/_diag/pages`, so setting `GRIDOPS_RUNNER_ROOT` to the runner root keeps live logs working once the runner runs from there.
  **Source:** `crates/gridops-core/src/provisioning.rs:328-334`
  **Verbatim excerpt:**
  > runner_root=${{GRIDOPS_RUNNER_ROOT:-$PWD}}
  > forwarded_state=/tmp/gridops-forwarded-job-state
  > rm -rf "$forwarded_state"
  > mkdir -p "$forwarded_state"
  >
  > flush_job_logs() {{
  >   for job_log in "$runner_root"/_diag/pages/*.log; do

- **Fact:** the `launch` part of the shared runner script runs in a background subshell. So the copy, the link and the `cd` have to go in the setup part (the main shell), as the diff puts them.
  **Source:** `crates/gridops-core/src/provisioning.rs:319`
  **Verbatim excerpt:**
  > {launch} >"$runner_diagnostics" 2>&1 &

- **Fact:** the sweep treats a cache as stale only when plain `docker inspect` fails with "no such object". `missing()` recognises that error, and not `docker image inspect`'s "No such image".
  **Source:** `crates/gridops-host-agent/src/docker.rs:1390-1393`
  **Verbatim excerpt:**
  > fn missing(error: &anyhow::Error) -> bool {
  >     let message = error.to_string().to_ascii_lowercase();
  >     message.contains("no such object") || message.contains("no such container")
  > }

- **Fact:** a failed `docker` call's error text includes the CLI's stderr. The fill reads "Could not find the file" in it to tell an image without `externals` (`Configuration, false`) from any other `docker cp` failure (`Runtime, true`).
  **Source:** `crates/gridops-host-agent/src/docker.rs:1380-1386`
  **Verbatim excerpt:**
  > anyhow::ensure!(
  >     output.status.success(),
  >     "`docker {}` failed: {}",
  >     args.first().unwrap_or(&""),
  >     String::from_utf8_lossy(&output.stderr).trim()
  > );
  > Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())

- **Fact:** `feed_secret` detaches 500 ms after it writes the secret. So the per-start copy goes after `IFS= read -r S` (the script has already read the line), never before it.
  **Source:** `crates/gridops-host-agent/src/docker.rs:641-643`
  **Verbatim excerpt:**
  > // Give the script a moment to read the line before detaching.
  > tokio::time::sleep(Duration::from_millis(500)).await;
  > drop(stdin);

- **Fact:** a retried Start returns the container it already created before the socket branch runs. So the image-id resolve, the cache check and the folder creation happen only for a container that does not exist yet.
  **Source:** `crates/gridops-host-agent/src/docker.rs:155-158`
  **Verbatim excerpt:**
  > // A retried start finds the container it already created.
  > if let Ok(id) = docker(&["inspect", "-f", "{{.Id}}", &name]).await {
  >     return started(start, id);
  > }

- **Fact:** GitLab Starts leave `start` before the GitHub socket branch. So `uses_externals_cache` (`docker_socket && !is_gitlab`) is the same condition the socket branch was reached on before.
  **Source:** `crates/gridops-host-agent/src/docker.rs:146-149`
  **Verbatim excerpt:**
  > if prepared.target.is_gitlab() {
  >     return self
  >         .start_gitlab(request, environment, prepared, material, start)
  >         .await;

- **Fact:** a fix sandbox is always a Docker/Linux `GithubRepository` prepare. So the shared predicate counts it as a GitHub runner, as the truth-table test expects.
  **Source:** `crates/gridops-core/src/fleet/protocol/agent.rs:274-277`
  **Verbatim excerpt:**
  > || (prepare.workload == PreparedWorkload::FixSandbox
  >     && (prepare.runtime_kind != RuntimeKind::Docker
  >         || prepare.execution_os != ExecutionOs::Linux
  >         || !matches!(prepare.target, CiTarget::GithubRepository { .. })))

- **Fact:** every runner container is created with an empty entrypoint. The fill container uses the same `--entrypoint ""` so that an image with no `Cmd` can still be created.
  **Source:** `crates/gridops-host-agent/src/docker.rs:274-275`
  **Verbatim excerpt:**
  > "--entrypoint".into(),
  > String::new(),

- **Fact:** the analogous `NativeRuntime` fill renames its staged tree with `tokio::fs::rename` and walks a tree under `spawn_blocking`, turning the `JoinError` into an `io::Error` with `?`. The async `remove_tree` uses the same pattern.
  **Source:** `crates/gridops-host-agent/src/native.rs:529` and `crates/gridops-host-agent/src/native.rs:551`
  **Verbatim excerpt:**
  > tokio::fs::rename(&staging, target).await?;

  > tokio::task::spawn_blocking(move || copy(&from, &to)).await??;
