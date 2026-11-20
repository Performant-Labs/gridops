# [GitLab: run gitlab-runner in Linux Docker runners (host agent)](https://github.com/Performant-Labs/gridops/issues/147)

Part of #136. **Wave 1.** Depends only on #137 (the agent protocol), merged as `b22c451`.

## Need

On Linux, the host agent must run `gitlab-runner` when it gets a `PrepareEnvironment` with a GitLab target, the way it runs the GitHub runner with `--jitconfig` today.

## Scope

In `crates/gridops-host-agent/src/docker.rs` (and `gridops-core/src/provisioning.rs` for the script):

- **Default image:** the official `gitlab/gitlab-runner`, pinned to a version and overridable by the pool's `docker_image`.
- **Command:** `gitlab-runner run-single --url <base_url> --executor docker --docker-image <default job image> --max-builds 1 --wait-timeout <idle timeout>`. The `glrt-` token is read from stdin, the way the JIT config is today, and never appears in env, args or `docker inspect`.
- **Docker access:** the Docker executor needs a Docker daemon. Use the same access the agent already gives container jobs, and document what `compose.no-docker-socket.yaml` deployments can't run.
- **Exit mapping:**
  - one job then exit → completed;
  - wait timeout → `IdleExpired`;
  - refused token → `RegistrationRefused`.

**Owns:** the GitLab branch of `docker.rs` and the GitLab runner script in `provisioning.rs`.

## Done when

- Tests show the token isn't visible through `docker inspect` or the container main process's `/proc/<pid>/environ`.
- Each exit path maps to the right outcome.
- Manual check: run the container with a real `glrt-` token from a test project. It shows online in GitLab, runs one job, and exits.

## Contract from #137 (wins over the issue text above where they differ)

The shared contract merged in #137 (`b22c451`). Its full rationale is `docs/handoffs/137-brief.md`. The deviation table posted on epic #136 replaces the issue text where they differ. The rows that apply to #147:

- **D1.** No `ExecutionConfiguration` or `PrepareEnvironment` change. The base URL travels only in the sealed bootstrap material `GitlabRunnerMaterial` (`crates/gridops-core/src/fleet/protocol/agent.rs`), JSON `{"kind":"gitlab_runner","token","url","wait_timeout_seconds"}`, a single-variant internally tagged enum with `deny_unknown_fields` and a hand-written redacting `Debug`. The host agent parses it with serde (`let GitlabRunnerMaterial::GitlabRunner { token, url, wait_timeout_seconds } = material;`). `token` goes only to the runner's stdin; `url` and `wait_timeout_seconds` are not secret and may be arguments.
- **D5.** Exit outcomes on the **fleet path** are `DiagnosticCode`s inside `CommandOutcome::Failed`, branched on `diagnostic.code`, never on the failure code alone:
  - wait timeout without a job: Inspect result `Failed { code: ResourceUnavailable, retryable: false, diagnostic: Some({ code: IdleExpired, .. }) }`;
  - GitLab refused the `glrt-` token: `Failed { code: Configuration, retryable: false, diagnostic: Some({ code: RegistrationRefused, .. }) }`;
  - one job then exit: the runner's normal completed/exited outcome, as the GitHub ephemeral runner reports today.
