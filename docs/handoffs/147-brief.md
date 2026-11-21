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
- Manual check: run the container with a real `glrt-` token from a test project. It shows online in GitLab, runs one job, and exits. **Not run in #147: deferred to #143**, as Boundaries records (no test project or token exists yet). The criterion testable within #147 is that the PR description states explicitly that this check was not run and is deferred to #143; it is not faked or simulated. (Brief review R1, B-1.)

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

- Owned files: the GitLab branch of `crates/gridops-host-agent/src/docker.rs` (plus whatever host-agent wiring is needed to advertise GitLab and route its material there), the GitLab runner script in `crates/gridops-core/src/provisioning.rs`, and the GitLab arm in `crates/gridops-manager/src/main.rs`. This list is not exhaustive: the amendments below extend it, and the consolidated list is in "Amendments (brief review R1)", NIT-2.
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

## Amendments (brief review R1)

Each finding of the outside brief review, round 1, is resolved here or in place (marked "Brief review R1"). These are additions and clarifications: no acceptance criterion, scope item, deviation or earlier decision is removed or loosened. Where this section and the text above differ, this section wins.

### B-1: the manual check in Done when

Resolved in place in "Done when". The manual check stays listed, marked **not run in #147, deferred to #143**, which is what Boundaries already recorded. The criterion #147 can meet is that the PR description says so explicitly. The two sections now agree.

### NV-1 to NV-4 and NV-6: facts the implementor confirms and cites

