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

## Amendments (architecture review R1, `docs/handoffs/147/handoff-A.md`)

Each finding is resolved here. Where this section and the text above differ, this section wins.

### B-1: which image is which (deviation D147-1)

- **The pool's `docker_image` is the gitlab-runner (runner) image** on both paths, exactly as the server already chooses images today: the fleet agent runs `PrepareEnvironment.image`, the manager runs `input.image`. **Neither `docker.rs` nor the manager's `main.rs` holds an image constant or picks a fallback image.**
- #147 adds two pinned constants next to the other `GITLAB_*` constants in `crates/gridops-core/src/provisioning.rs` (additive; no existing contract item changes):
  - `pub const GITLAB_RUNNER_IMAGE: &str = "gitlab/gitlab-runner:<pinned ubuntu-based tag>";` — the official image, pinned to an exact version tag (not `latest`), Ubuntu-based so the script's tools (`bash`, `curl`) exist. The implementor confirms the tag exists on Docker Hub headlessly (registry/tags API) and records the evidence.
  - `pub const GITLAB_DEFAULT_JOB_IMAGE: &str = "<pinned image>";` — the `--docker-image` default job image for jobs whose `.gitlab-ci.yml` names none, pinned to an exact tag (for example a specific `alpine:3.x` or `ubuntu:24.04`), confirmed to exist the same way.
  - A test pins both values and that neither ends in `:latest`.
- Applying `GITLAB_RUNNER_IMAGE` as the default when a GitLab pool has no runner image of its own (today an empty `docker_image` becomes `STOCK_DOCKER_IMAGE`, the GitHub image, in `fleet/targets.rs::pool_docker_image`) is the **server's** job: #139 (profile and provisioning-plan image) and #138/#141 (pool form). #147 records this as deviation **D147-1** in the PR and on #136, and leaves a note for #139 in the shared notes folder. #147 does not edit `fleet/targets.rs`.
- `--docker-image` is always `GITLAB_DEFAULT_JOB_IMAGE` on both paths.

### B-2: `RegistrationRefused` is the **Start** result; exit semantics per command

- The GitLab runner script (B-8 below) first verifies the token against GitLab's documented `POST {url}/api/v4/runners/verify` (200: valid; 403: refused), sending the token as form data from stdin (`printf 'token=%s' "$S" | curl ... --data @- ...`; `printf` is a shell builtin, so the token is never in any process's argv or environment). On 403 it exits `GITLAB_EXIT_REGISTRATION_REFUSED` (77). Any other verify failure (network, 5xx) exits with a distinct non-table code so it is a retryable runtime failure, not a refusal. After a successful verify it prints a fixed readiness marker line to stdout (pinned as a constant in `provisioning.rs`) and then runs `gitlab-runner run-single ...` with the token passed through a mechanism that keeps it out of argv and env of the container's **main process** (the implementor confirms from the pinned gitlab-runner version's source/docs how `run-single` takes its token — e.g. its flag/env var set only on the `exec`'d child is **not** acceptable for the main process if the child becomes PID 1; document the exact mechanism and test what `docker inspect` and `/proc/1/environ` show).
- Distinguishing completed from idle expiry: the script maps `run-single`'s outcome to exit 0 (one job received and run) or 75 (wait timeout with no job). The implementor determines, from the pinned gitlab-runner version's source, the exact signal that a job was received (log line or exit status), pins it in a constant, and cites the source location in the handoff.
- **Fleet Start** (`docker.rs`, GitLab branch): after writing the token on stdin, Start waits (bounded, a pinned timeout such as 60 s) until either the readiness marker appears in the container's output (→ `started`, as today) or the container exits. An exit with 77 during Start → `Failed { code: Configuration, retryable: false, diagnostic: Some(RegistrationRefused) }`. Any other exit or the timeout during Start → today's `Failed { Runtime, retryable: true }` behaviour.
- **Fleet Inspect** of an exited GitLab container: always `Failed { code: ResourceUnavailable, retryable: false, .. }` so `placement::reported_gone` cleans it up, with `diagnostic: Some(IdleExpired)` for exit 75, `Some(RegistrationRefused)` for a late exit 77, and `None` for exit 0 (completed) and any other code. A running GitLab container inspects exactly as a running GitHub runner does today.
- For #139 (recorded in the PR and in the shared notes): on the fleet path branch on `diagnostic.code` of the **Start** result for `RegistrationRefused` and of the **Inspect** result for `IdleExpired` (and a late `RegistrationRefused`); on the manager path branch on `exitReason`.

