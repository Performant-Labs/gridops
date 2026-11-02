# Fleet machines

A fleet machine is any Linux or macOS host you enroll in GridOps. It runs
`gridops-host-agent`, which connects out to GridOps over HTTPS. Nothing
connects in to the machine. Pools can then run their GitHub Actions runners on
it, in Docker containers or natively on the host. Scripts and bots can
onboard machines without a browser; see
[the automation guide](fleet-automation.md).

## How it fits together

1. An administrator issues a one-time enrollment code for the repositories the
   machine may serve (**Fleet → Add machine**).
2. The agent enrolls with the code, then reports the machine's capacity and
   runtimes every 30 seconds and heartbeats every 15.
3. An administrator approves the machine, turns on the runtimes it should use,
   and activates it. Approving a repository lets the machine run that
   repository's jobs. Running jobs natively on the host, or sharing the host's
   Docker socket, are separate switches on the machine page.
4. A pool runs on fleet machines once its **Fleet execution** profile is
   enabled (pool settings). The reconciler then places each new runner on a
   machine with room, registers it with GitHub, and the agent starts it.
   Ephemeral runners are cleaned up after their job.

A machine only gets work while it heartbeats: if it goes quiet for 90 seconds
it stops receiving new runners, and work that was never delivered is
cancelled and placed elsewhere.

## Capacity: each machine's budget

A machine's **budget** is the most of it GridOps may fill with runners: CPU,
memory and disk. A new machine starts at 80% of what it reports, which leaves
room for the machine itself. An administrator can change it on the machine's
page (**Budget**) or with `PUT /api/v1/fleet/hosts/{host_id}/budget` (see
[the API guide](fleet-api.md)). It can be set anywhere from just above zero up
to exactly what the machine reports, never above. Lowering it below what
runners already use never stops a running job; nothing new is placed until
there is room again.

### The host GridOps runs on

The machine running GridOps itself can start runners two ways, and they are
accounted separately:

- **Built-in runner host.** The runner manager starts runners through Docker on
  its own host. This host is not listed under Machines. Its budget is set on
  the manager (`GRIDOPS_RUNNER_CPU_BUDGET` and `GRIDOPS_RUNNER_MEMORY_BUDGET_MB`,
  95% of the host by default) and shown under **Settings → Runner host**. The
  capacity warning on pool settings names this host and links there.
- **As a fleet machine.** Install the agent on that same host like any other
  machine, then approve and activate it. It then has a budget of its own on
  its machine page.

Until the built-in runner host is retired, the two budgets draw on the same
CPU and memory, so keep their sum within what you want GridOps to use on that
host: lower the manager's budgets and set the machine's. A pool set to run
**Only on machines** never starts runners on the built-in host.

## Install the agent

**Machines → Add machine** issues the enrollment code and shows the installer
for this GridOps. On Linux or macOS:

```sh
curl -fsSL https://gridops.example.com/install.sh | sh
```

Windows is not supported as a fleet machine.

The installer downloads the agent for the machine's OS and architecture from
the GitHub release matching your GridOps version, and checks its SHA-256. It
then asks for the enrollment code and runs the agent as a user service: a
LaunchAgent on macOS or a systemd user unit on Linux. Docker runners use the image each platform names in
GridOps, so a machine needs no image setup. Other settings are listed at the
top of each script.
Running the installer again upgrades the agent and keeps its settings.

### Upgrade an agent that runs as a system service

