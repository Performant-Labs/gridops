# [Container jobs fail on the linux pool: runner externals aren't mounted into job containers](https://github.com/Performant-Labs/gridops/issues/126)

Read the issue in full first: `gh issue view 126 --repo Performant-Labs/gridops`.

## The failure

On the fleet `linux` pool, every GitHub Actions job that uses `container:` fails in its first JavaScript action:

    exec: "/__e/node24/bin/node": stat /__e/node24/bin/node: no such file or directory

The runner (actions/runner 2.338.2) runs in a Docker container that shares the host's Docker socket. For a container job it runs
`docker create ... -v /var/lib/gridops-agent/work/<env>:/__w -v /actions-runner/externals:/__e -v /opt/hostedtoolcache:/__t`.
The host's daemon resolves bind-mount sources **on the host**. `/__w` works because GridOps already gives the runner a work folder
with the same absolute path on host and in the runner (`gridops_core::fleet::work_folder`, docs/fleet-agent.md "Docker actions
and container jobs"). `/actions-runner/externals` and `/opt/hostedtoolcache` exist only inside the runner container, so Docker
mounts empty, root-owned host directories it creates itself (it also leaves `/actions-runner/externals` and
`/opt/hostedtoolcache` behind on the host, shared by every runner).

## Facts established (read-only, callisto, 2026-11-22)

- Pool image `harbor.performantlabs.com/performantlabs/pl-runner:1.70.4`: `WORKDIR /actions-runner`, entrypoint `/entrypoint.sh`
  (bypassed by the agent), env includes `AGENT_TOOLSDIRECTORY=/opt/hostedtoolcache`. `/actions-runner` is 683 MB, almost all of it
  `externals` (node20, node24, ...). `/opt/hostedtoolcache` is **empty** in the image.
- The runner derives the `/__e` source from its own install root (`<root>/externals`, root = parent of the running `bin/`), not from
  an env var, so the only way to give `/__e` a host-valid source is to run the runner from a root whose `externals` path exists at
  the same absolute path on the host. A symlink inside the runner container at `/actions-runner/externals` does not help (the
  host has no `/actions-runner`). A symlink inside a host-shared root does work: the host kernel follows it at mount time.
- The tool-cache source is the runner's tool directory: `RUNNER_TOOL_CACHE`, else `AGENT_TOOLSDIRECTORY`, else `<work>/_tool`.
- The runner image is built outside this repo (Harbor). The fix must live in the **host agent** (`crates/gridops-host-agent/src/docker.rs`,
  `start` for GitHub runners, the `request.docker_socket` branch around line 180), with shared paths in `gridops_core::fleet::work_folder`.
  The reconciler/server is unchanged unless the plan finds otherwise.

## What to build

1. For a GitHub Docker runner that shares the socket (the branch that already sets up the shared work folder), make the runner's
   externals available at **the same absolute path on the host and in the runner**, and run the runner from a root whose
   `externals` is that path. Both the JIT (`run.sh --jitconfig`) and persistent (`config.sh` then `run.sh`) scripts must work.
   - Do not copy 683 MB per runner start: many runners start per host. Use a per-image externals cache under the agent's
     StateDirectory, `/var/lib/gridops-agent/externals/<image id>`, filled once **in Prepare** (see decision 2), mounted read-only
     into the runner at the same path, plus a small per-environment runner root **outside the work folder** at
     `/var/lib/gridops-agent/runners/<environment id>` (decision 1): everything of the image's runner directory except
     `externals`, with `externals` a symlink to the cache. State what the copy costs per start: the result must give
     the byte count and file count copied into the per-environment root on each start (the image's runner directory
     minus `externals`) and the size of the one-time per-image cache fill, each measured from the pool image when the
     implementer can pull it locally, otherwise estimated, saying which.
   - The cache directory name under `/var/lib/gridops-agent/externals/` is derived from `{{.Id}}` (decision 7) by a
     pure, unit-tested function whose output is always a single, non-empty path component (no `/`, never `.` or
     `..`); an id that cannot be turned into one is treated as a Docker error (`Runtime, true`, decision 5). The
     function's doc comment says how the name is formed (for example whether the `sha256:` prefix is kept).
   - An `externals` in the image that is itself a symlink is either followed (the cache holds the real files) or
     rejected with `Configuration, false` (decision 5, "image lacks ... `externals`"); the implementation picks one,
     says which in a doc comment, and a test pins it. The cache must never end up holding a dangling link.
   - The per-environment root is removed with the runner by the existing cleanup (`shared_work_leftover` /
     `remove_shared_work_folder`, extended to both roots).
   - Stale per-image caches: see decision 8.
2. Point the tool cache at a host-valid per-runner path (for example `RUNNER_TOOL_CACHE` and `AGENT_TOOLSDIRECTORY` set to
   `<work>/_tool`), so `/__t` is not a host-global, root-owned directory shared across runners. The image's tool cache is empty, so
   nothing is lost.
3. Fail loudly instead of starting a runner whose container jobs would fail: a Prepare that cannot fill the externals cache
   fails, and a Start whose cache is missing fails (codes in decision 5), each logging why. The Start cache-exists
   check runs in the socket branch before either runner script is used, so it applies to both the JIT
   (`run.sh --jitconfig`) and the persistent (`config.sh` then `run.sh`) paths.
4. Runners that do not share the socket, GitLab runners, Tart and native runners are unchanged.
5. Tests: unit tests of the generated `docker create` arguments and runner script (the mounts, the env, the root the scripts run
   from, both JIT and persistent), of the cache-path/cleanup parsing, and of the atomic cache fill on a temp dir. No test
   needs a Docker daemon (decision 10 governs; the stand-in `docker` CLI pattern of `tests/docker_runtime.rs` is not a
   daemon and may be used, as decision 10 and handoff-A finding 10 allow, for the fill's `create`/`cp`/`rm` sequence).
   In addition:
   - **Every failure path of requirement 3 / decision 5 has a test** asserting the exact `(FailureCode, retryable)` pair,
     exercising the same pure functions / refactor as decision 10:
     - Prepare, image has no working directory (`{{.Config.WorkingDir}}` empty) → `Configuration, false`;
     - Prepare, image's working directory has no `externals` → `Configuration, false`;
     - Prepare, a folder under the StateDirectory roots cannot be created (permission; use a 0555 temp root as
       `moves_an_unwritable_root_aside_and_starts_a_fresh_one` in `docker.rs` does, skipping when run as root) →
       `Configuration, false`;
     - Prepare, a Docker or IO error during the fill (stand-in `docker` failing `create`, `cp` or `rm`, or a failed
       rename) → `Runtime, true`, and no staging directory is taken for a cache;
     - Start, cache missing for the resolved image id, in both JIT and persistent modes → `Runtime, true`, and no
       `docker create` of the runner container is issued.
     Each test also asserts the reason that is logged: the crate has no log-capture pattern, so the failing function
     returns an error whose text names the image id and the path, the caller logs that text with `tracing::warn!`, and
     the test asserts on the text.
   - The cleanup parse returns both the work folder and the runner root and **never** the externals cache; a parse test
     has the cache mount among the mount sources. A test pins that the cache root is never under `SHARED_WORK_ROOT`
     (and, per decision 1, that the runner root is never under `<work>/`).
6. Docs: extend docs/fleet-agent.md "Docker actions and container jobs" (externals and tool cache, disk used under
   `/var/lib/gridops-agent`), the `work_folder.rs` module doc, and a CHANGELOG entry. Say that hosts need the new agent build:
   that statement goes in the CHANGELOG's `## Unreleased` → `### Upgrading` list (the entry itself under the fitting
   Unreleased heading), and fleet-agent.md "Docker actions and container jobs" says it too.

## Constraints

- `CARGO_BUILD_JOBS=2`, `RUST_TEST_THREADS=2`; prefer `cargo test -p gridops-host-agent` / `-p gridops-core`. Never set `CARGO_TARGET_DIR`.
- Do not touch any fleet host, Mars, or a deployed agent. No live verification on the fleet: the agent's real path cannot be run
  from the Mac. Say plainly in the result what was and was not exercised.

## Architecture decisions (plan review R1, binding)

Full detail: `docs/handoffs/126/handoff-A.md`.

1. **Runner root outside the work folder.** Runner 2.338.2 maps host paths to job-container paths checking Work (`/__w`) before
   Tools and Externals, first match wins (ContainerInfo.cs). A root inside `<work>` would turn the node path into
   `/__w/.../externals/...`, a symlink the job container cannot follow, rebuilding #126. So the root is
   `/var/lib/gridops-agent/runners/<environment id>` (new constant beside `SHARED_WORK_ROOT`), created by the agent and mounted at
   the same path. `parse_shared_work_leftover` returns every direct child of either root, and each is removed by
   `remove_shared_work_folder`. A test pins that the root is never under `<work>/`.
2. **Fill the cache in Prepare, never in Start.** Start has a 25 s usable budget and commands run one at a time. Fill it in
   `prepare`, after the pull, when `request.docker_socket && !prepare.target.is_gitlab()` (Prepare has 20 minutes, as
   NativeRuntime's staging does). Start only resolves the image id and checks the cache exists; missing cache → `Runtime, true`.
3. **No hardcoded `/actions-runner`.** Fill the cache from the image's working directory
   (`docker image inspect -f '{{.Id}} {{.Config.WorkingDir}}'`); the script copies the runner from `$PWD`. The default image
   `ghcr.io/actions/actions-runner` uses `/home/runner`; an existing test (docker_runtime.rs:160-211) pins that the agent assumes
   no absolute install path. No working directory, or no `externals` there → `Configuration, false`.
4. **Reuse `GRIDOPS_RUNNER_ROOT`.** The live job-log forwarder reads `${GRIDOPS_RUNNER_ROOT:-$PWD}/_diag/pages`
   (provisioning.rs:311-334): set it to the runner root. Do the copy, the link and the `cd` in the main shell before `config.sh` /
   `run.sh` (`launch` runs in a background subshell). Copy `bin`, do not link it. Test by running the generated script under bash
   (precedent: gridops-manager main.rs:3248-3277).
5. **Failure codes** (no new FailureCode / DiagnosticCode variants): image lacks a working directory or `externals` →
   `Configuration, false`; cannot create folders under the StateDirectory (permission) → `Configuration, false`; Docker/IO errors
   or full disk → `Runtime, true`; cache missing at Start → `Runtime, true`.
6. **Fill like NativeRuntime** (native.rs:121-135, :484-531): a Mutex field, a staging directory, a rename. Copy with
   `docker create` + `docker cp` + `docker rm` so files are owned by the agent. Mount the cache with
   `--mount type=bind,source=…,target=…,readonly`, not `-v` (which silently creates a missing source). Cache root outside
   `SHARED_WORK_ROOT`. No file lock needed.
7. Key the cache by `{{.Id}}`, and create the runner container from that id rather than the tag.
8. **Stale caches:** after a new fill, remove caches whose image no longer exists locally (the existing `missing()` check) and any
   leftover staging directories. Not "no container uses it" (a prepared-but-not-started placement has no container).
9. Tool cache at `<work>/_tool` via `RUNNER_TOOL_CACHE` (first in the runner's lookup, HostContext.cs:409-415). Check that the
   image's `.env` (loaded by the listener over its environment, Program.cs:172-194) cannot override it, and handle it if it can.
10. **Tests:** make the socket branch's mounts/env and both scripts pure functions taking every root as `&Path`; keep tests in the
    in-crate `docker.rs` module; extend the GitLab test (docker_gitlab.rs:510-515) to assert GitLab gets neither mount. No test
    needs a Docker daemon.

Out of scope: the gridops-manager built-in host has the same class of problem (shares the socket, registers with `_work`); it gets
its own issue.

## Repo-local citations verified (brief gate R1, NV-4 / NV-5 / NV-8)

Checked against this worktree (head 9e357e0). Excerpts are verbatim; line numbers are where they sit today.

- NV-4, live-log forwarder (`crates/gridops-core/src/provisioning.rs`, `instrumented_runner_script` at :311): `:328`
  `runner_root=${{GRIDOPS_RUNNER_ROOT:-$PWD}}` (inside a `format!` string, so the shell sees `${GRIDOPS_RUNNER_ROOT:-$PWD}`),
  and `:334` `for job_log in "$runner_root"/_diag/pages/*.log; do`. `launch` runs as `{launch} >"$runner_diagnostics" 2>&1 &`.
- NV-5, Start budget: `crates/gridops-reconciler/src/fleet.rs:51` `const START_COMMAND_DEADLINE_MILLIS: i64 = 30_000;`;
  `crates/gridops-core/src/fleet/registry/commands.rs:32` `const DELIVERY_MARGIN_MILLIS: i64 = 5_000;`; the 25 s figure is
  spelled out in `provisioning.rs:64-69` ("so creating the container, feeding the token and this wait must all finish
  within 25 s"). Prepare budget: `crates/gridops-core/src/fleet/admission.rs:1151` `.checked_add(1_200_000)` (20 minutes).
  One command at a time: `crates/gridops-host-agent/src/supervisor.rs:491`
  `let in_flight = Arc::clone(&self.in_flight).lock_owned().await;` (field `in_flight: Arc<tokio::sync::Mutex<()>>`, :76).
- NV-8, NativeRuntime fill shape (`crates/gridops-host-agent/src/native.rs`): `:39` `release_lock: Mutex<()>,` (a
  `tokio::sync::Mutex`); `:121-122` `async fn release(&self) -> Result<PathBuf> {` / `let _guard = self.release_lock.lock().await;`;
  `:485` `let staging = target.with_extension("partial");`; `:529` `tokio::fs::rename(&staging, target).await?;`.
- NV-8, precedents: `crates/gridops-host-agent/tests/docker_runtime.rs` — the test
  `a_runner_container_is_created_with_the_flags_its_image_needs` starts at :169 (its doc comment from :160), and :209 asserts
  `recorded.contains("./run.sh") && !recorded.contains("/home/runner/run.sh")`. `crates/gridops-host-agent/tests/docker_gitlab.rs:510-515`
  asserts no arg `contains(gridops_core::fleet::SHARED_WORK_ROOT)` ("a GitLab runner got the GitHub shared work folder").
  `crates/gridops-manager/src/main.rs:3249` `fn job_console_pages_are_forwarded_to_runner_stdout()` (through :3277) runs
  `instrumented_runner_script` under `bash -c` in a temp dir with `.env_remove("GRIDOPS_RUNNER_ROOT")`.

The actions/runner 2.338.2 citations (ContainerInfo.cs, HostContext.cs, Program.cs) and the Docker/kernel bind-mount
behaviour are third-party; handoff-A records them as read from the upstream source, and this brief does not quote them.

## Source material read

- `docs/handoffs/126-brief.md` (this file), in full.
- Issue #126 (`gh issue view 126 --repo Performant-Labs/gridops`), in full.
- `docs/handoffs/126/handoff-A.md`, in full; `docs/handoffs/126/decisions.md`, in full.
- `crates/gridops-host-agent/src/docker.rs`, in full.
- `crates/gridops-core/src/fleet/work_folder.rs`, in full.
- `crates/gridops-core/src/provisioning.rs:55-80`, `:280-370`.
- `crates/gridops-host-agent/src/native.rs:30-140`, `:480-535`, plus its function list.
- `crates/gridops-host-agent/tests/docker_runtime.rs`, in full.
- `crates/gridops-host-agent/tests/docker_gitlab.rs:490-525`.
- `crates/gridops-manager/src/main.rs:3240-3280`.
- `crates/gridops-reconciler/src/fleet.rs:40-51`; `crates/gridops-core/src/fleet/admission.rs:1145-1155`;
  `crates/gridops-core/src/fleet/registry/commands.rs:32`; `crates/gridops-host-agent/src/supervisor.rs:470-500`.
- `docs/fleet-agent.md:295-383` ("Docker actions and container jobs" and neighbours), plus a search of `:1-294` for
  socket/StateDirectory mentions.
- `CHANGELOG.md:1-30` (the `## Unreleased` / `### Upgrading` layout).
- actions/runner 2.338.2 source (ContainerInfo.cs, HostContext.cs, Program.cs): not in this repository, not read in this pass.
