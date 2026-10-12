# Fleet machines

A fleet machine is any Linux or macOS host you enroll in GridOps. It runs
`gridops-host-agent`, which connects out to GridOps over HTTPS. Nothing
connects in to the machine. Pools can then run their GitHub Actions runners on
it, in Docker containers or natively on the host.

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

## Build the agent

```sh
cargo build --release -p gridops-host-agent
```

The binary is `target/release/gridops-host-agent`. Copy it to the machine
(for example `/usr/local/bin`).

Check what the machine will report:

```sh
gridops-host-agent probe
```

Docker appears when the agent's user can run `docker`. Native runners appear
on macOS and Linux and download the current GitHub `actions-runner` release on
first use.

## Enroll

```sh
export GRIDOPS_FLEET_URL=https://gridops.example.com
export GRIDOPS_AGENT_STATE_DIR="$HOME/Library/Application Support/GridOps/agent"   # macOS
export GRIDOPS_ENROLLMENT_CODE=<code from GridOps>
gridops-host-agent enroll
```

The code works once and expires after 10 minutes. The agent stores its
credential in `journal.sqlite` in the state directory, readable only by its
user. Then approve the machine in GridOps.

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
| `GRIDOPS_FLEET_URL` | GridOps address. Must be HTTPS except on loopback. |
| `GRIDOPS_AGENT_STATE_DIR` | Journal, credential and native runner files. |
| `GRIDOPS_AGENT_NAME` | Machine name shown in GridOps (default: hostname). |
| `GRIDOPS_AGENT_IMAGES` | Comma-separated runner images Docker profiles may use. Images outside this list are refused. |
| `GRIDOPS_AGENT_DOCKER_NETWORK` | Docker network for runner containers. |
| `GRIDOPS_AGENT_DOCKER_SOCKET` | Host Docker socket to share with profiles that need Docker-in-job, when the machine's grant allows it. |
| `GRIDOPS_AGENT_HELPER_SOCKET` | Use a separate privileged helper (`gridops-host-agent helper <socket>`) instead of running runtimes in-process. |

Native profiles use the image `native:actions-runner` and must be ephemeral.

## Exit codes

`run` exits with 3 when GridOps doesn't support this agent version (upgrade the
agent) and 4 when the credential was rejected (recover the machine from its
page in GridOps, then enroll again with the recovery code).
