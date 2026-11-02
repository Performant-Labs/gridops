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

### The host GridOps runs on: the built-in runner host

The machine GridOps itself runs on can start runners two ways, and they are
accounted separately:

- **Built-in runner host.** The runner manager starts runners through Docker on
  its own host. This host is not listed under Machines. Its budget is set under
  **Settings → Runner host → Budget** (see below).
- **As a fleet machine.** Install the agent on that same host like any other
  machine, then approve and activate it. It then has a budget of its own on
  its machine page.

#### The built-in host's budget

Like a machine's budget, it is the most of the host GridOps may fill with
runners, in CPU and memory. An administrator changes it under **Settings →
Runner host → Budget**, or with `PUT /api/v1/settings/runner-host/budget` as a
signed-in administrator (cookie, same origin):

```json
{"cpuMillis": 12000, "memoryMib": 49152}
```

- A figure left out of the request is kept. `null` clears it, and the runner
  manager's own figure applies again.
- A figure must be above zero and no more than the host reports (the manager's
  `availableCpus` and `totalMemoryMb`). So a figure can only be set while the
  manager is reachable and reports its host; clearing one always works.
- The answer carries the stored `budget`, whether it `changed`, whether the
  manager confirmed it (`managerSynced`), and the budget now in force
  (`effective`: `cpuBudget`, `memoryBudgetMb`, `maxRunners`, and `source` per
  figure).
- Each change is recorded in the audit log as
  `settings.builtin_runner_host_budget_changed` with the figures before and
  after. `GET /api/v1/settings` reports it as `settings.builtinRunnerHostBudget`.
- Lowering it below what runners already use never stops a running job. No new
  runner starts on the host until there is room.

The manager's environment is only the fallback for a figure that is not set:
`GRIDOPS_RUNNER_CPU_BUDGET` and `GRIDOPS_RUNNER_MEMORY_BUDGET_MB` if they are
set there, otherwise 95% of the host. Settings → Runner host says where each
figure in force comes from. The runner count follows the budget (one runner per
2 cores and 2 GB) unless `GRIDOPS_MAX_MANAGED_RUNNERS` fixes it. The reconciler
sends the budget to the manager on every pass, as it does the on/off setting.
After a manager restart its own figures apply until then (at most one reconcile
interval), and no built-in runner starts in that time anyway.

If you also enroll this host as a fleet machine, the two budgets are counted
separately. Keep their sum within what you want runners to use. To move the
host's runners onto the fleet path without a gap, lower this budget as you
raise the machine's, then turn the built-in host off.

If the host that runs GridOps should do no CI work (it is usually the slowest
machine and also serves other things), turn the built-in runner host **off**.

#### Turning it off

**Settings → Runner host → Use the built-in runner host** (administrators), or
`PUT /api/v1/settings/runner-host` with `{"builtinHostEnabled": false}` as a
signed-in administrator (cookie, same origin). `GET /api/v1/settings` reports it
as `settings.builtinRunnerHostEnabled`. Each change is recorded in the audit
log as `settings.builtin_runner_host_changed` with the value before and after
and who made it.

With it off:

- The runner manager refuses to start, restart, resume or rebuild a runner on
  its host (HTTP 409, code `builtin_runner_host_disabled`). So do the API's
  rebuild, start, restart and resume actions, and the reconciler never asks.
  The API's scale and reconcile actions start nothing; the reconciler places
  runners on machines.
- Pools run on machines only. A Linux platform ignores a saved placement of
  *Wherever there's room* or *Machines first* (the saved value is kept and
  applies again if you turn the host back on). Saving a pool's fleet settings
  requires **Run on machines** and *Machines only*.
- A pool that uses only Docker and has no fleet execution cannot run. It is
  shown as **Cannot run: built-in runner host is off** (state
  `builtin-host-off`; `cannotRun` on the pool in the API) instead of looking
  healthy and waiting. Giving it fleet execution, or turning the host back on,
  clears this within a reconcile pass.
- Runners already on the host are not restarted. Idle ones are removed and busy
  ones finish their job; nothing replaces them there.
- The built-in host's capacity warning, budget and disk guardrail are not shown.
- A pool that also has the Tart provider keeps starting its Tart (macOS)
  runners, which run on a separate Mac, not on this host. Tart is not covered by
  this setting. Neither are agent sandboxes ("Fix with agent"), which are not
  runners; see [fix-agent.md](fix-agent.md). They still use the manager's
  Docker access on this host.

The manager treats the host as off until GridOps tells it otherwise, which the
reconciler does at the start of every pass (and the API does when you change the
setting). After the manager restarts, a built-in runner start is refused for up
to one reconcile interval (30 seconds by default).

#### What the setting starts as

A fresh install starts with it **off**. An install that already has pools keeps
working: the migration turns it **on** if any of these hold, and **off**
otherwise:

- a pool that uses only or also uses Docker has no enabled fleet execution;
- a pool's enabled fleet execution has placement *Machines first* or *Wherever
  there's room*;
- a live runner has a container on the host.

Check **Settings → Runner host** after upgrading; if it is on and you do not
want it, turn it off there.

#### Running the manager without the Docker socket

By default the manager container mounts `/var/run/docker.sock`. It needs it for
the built-in runner host, **and** for agent sandboxes ("Fix with agent"), so the
mount stays in `compose.yaml`. If the host is off and you do not use "Fix with
agent", drop the socket with the override file:

```sh
docker compose -f compose.yaml -f compose.no-docker-socket.yaml up -d
```

It sets `GRIDOPS_MANAGER_DOCKER=off` and removes the mount (Docker Compose
2.24.4 or newer). The manager then still serves health and the Tart provider,
reports that it has no Docker access under Settings → Runner host, and refuses
anything that would start a container on its host (`docker_unavailable`), even
if the setting is on. To go back, start the stack without the override.

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

## When runners fail to start on a machine

A machine can look healthy and still fail some of the runners placed on it,
for example when its network drops packets and a start can't fetch what it
needs or reach GridOps in time. GridOps records why each placement failed (the
machine page lists the last day's failures) and acts on it in two ways:

- **The runner is started again elsewhere.** When a runner's start fails on a
  machine and GridOps has proof that nothing of it can still start there, the
  runner is replaced by a new one that tries the other machines first. It
  goes back to the same machine only when no other machine can take it. A
  runner gets at most three attempts. Each retry shows on the machine's page
  as **Failed start retried**.
- **A machine that keeps failing is set aside for a while.** After three
  failed starts within 15 minutes with no successful start in between, the
  machine takes no new runners. Its page and the Fleet list say so ("Not
  accepting new runners: 3 start failures in the last 15 minutes") and when it
  will be reconsidered. It is reconsidered when those failures are more than
  15 minutes old, or sooner if a start on it succeeds. Nothing else changes:
  it stays active, keeps its running jobs, and isn't drained or retired. Fix
  the cause (often the machine's network) using the reasons on its page.

## Exit codes

`run` exits with 3 when GridOps doesn't support this agent version (upgrade the
agent) and 4 when the credential was rejected (recover the machine from its
page in GridOps, then enroll again with the recovery code). A machine that was
deleted from GridOps can't be recovered: its credential stays rejected until
you enroll the computer again with a new code from **Machines → Add machine**,
which creates a new machine.
