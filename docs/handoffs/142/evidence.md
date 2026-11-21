# Evidence: #142 GitLab macOS runners on Tart pools (Tart agent side)

Source facts in unchanged code (and upstream code) that the change, or T's tests, rely on. Each excerpt is copied verbatim from the source by the author named in the section heading.

## T (Phase 4, RED)

### This repository (at 229081c)

- **Fact:** the guest-readiness probe runs `tart exec` **without** `-i`, so the fake `tart`'s `-i` branch (scripted status, stdin capture) only ever answers the runner start.
  **Source:** `crates/gridops-tart-agent/src/main.rs:1726`
  **Verbatim excerpt:**
  > .args(["exec", record.vm_name.as_str(), "/usr/bin/true"])

- **Fact:** `delete_runner` stops and deletes the VM first, and only then removes the in-memory record. So in the delete-race test, the runner's `tart exec` ends while the delete is still running.
  **Source:** `crates/gridops-tart-agent/src/main.rs:1295` and `:1297`; `cleanup_vm` at `:2187-2188`
  **Verbatim excerpt:**
  > cleanup_vm(&state, &record.vm_name).await;

  > state.records.write().await.remove(&runner_id);

  > let _ = tart_status(&state.tart, ["stop", vm_name]).await;
  > let _ = tart_status(&state.tart, ["delete", vm_name]).await;

- **Fact:** `FakeTart`'s agent home is `<fake dir>/home`, which is where the fake `stop` reads `home/records/*.json` for its `records-at-stop` snapshot.
  **Source:** `crates/gridops-tart-agent/src/main.rs:2915` (tests module)
  **Verbatim excerpt:**
  > let home = dir.join("home");