- **D6.** `TargetKind::{GitlabProject, GitlabGroup}` (wire `gitlab_project`, `gitlab_group`) exist. The Linux Docker host agent advertises them in `supported_runners` (`SupportedRunner { target_kind, mode }`, ephemeral mode) so GitLab work can be placed on it. Pool-target sync and admission are #139's, not #147's.
- **D9.** `ProvisioningPlan::for_selected_provider(provider, platform: CiPlatform, …)`: GitLab is always `ephemeral`, runtime `vm`, image `docker_image` for the Docker provider. #147 does not change this function; choosing `CiPlatform::Gitlab` in its callers is #139's.
- **D10 (#147 owns the manager arm).** The built-in Docker host (`crates/gridops-manager`) gets GitLab through `POST /v1/runners` with `"platform":"gitlab"`, `"mode":"ephemeral"`, `"runtime":"vm"`, `gitlab: {url, waitTimeoutSeconds}` and the secret `gitlabRunnerToken` (no `jitConfig`, `registrationToken`, `bitbucket` or `bitbucketOauthClientSecret`). #147 replaces the manager's placeholder `("gitlab", _)` validation arm (today it returns `BadRequest("GitLab runners are not supported yet: Docker is #147, Tart is #142.")`):
  - `provider: "docker"`: validate the request (both fields present, mode ephemeral, token has the `GITLAB_RUNNER_TOKEN_PREFIX` `glrt-` prefix, URL is an http(s) URL, no GitHub/Bitbucket secrets) and launch `gitlab-runner` in a container with the **same runner script as the fleet host agent** (one shared script in `gridops-core/src/provisioning.rs`), the token on stdin the way the manager already delivers `jitConfig` (attach/open stdin), never in env, args, labels or logs.
  - `provider: "tart"`: validate the same way and forward to the Tart agent with both fields intact, as Bitbucket requests are forwarded today. The Tart agent's own GitLab support is #142; until it lands, the Tart agent refuses with its own message. #147 does not edit `crates/gridops-tart-agent`.
  - `gitlabRunnerToken` is never logged and never appears in an error message.
- **Exit reporting on the manager path (D10, R2-1 item 3 of the #137 brief).** `GET /v1/runners` entries gain an optional `exitReason`, present only for a GitLab runner whose `state` is `"exited"`, with one of `"completed"`, `"idle_expired"`, `"registration_refused"`. The value comes from the container's exit status looked up in `gridops_core::provisioning::GITLAB_EXIT_REASONS` (`[(0,"completed"),(75,"idle_expired"),(77,"registration_refused")]`), using the core constants directly, no copy. The runner script exits with exactly those codes (`GITLAB_EXIT_COMPLETED`, `GITLAB_EXIT_IDLE_EXPIRED`, `GITLAB_EXIT_REGISTRATION_REFUSED`). An exit status not in the table yields no `exitReason` (behaviour not pinned by the contract; keep it absent).
- **D12.** The idle timeout is `wait_timeout_seconds` / `waitTimeoutSeconds` on both paths; default `GITLAB_DEFAULT_WAIT_TIMEOUT_SECS = 600` is the reconciler's (#139) to send.
- **D15.** The manager's private `ManagedRunner` gains `#[serde(default, skip_serializing_if = "Option::is_none")] exit_reason: Option<String>` (wire `exitReason`), so a Tart agent's `exitReason` survives the manager's re-parse in `tart_runners`. Tests: a Tart entry with `exitReason` survives the re-parse and re-serialization; one without it serializes with no `exitReason` key.
- D13, D14 and D16 do not affect #147 (jobs index, reconciler-owned starts, migration number).

## Boundaries

- Owned files: the GitLab branch of `crates/gridops-host-agent/src/docker.rs` (plus whatever host-agent wiring is needed to advertise GitLab and route its material there), the GitLab runner script in `crates/gridops-core/src/provisioning.rs`, and the GitLab arm in `crates/gridops-manager/src/main.rs`.
- Do **not** change shared contract types (`GitlabRunnerMaterial`, `DiagnosticCode`, `TargetKind`, `ProvisionRunner`'s `gitlab`/`gitlabRunnerToken` shapes, `GitlabBootstrap`, the `GITLAB_*` constants, `for_selected_provider`). If the contract truly needs a change, stop and report it.
- Do not edit `crates/gridops-tart-agent` (#142), the reconciler (#139), the API (#138), the GitLab client (#146), webhooks (#140), scaling (#148) or the UI (#141).
- Docker access: the GitLab Docker executor needs a Docker daemon. Use the access the agent already gives container jobs that require the Docker socket (`requires_docker_socket`), and document in the docs (for example `docs/fleet-agent.md` and the `compose.no-docker-socket.yaml` header comment) that a deployment without the Docker socket can't run GitLab runners. If the socket is unavailable, refuse the GitLab start with a clear configuration failure rather than starting a runner that can't run jobs.
- The issue's manual check (a real `glrt-` token against a real GitLab project) **cannot be run in this issue**: no test project or token exists yet. It is **not run, deferred to #143** (the end-to-end test against real GitLab). Do not fake it; the PR says so.
