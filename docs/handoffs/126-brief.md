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
   - Do not copy 683 MB per runner start if it can be avoided: many runners start per host. The preferred shape (architect decides)
     is a per-image externals cache under the agent's StateDirectory, for example `/var/lib/gridops-agent/externals/<image id>`,
     populated once (atomically: build in a temp dir, rename into place; concurrent starts of the same image must not race or see a
     half-filled cache) from the runner image, mounted read-only into the runner at the same path; plus a per-environment runner
     root inside the shared work area (small: everything of `/actions-runner` except `externals`, with `externals` pointing at the
     cache). State what the copy costs per start.
   - The per-environment root must be removed with the runner, by the existing cleanup (`shared_work_leftover` /
     `remove_shared_work_folder`), and must never collide with job checkouts in the work folder.
   - Say what happens to stale per-image caches (an image no longer used). A bounded, simple rule is enough.
2. Point the tool cache at a host-valid per-runner path (for example `RUNNER_TOOL_CACHE` and `AGENT_TOOLSDIRECTORY` set to
   `<work>/_tool`), so `/__t` is not a host-global, root-owned directory shared across runners. The image's tool cache is empty, so
   nothing is lost.
3. Fail loudly instead of starting a runner whose container jobs would fail: if the externals cache cannot be prepared, the
   start fails with a configuration/runtime error (pick the right `FailureCode` and retryability) and logs why.
4. Runners that do not share the socket, GitLab runners, Tart and native runners are unchanged.
5. Tests: unit tests of the generated `docker create` arguments and runner script (the mounts, the env, the root the scripts run
   from, both JIT and persistent), of the cache-path/cleanup parsing, and of the atomic cache fill on a temp dir. Tests must not
   need a Docker daemon in CI unless the existing suite already has a Docker-gated pattern.
6. Docs: extend docs/fleet-agent.md "Docker actions and container jobs" (externals and tool cache, disk used under
   `/var/lib/gridops-agent`), the `work_folder.rs` module doc, and a CHANGELOG entry. Say that hosts need the new agent build.

## Constraints

- `CARGO_BUILD_JOBS=2`, `RUST_TEST_THREADS=2`; prefer `cargo test -p gridops-host-agent` / `-p gridops-core`. Never set `CARGO_TARGET_DIR`.
- Do not touch any fleet host, Mars, or a deployed agent. No live verification on the fleet: the agent's real path cannot be run
  from the Mac. Say plainly in the result what was and was not exercised.