- **Fact:** `load_records` keeps only records whose id passes `valid_id` (`tart-` + 32 hex digits), so the record-reload test uses ids of that shape.
  **Source:** `crates/gridops-tart-agent/src/main.rs:2214`
  **Verbatim excerpt:**
  > if valid_id(&record.id) {

- **Fact:** a VM Tart reports as `stopped` is listed with state `exited`. That state is the only one in which `exitReason` may appear.
  **Source:** `crates/gridops-tart-agent/src/main.rs:734`
  **Verbatim excerpt:**
  > "stopped" => "exited",

### Upstream `gitlab-runner`, commit `e2320620bae7319498c63f7b2b05e52afe99393e` (read for the fake `gitlab-runner`)

- **Fact:** `run-single` counts a build against `--max-builds` after the build runs, whatever its result: `postBuild` runs before `processBuild` returns the job's error.
  **Source:** `commands/single.go:160-164`
  **Verbatim excerpt:**
  > err = newBuild.Run(ctx, config, trace)
  >
  > r.postBuild()
  >
  > return err

- **Fact:** `run-single` ends its loop after one build with a "processed its build limit" log, and after the wait timeout with a "not received a job" log. `Execute` then returns normally, so the process exits 0 in both cases.
  **Source:** `commands/single.go:169` and `:173`
  **Verbatim excerpt:**
  > logrus.Println("This runner has processed its build limit, so now exiting")

  > logrus.Println("This runner has not received a job in", r.WaitTimeout, "seconds, so now exiting")

- **Fact:** a 403 on the job request (a refused token) is logged as "Checking for jobs... forbidden" and reported as unhealthy. `run-single` does not exit; it keeps polling until the wait timeout.
  **Source:** `network/gitlab.go:711-713`
  **Verbatim excerpt:**
  > case http.StatusForbidden:
  > 	logger.WithField("status", statusText).Errorln("Checking for jobs...", "forbidden")
  > 	return nil, false

- **Fact:** the runner token can be given as the `CI_SERVER_TOKEN` environment variable, and the builds directory as `--builds-dir` or `RUNNER_BUILDS_DIR`.
  **Source:** `common/config.go:1466` and `:1506`
  **Verbatim excerpt:**
  > Token           string    `toml:"token" json:"token" short:"t" long:"token" env:"CI_SERVER_TOKEN" required:"true" description:"Runner token" jsonschema:"minLength=1"`

  > BuildsDir string `toml:"builds_dir,omitempty" json:"builds_dir" long:"builds-dir" env:"RUNNER_BUILDS_DIR" description:"Directory where builds are stored"`

## F (Phase 5, implementation)

### This repository (working tree after F's change; these lines are unchanged code)

- **Fact:** `delete_runner` removes the in-memory record before it removes the record file, and both after `cleanup_vm`. So when `record_exit_code` (which persists under the records write lock, only if the entry is still in the map) races a delete, either it writes first and the delete then removes both copies, or it finds no entry and writes nothing.
  **Source:** `crates/gridops-tart-agent/src/main.rs:1342` and `:1344`
  **Verbatim excerpt:**
  > cleanup_vm(&state, &record.vm_name).await;

  > state.records.write().await.remove(&runner_id);

- **Fact:** `persist_record` writes a temporary file and renames it over the record, so the exit-code write never leaves a half-written record file.
  **Source:** `crates/gridops-tart-agent/src/main.rs:2301` and `:2303`
  **Verbatim excerpt:**
  > let temporary = path.with_extension("json.tmp");

  > fs::rename(temporary, path).await?;

- **Fact:** `start_runner` delivers the bootstrap secret (for GitLab, the token) by writing it to `tart exec -i`'s stdin. It is not on the `tart exec` argv, which carries only `/bin/bash -lc <script>`.
  **Source:** `crates/gridops-tart-agent/src/main.rs:1811` and `:1823`
  **Verbatim excerpt:**
  > "-lc",

  > .write_all(bootstrap_secret.expose_secret().as_bytes())

- **Fact:** a failed VM provisioning logs the `anyhow` error with `Debug` formatting, and `AgentError::Internal` does the same. So no error built on the GitLab path may embed the token. The GitLab arm's only errors are `.context("validated GitLab ... disappeared")` strings and the shared clone, boot and start errors, none of which formats the request.
  **Source:** `crates/gridops-tart-agent/src/main.rs:920` and `:498`
  **Verbatim excerpt:**
  > tracing::error!(runner_id = %id, vm = %vm_name, error = ?error, "Tart runner provisioning failed");

  > tracing::error!(error = ?error, "Tart agent request failed");

- **Fact:** the listing maps a VM Tart reports as `stopped` to `exited`. That is the only state in which `list_runners` adds `exitReason`.
  **Source:** `crates/gridops-tart-agent/src/main.rs:752`
  **Verbatim excerpt:**
  > "stopped" => "exited",

### Upstream Tart 2.41.1 (`cirruslabs/tart` at `543453404cd1b15c1338b6b70e162d000c8ba5e2`; NV-1)

- **Fact:** `tart exec` turns the guest command's exit event into an error that carries the guest's exit code, and the top-level handler exits the `tart` process with that code. So the agent's `child.wait()` on `tart exec -i` sees the guest script's 0, 75, 77 or 78.
  **Source:** `Sources/tart/Commands/Exec.swift:195-196`; `Sources/tart/Root.swift:164-168`
  **Verbatim excerpt:**
  > case .exit(let exit):
  >   throw ExecCustomExitCodeError(exitCode: exit.code)

  > // Not an error, just a custom exit code from "tart exec"
  > if let execCustomExitCodeError = error as? ExecCustomExitCodeError {
  >   span.end()
  >   OTel.shared.flush()
  >   Foundation.exit(execCustomExitCodeError.exitCode)

### Upstream gitlab-runner v19.5.3, the release the image script pins (`gitlab-org/gitlab-runner` tag `v19.5.3`; NV-2, NV-3)

- **Fact:** the runner's own verify call treats 200 as a valid token and 403 as an invalid one. Any other status is an error, not a refusal. The script maps 200 to "go on", 403 to 77, and anything else to 69.
  **Source:** `network/gitlab.go:451-466`
  **Verbatim excerpt:**
  > switch result {
  > case http.StatusOK:
  > 	// this is expected due to fact that we ask for non-existing job
  > 	if TokenIsCreatedRunnerToken(runner.Token) {
  > 		logger.Println("Verifying runner...", "is valid")
  > 	} else {
  > 		logger.Println("Verifying runner...", "is alive")
  > 	}
  > 	return &response, nil
  > case http.StatusForbidden:
  > 	if TokenIsCreatedRunnerToken(runner.Token) {
  > 		logger.Println("Verifying runner...", "is not valid")
  > 	} else {
  > 		logger.WithField("status", statusText).Errorln("Verifying runner...", "is removed")
  > 	}
  > 	return nil, nil

- **Fact:** the runner's API base is `<url>/api/v4/`, so its verify call is `POST <url>/api/v4/runners/verify`, the endpoint the script calls.
  **Source:** `network/client.go:636`; `network/gitlab.go:426`
  **Verbatim excerpt:**
  > u, err := url.Parse(fixCIURL(base) + "/api/v4/")

  > "runners/verify",

- **Fact:** a job the runner receives is logged as "Checking for jobs... received", and a 403 on the job request as "Checking for jobs... forbidden". The script matches these strings, together with `single.go`'s two exit lines (cited by T above at commit `e2320620`, and present at `commands/single.go:168` and `:172` in v19.5.3).
  **Source:** `network/gitlab.go:690` and `:701-702`
  **Verbatim excerpt:**
  > }).Println("Checking for jobs...", "received")

  > case http.StatusForbidden:
  > 	logger.WithField("status", statusText).Errorln("Checking for jobs...", "forbidden")

- **Fact:** after an unhealthy job request (for example the 403 above), `run-single` sleeps `NotHealthyCheckInterval` seconds before it checks its finish conditions, and that interval is 300. So a token refused during `run-single` surfaces as exit 77 only after about 300 seconds, or after the wait timeout if that is longer.
  **Source:** `commands/single.go:87-89`; `common/consts.go:14`
  **Verbatim excerpt:**
  > logrus.Println("Runner is not healthy!")

  > case <-time.After(common.NotHealthyCheckInterval * time.Second):

  > const NotHealthyCheckInterval = 300

- **Fact:** `run-single` takes `--url`, `--executor`, `--builds-dir`, `--wait-timeout` and `--max-builds`, and reads the token from `CI_SERVER_TOKEN`.
  **Source:** `common/config.go:1417`, `:1421`, `:1456-1457`; `commands/single.go:24` and `:27`
  **Verbatim excerpt:**
  > URL             string    `toml:"url" json:"url" short:"u" long:"url" env:"CI_SERVER_URL" required:"true" description:"GitLab instance URL" jsonschema:"minLength=1"`

  > Token           string    `toml:"token" json:"token" short:"t" long:"token" env:"CI_SERVER_TOKEN" required:"true" description:"Runner token" jsonschema:"minLength=1"`

  > Executor  string `toml:"executor" json:"executor" long:"executor" env:"RUNNER_EXECUTOR" required:"true" description:"Select executor, eg. shell, docker, etc."`
  > BuildsDir string `toml:"builds_dir,omitempty" json:"builds_dir" long:"builds-dir" env:"RUNNER_BUILDS_DIR" description:"Directory where builds are stored"`

  > WaitTimeout       int `long:"wait-timeout" description:"How long to wait in seconds before receiving the first job"`

  > MaxBuilds         int `long:"max-builds" description:"How many builds to process before exiting"`