### B-3: the diagnostic channel from helper to agent

- **Owns, added:** `crates/gridops-host-agent/src/helper.rs`, `crates/gridops-host-agent/src/dispatch.rs`, and the `HelperResponse::Failed` constructors in `native.rs` and `runtime.rs` (mechanical: they pass `diagnostic: None`).
- `HelperResponse::Failed` gains `#[serde(default, skip_serializing_if = "Option::is_none")] diagnostic: Option<DiagnosticReference>`. `HelperResponse` is private to the host agent (agent and helper ship as one binary/version), keeps `deny_unknown_fields`, and a GitHub/Bitbucket failure serializes byte-for-byte as before (test). `dispatch::outcome` passes it through to `CommandOutcome::Failed.diagnostic`. The helper creates the `DiagnosticReference.id` the way `cleaned()` creates its ids.
- The GitLab branch is chosen from `prepared.target.is_gitlab()` (the `CiTarget` in the `PrepareEnvironment` the helper already receives on Start and Inspect). A GitHub or Bitbucket runner that exits 75 or 77 is never given a GitLab diagnostic (test).

### W-4: parse the core material

The GitLab branch parses `gridops_core::fleet::protocol::agent::GitlabRunnerMaterial` directly when the target is GitLab. No private copy is added to `docker.rs`'s `Material`. A GitLab target in Persistent mode is refused with `Failed { Configuration, retryable: false }`. Material that does not parse is refused as today.

### W-5: refuse at Prepare without the socket

A GitLab target whose profile does not set `requires_docker_socket` is refused at **Prepare** with `Failed { Configuration, retryable: false }`, before any token is minted. The existing `Permission` refusals for a socket the host does not grant are kept unchanged. That GitLab profiles set `requires_docker_socket` is #139/#141's (recorded in the notes and PR).

### W-6: manager specifics

- `runner_command` and `take_bootstrap_secret` branch on **platform** for GitLab (not mode only), so a GitLab request never takes the `jit_config` branch; the bootstrap secret is `gitlab_runner_token`.
- GitLab containers get an `io.gridops.platform=gitlab` label, which the listing uses to know a runner is GitLab.
- The listing reads the exit status with `inspect_container` (`State.ExitCode`), not from `list_containers` status text, and looks it up in `GITLAB_EXIT_REASONS`.
- A GitLab Docker request is refused with `BadRequest` when `shared_docker_socket` is `None` (no Docker socket for runners).
- The manager's Start does not need the readiness wait; a refusal surfaces as `exitReason: "registration_refused"` in the listing.

### W-7: docs

The docs (`docs/fleet-agent.md` and the header comment of `compose.no-docker-socket.yaml`) name `GRIDOPS_RUNNER_DOCKER_SOCKET` (manager path) and `GRIDOPS_AGENT_DOCKER_SOCKET` plus the profile's socket grant (fleet path) as what GitLab runners need, and state that `compose.no-docker-socket.yaml` turns off every built-in runner container, GitLab included.

### W-8: a separate GitLab script

