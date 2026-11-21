## Implementation Review (Round 1)

### BLOCK findings
None.

### Needs-verification findings
**[NV-1]** docker.rs `start_gitlab_runner` — the claim that killing the attached `docker start --interactive` client (`child.start_kill()`, `kill_on_drop(true)`) after the readiness marker leaves the container running on a real Docker daemon. Detaching a killed `-i` client is a runtime hypothesis the stand-in docker cannot settle. — Evidence: Docker CLI documentation of `docker start --i` detach semantics, or a one-off run against a real daemon.

**[NV-2]** crates/gridops-core/src/provisioning.rs — the claim that `GITLAB_RUNNER_TOKEN_PREFIX` equals `glrt-` and that the manager's `starts_with` check therefore enforces D10's prefix requirement. The constant is not shown in this prompt. — Evidence: the `GITLAB_*` constants block in `provisioning.rs` at the pinned revision.

**[NV-3]** crates/gridops-manager/src/main.rs `provision_runner_inner` — the claim that GitLab **Tart** requests are forwarded to the Tart agent before `provision_runner_inner` runs, so the new `if gitlab && state.shared_docker_socket.is_none()` refusal (D10/W-6, scoped to Docker) cannot wrongly refuse a Tart GitLab start on a Docker-socket-less manager. The forwarding path is not in the prompt. — Evidence: the caller chain of `provision_runner_inner` in `main.rs` (the provider dispatch in `provision_runner`/`start_runner`).

**[NV-4]** handoff "External facts confirmed" — all gitlab-runner v19.6.0 / GitLab-API claims (flag set incl. `--docker-disable-cache` under `namespace:"docker"`, `CI_SERVER_TOKEN` env consumption in `HandleArgs`, the two `checkFinishedConditions` log lines and the formatter padding that justifies substring `grep -qF`, verify 200/403 semantics, image Ubuntu base with `bash`/`curl`). The cited upstream sources are not reproduced in this prompt, so none can be checked here. — Evidence: the cited files at the pinned versions (`commands/single.go`, `common/config.go`, `log/runner_formatter.go`, GitLab `lib/api/ci/runner.rb`, Docker Hub manifests).

**[NV-5]** PR description (Done when) — the brief's testable criterion is that the PR explicitly states the real-`glrt-`-token manual check was not run and is deferred to #143, and lists D147-1/D147-2 and the pinned image values. The PR description is not in this prompt. — Evidence: the PR body.

**[NV-6]** docker.rs `gitlab_exit_diagnostic` — the claim that `{{.State.Running}} {{.State.ExitCode}}` formats as `false <code>` with exactly one space separator on a real daemon (the parse does `split_once(' ')` on "false"/"true" literal match). The stand-in is written to match, so it proves nothing about real Docker's template output. — Evidence: Go template output of `docker inspect` on a real exited container.

### WARN findings
**[W-1]** docker.rs:318-321 (`start_gitlab`) — the retried-start shortcut returns `started(start, id)` for **any** pre-existing container with the expected name, without checking its state or exit code. If a prior attempt was interrupted after `docker create` but before readiness (agent crash mid-Start, so the `rm -f` never ran), a container that exited 77 (refused) or never became ready is reported as successfully started; the refusal then surfaces only later on Inspect. — Recommendation: on the retry path, inspect state; if exited, fall through to the failure mapping instead of returning `started`.

**[W-2]** tests (docker_gitlab.rs / gitlab_runner_script.rs) — the Done-when criterion "the token isn't visible … through the container main process's `/proc/<pid>/environ`" has no direct test: the docker tests cover create-args (inspect-visible env) and stdin, and the script tests record the stand-in curl's and runner's environments, but never assert the *script's own* (main-process) environment lacks the token. The mechanism (non-exported `S`) makes it very likely true, but it is untested. — Recommendation: add an assertion that the stand-in `env` snapshot of the bash process (or a `S` absent check) shows no token.

**[W-3]** docker.rs `start_gitlab_runner` — `docker logs` (full log, no `--since`/`--tail`) and `docker inspect` are both polled every 200 ms for up to 15 s per start; on a chatty runner this is ~150 full-log transfers per start. — Recommendation: `--tail` the marker window or poll inspect only.

### NIT findings
**[NIT-1]** provisioning.rs `GITLAB_RUNNER_SCRIPT` — the collapsed multi-space continuations (`curl --silent --show-error --output /dev/null   --write-out …`, `gitlab-runner run-single   --url …`) make the script text hard to read and diff; the test also matches `args.last() == gitlab_runner_script()`, so any reformat breaks pinned comparisons silently.

**[NIT-2]** gitlab_runner_tests.rs `Scenario::answered` — `name` is computed from the status stringly ("refused" vs anything else); a `&'static str` parameter would be clearer.

### Verdict
PASS — no BLOCK findings; testing may proceed.