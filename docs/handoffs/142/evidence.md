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