These rest on sources outside this repository (the pinned `gitlab-runner` version's source or `--help` output, GitLab's runner API documentation, the pinned image's Dockerfile or manifest). They were **not** verified while amending the brief. Before code relies on any of them, the implementor confirms each headlessly and cites the evidence (source file and line at the pinned version, documentation URL and section, or the command and its output) in the handoff and the PR description:

- **NV-1.** `gitlab-runner run-single` at the pinned version accepts every flag the script passes: `--url`, `--executor docker`, `--docker-image`, `--docker-disable-cache`, `--max-builds 1` and `--wait-timeout`.
- **NV-2.** How `run-single` takes the `glrt-` token (flag, environment variable, config file or stdin), and that the mechanism chosen keeps it out of the container main process's argv and `/proc/1/environ` (B-2 already requires this and the test of what `docker inspect` and `/proc/1/environ` show).
- **NV-3.** The exact signal that a job was received (log line or exit status) that maps the outcome to exit 0 or 75; B-2 already requires it pinned in a constant with the source location cited.
- **NV-4.** `POST {url}/api/v4/runners/verify` accepts the form-encoded `token` field and answers 200 for a valid token and 403 for a refused one.
- **NV-6.** The pinned `GITLAB_RUNNER_IMAGE` tag is Ubuntu-based and ships `bash` and `curl`.

If any of these turns out false, the implementor stops and reports it rather than working around it, as the brief already requires for a contract change.

### NV-5: what Inspect reports for an exited runner today (verified)

A completed GitHub ephemeral runner's exited container inspects today as `Failed { code: ResourceUnavailable, retryable: false, diagnostic: None }`:

- `crates/gridops-host-agent/src/docker.rs:301-317`: Inspect reads `{{.State.Running}}` and the environment label. Anything but running and labelled falls to the `Ok(_)` arm at `:312`, `failed(FailureCode::ResourceUnavailable, false)`, whatever the exit status.
- `crates/gridops-host-agent/src/helper.rs:67`: `HelperResponse::Failed { code: FailureCode, retryable: bool }` has no diagnostic field today.
- `crates/gridops-host-agent/src/dispatch.rs:414-422`: `outcome()` maps `HelperResponse::Failed` to `CommandOutcome::Failed { code, retryable, diagnostic: None }`.
- `crates/gridops-host-agent/tests/docker_runtime.rs:82`: the `"exited"` case pins `HelperResponse::Failed { code: ResourceUnavailable, retryable: false }`.

So B-2's "always `Failed { ResourceUnavailable, retryable: false }` for an exited GitLab container, with `diagnostic: None` for exit 0" matches what the GitHub runner reports today. #147 adds only the diagnostic for exit 75 and a late 77.

### NIT-1: where the script reads the token

The script reads the token from stdin into a **non-exported shell variable**, the way the GitHub branch does today (`docker.rs:128`: `IFS= read -r S`, then `[ -n "$S" ]`). `S` is never `export`ed, so it is in no process's environment. The verify call sends it with the shell builtin `printf 'token=%s' "$S" | curl ... --data @- ...`, so it is in no process's argv either. The script `unset`s `S` once it has handed the token to `run-single` by the mechanism NV-2 confirms.

### W-1: the env-var choice against D1

D1 (and the `GitlabRunnerMaterial` doc comment) says `url` and `wait_timeout_seconds` "may be arguments". R2's W-3 amendment passes them, and the job image, as fixed-name environment variables on both paths. That is within D1's permission and changes no contract item. The PR description and its comment on #136 record it next to D147-1 and D147-2, so #139 and #142 implementors reading #137's contract alone do not diverge.

### W-2: the GitLab URL is checked on the fleet path too

The fleet path checks `url` the way the manager path does (D10: an http(s) URL). The material arrives only on Start, so after the GitLab branch parses `GitlabRunnerMaterial` and before it creates any container, a `url` that does not parse as a URL or whose scheme is not `http` or `https` is refused with `Failed { code: Configuration, retryable: false }`, the same as material that does not parse. It never reaches the verify `curl`, so it can't surface as a retryable runtime failure. A test covers a non-http(s) URL and an unparseable one.

### W-3: the pinned image values go in the PR

The PR description states the exact values chosen for `GITLAB_RUNNER_IMAGE` and `GITLAB_DEFAULT_JOB_IMAGE`, and the headless registry check for each (the command run and the answer that shows the tag exists), so review can confirm them. The test that pins both values and refuses `:latest` is unchanged.

### NIT-2: consolidated ownership

Every path #147 may edit, gathered from the sections above (Boundaries, B-1, B-3, W-7, R2 W-2, R2 W-4):

- `crates/gridops-host-agent/src/docker.rs`: the GitLab branch (Prepare refusal, Start, Inspect diagnostics), the GitLab advertisement in `docker_backend`, and the mechanical `diagnostic: None` fallout.
- `crates/gridops-host-agent/src/helper.rs`, `dispatch.rs`: the `HelperResponse::Failed.diagnostic` field and its pass-through (B-3).
- `crates/gridops-host-agent/src/native.rs`, `runtime.rs`: mechanical `diagnostic: None` only.
- `crates/gridops-host-agent/tests/docker_runtime.rs`, `crates/gridops-host-agent/tests/dispatch.rs`: mechanical fallout, plus new tests.
- `crates/gridops-api/tests/fleet_http/placement.rs`: mechanical `diagnostic: None` only (test-only; the one exception to the API-crate boundary).
- `crates/gridops-core/src/provisioning.rs`: the GitLab runner script and the additive constants (`GITLAB_RUNNER_IMAGE`, `GITLAB_DEFAULT_JOB_IMAGE`, the readiness marker, `GITLAB_START_READY_TIMEOUT_SECS`, `GITLAB_VERIFY_MAX_TIME_SECS`, `GITLAB_EXIT_VERIFY_UNAVAILABLE`). Existing contract items are unchanged.
- `crates/gridops-core/src/fleet/protocol/outcomes.rs`: the text of the `DiagnosticCode::RegistrationRefused` doc comment only (R2 W-4).
- `crates/gridops-manager/src/main.rs`: the GitLab validation arm, launch, label, listing `exitReason` and `ManagedRunner.exit_reason` (D10, D15, W-6).
- `docs/fleet-agent.md` and the header comment of `compose.no-docker-socket.yaml` (W-7, R2 W-5).
- A note for #139 in the shared notes folder (B-1, B-2, W-5).

The "do not edit" list in Boundaries is unchanged.

## Source material read

- docs/handoffs/147-brief.md
- docs/handoffs/147-brief-result-r1.md (the brief review, round 1)
- docs/handoffs/147/handoff-A.md (architecture review, round 2)
- docs/handoffs/147/decisions.md
- docs/handoffs/137-brief.md
- crates/gridops-host-agent/src/docker.rs
- crates/gridops-host-agent/src/helper.rs
- crates/gridops-host-agent/src/dispatch.rs
- crates/gridops-host-agent/src/native.rs
- crates/gridops-host-agent/src/runtime.rs
- crates/gridops-host-agent/tests/docker_runtime.rs
- crates/gridops-host-agent/tests/dispatch.rs
- crates/gridops-api/tests/fleet_http/placement.rs (lines 1-200, the `FakeDocker` helper with every `HelperResponse::Failed` literal; the rest of the file not read)
- crates/gridops-core/src/provisioning.rs
- crates/gridops-core/src/fleet/protocol/agent.rs
- crates/gridops-core/src/fleet/protocol/outcomes.rs
- crates/gridops-core/src/fleet/targets.rs
- crates/gridops-reconciler/src/fleet.rs
- crates/gridops-manager/src/main.rs
- docs/fleet-agent.md
- compose.no-docker-socket.yaml