The GitLab runner script is its own function in `provisioning.rs` (for example `gitlab_runner_script(url, wait_timeout_seconds, job_image) -> String`), not a wrapper around the GitHub-specific `instrumented_runner_script`; `gitlab-runner` output goes to the container's stdout/stderr so `docker logs` shows it. `url` is shell-quoted safely (test with a URL containing quotes/spaces is refused or quoted). The GitLab branch **skips** the GitHub shared work folder: the Docker executor keeps builds inside its own job containers and volumes, so no host path mapping is needed.

## Amendments (architecture review R2)

Where this section and the text above differ, this section wins.

### B-2 (R2 block): Start fits inside the reconciler's start deadline

- `provisioning.rs` pins `pub const GITLAB_START_READY_TIMEOUT_SECS: u64 = 15;` (the bounded wait of the fleet Start for the readiness marker or an exit) and `pub const GITLAB_VERIFY_MAX_TIME_SECS: u32 = 10;` (the verify `curl --max-time`). Their doc comment cites `START_COMMAND_DEADLINE_MILLIS` (30 s, `gridops-reconciler/src/fleet.rs`) and `DELIVERY_MARGIN_MILLIS` (5 s), and explains that create + stdin feed + wait must finish under 30 s − 5 s. A test pins both values. `START_COMMAND_DEADLINE_MILLIS` is not changed.
- A verify that times out, fails to connect or gets a non-200/non-403 answer exits with a pinned distinct code `GITLAB_EXIT_VERIFY_UNAVAILABLE` (a constant in `provisioning.rs`, not in `GITLAB_EXIT_REASONS`, not 0/75/77), which Start reports as today's retryable runtime failure. The ready wait timing out is also today's retryable runtime failure.
- The "60 s" example above is withdrawn.

### W-2: compile-forced fallout is owned

Adding `diagnostic` to `HelperResponse::Failed` is mechanical fallout at every struct literal of that variant; all are in Owns, each gaining only `diagnostic: None` (or `..` in patterns): `helper.rs`, `docker.rs`, `native.rs`, `runtime.rs`, `crates/gridops-host-agent/tests/docker_runtime.rs`, `crates/gridops-host-agent/tests/dispatch.rs`, and `crates/gridops-api/tests/fleet_http/placement.rs` (test-only, mechanical; this is the one exception to the "don't edit the API crate" boundary, and no wave-1 issue owns that file). Constructor helpers may be used instead of literals where that is simpler.

### W-3: non-secret values as env vars

Like `GRIDOPS_REGISTRATION_URL` today, the GitLab URL, wait timeout and job image reach the script as fixed-name environment variables (for example `GRIDOPS_GITLAB_URL`, `GRIDOPS_GITLAB_WAIT_TIMEOUT`, `GRIDOPS_GITLAB_JOB_IMAGE`) on both the fleet and manager paths; the script text is constant and contains no interpolated values. The token is never an env var. The W-8 shell-quoting requirement is replaced by this.

### W-4: deviation D147-2

A late exit 77 (after a successful Start) is reported on Inspect as `Failed { ResourceUnavailable, retryable: false, diagnostic: Some(RegistrationRefused) }`, so `reported_gone` still cleans it up. This contradicts the `DiagnosticCode::RegistrationRefused` doc comment (`outcomes.rs`, "Carried in Failed { Configuration }"). Recorded as deviation **D147-2** in the PR and on #136. #147 updates **only that doc comment's text** to say it is carried in `Failed { Configuration }` on Start and `Failed { ResourceUnavailable }` on Inspect; no type, variant or wire value changes. #139 has been told and treats the Inspect case as a failure.

### W-5: Docker executor leftovers

- The runner runs with the Docker executor's cache volumes disabled (`--docker-disable-cache`), so a runner leaves no cache volumes on the host daemon.
- Job and helper containers are created and removed by `gitlab-runner` itself during a normal job. Containers orphaned when Stop/Cleanup kills the runner mid-job are a **known gap deferred to #143**, which runs against a real GitLab and can confirm the labels `gitlab-runner` puts on them before a cleanup filter is written. Recorded in the PR and in `docs/fleet-agent.md`.