A Linux machine set up with the system unit (see [Run as a service](#run-as-a-service))
keeps its binary in `/usr/local/bin` and runs as the `gridops-agent` user. The
installer recognises it: it replaces the binary at the path the unit runs,
keeps the previous one as `gridops-host-agent.previous`, and restarts the
service. It needs root, and it never adds a second per-user agent beside the
system one:

```sh
curl -fsSL https://gridops.example.com/install.sh | sudo sh
```

Run as an ordinary user on such a machine, it stops and tells you to use `sudo`.
If the service doesn't come back, the installer says so and prints the rename
that restores the previous binary. Your drop-ins (such as the Docker socket
setting) and the agent's enrollment are not touched.

Release binaries (`gridops-host-agent-<linux|macos>-<x64|arm64>`) can
also be downloaded directly from the
[releases page](https://github.com/iamngoni/gridops/releases). To build from
source instead:

```sh
cargo build --release -p gridops-host-agent
```

Check what the machine will report:

```sh
gridops-host-agent probe
```

Docker appears when the agent's user can run `docker`; if it doesn't, `probe`
shows why under `docker_problem`. Native runners appear on macOS and Linux and
download the current GitHub `actions-runner` release on first use.

## Enroll

On the machine, run the agent with no arguments:

```sh
gridops-host-agent
```

It asks for the GridOps address and the one-time code from **Machines → Add
machine**, enrolls the machine, and starts it. The code works once and expires
after 10 minutes. The address is saved, so later runs need no settings. Then
approve the machine in GridOps.

The agent keeps its state in `~/.gridops/host-agent`: the journal with its credential, the saved address, and native
runner files. Only its user can read the journal.

For scripted installs, set `GRIDOPS_FLEET_URL` and `GRIDOPS_ENROLLMENT_CODE`
and run `gridops-host-agent enroll`; nothing is asked.

To recover a machine (after a quarantine or a lost credential), issue a
recovery code from its page in GridOps and run
`gridops-host-agent enroll --replace`.

## Run as a service

**macOS** (a LaunchAgent, so native jobs run as your user and Docker Desktop or
OrbStack is reachable): copy `deploy/launchd/dev.gridops.host-agent.plist` to
`~/Library/LaunchAgents/`, replace the `__PLACEHOLDERS__` (binary path, fleet
URL, state directory, a `PATH` that includes your Docker CLI, log file), then:

```sh
launchctl bootstrap gui/$(id -u) ~/Library/LaunchAgents/dev.gridops.host-agent.plist
```

**Linux**: install `deploy/systemd/gridops-host-agent.service` to
`/etc/systemd/system/`, set `GRIDOPS_FLEET_URL`, create the `gridops-agent`
user, enroll once as that user, then:

```sh
sudo systemctl enable --now gridops-host-agent
```

## Settings

| Variable | Purpose |
|---|---|
| `GRIDOPS_FLEET_URL` | GridOps address, overriding the one saved at enrollment. Must be HTTPS except on loopback. |
| `GRIDOPS_AGENT_STATE_DIR` | Journal, credential, saved address and native runner files (default `~/.gridops/host-agent`). |
| `GRIDOPS_AGENT_NAME` | Machine name shown in GridOps (default: hostname). |
| `GRIDOPS_AGENT_DOCKER_NETWORK` | Docker network for runner containers. |
| `GRIDOPS_AGENT_DOCKER_SOCKET` | Host Docker socket (usually `/var/run/docker.sock`) to share with profiles that need Docker-in-job, when the machine's grant allows it. Must be set for the agent to advertise the capability at all; see below. |
| `GRIDOPS_AGENT_HELPER_SOCKET` | Use a separate privileged helper (`gridops-host-agent helper <socket>`) instead of running runtimes in-process. |

Native profiles use the image `native:actions-runner` and must be ephemeral.

### Docker-in-job profiles

A profile that needs the host Docker socket is only placed on a machine when
both of these hold:

1. The agent has `GRIDOPS_AGENT_DOCKER_SOCKET` set. Without it the agent
   reports the socket as unsupported, and GridOps leaves the placement queued
   with the reason `capability_unverified`.
2. The machine's grant in GridOps allows the Docker socket.

On Linux, add the variable with a drop-in so it survives reinstalling the unit
file, then restart the agent:

```sh
sudo systemctl edit gridops-host-agent
# [Service]
# Environment=GRIDOPS_AGENT_DOCKER_SOCKET=/var/run/docker.sock
sudo systemctl daemon-reload && sudo systemctl restart gridops-host-agent
```

Once the agent reconnects, the queued placements should clear on the next
admission pass.

## Docker actions and container jobs

A profile that shares the host Docker socket lets jobs use Docker actions and `container:` or
`services:` entries. Those make the runner ask the **host's** Docker daemon to bind-mount paths from the
runner's work folder into another container, so the work folder must exist at the same absolute path on
the host. For these profiles (Docker runtime, socket shared) GridOps therefore gives each runner the
work folder `/var/lib/gridops-agent/work/<environment id>` instead of the default `_work`: the server
writes it into the runner's registration, and the agent creates the folder and mounts it at the same
path. The agent removes it when the runner is cleaned up.

The folder lives under the agent's systemd `StateDirectory` (`/var/lib/gridops-agent`). An agent whose
service user cannot create it there, such as one installed as a user service, fails the runner's start
with a configuration error rather than starting a runner whose Docker actions would fail. Profiles that do
not share the socket are unchanged.

## Exit codes

`run` exits with 3 when GridOps doesn't support this agent version (upgrade the
agent) and 4 when the credential was rejected (recover the machine from its
page in GridOps, then enroll again with the recovery code).
