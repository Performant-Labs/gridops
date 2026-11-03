# macOS runners with Tart

GridOps can run ephemeral Apple Silicon macOS runners in [Tart](https://github.com/openai/tart) virtual machines. Tart uses Apple's Virtualization framework and APFS copy-on-write clones, so each job receives a clean VM without copying the full base disk.

This provider is optional. Docker-only installations do not need Tart or the native agent.

## Requirements

- An Apple Silicon Mac running macOS 13 or newer
- An interactive user session with an unlocked login keychain; Virtualization.framework requires this on recent macOS releases
- Rust 1.96+, `gh`, `jq`, and Homebrew
- Approximately 25 GB for the first Cirrus Labs macOS image, plus working space for clones and job output
- A review of Tart's current license and Apple's macOS virtualization terms before organizational use

Install Tart from its current Homebrew tap:

```sh
brew install openai/tools/tart
```

## Prepare a base image

The default non-vanilla Cirrus Labs images include the Tart guest agent, which allows GridOps to execute the runner without SSH or a host-mounted workspace. Prepare a stopped local base VM containing the current GitHub Actions runner:

```sh
./scripts/prepare-tart-runner-image.sh \
  ghcr.io/cirruslabs/macos-tahoe-base:latest \
  gridops-macos-tahoe-base
```

The script obtains the latest runner release and its SHA-256 digest from GitHub, verifies the archive inside the guest, installs it at `/Users/admin/actions-runner`, and stops the VM. Rerun the script to refresh an existing stopped base image.

Remote image pulls use eight concurrent transfers by default. Set `GRIDOPS_TART_PULL_CONCURRENCY` between 1 and 32 when the registry or network needs a different balance.

Tart's standard pull resumes interrupted layer transfers. On a connection where the registry repeatedly drops large layers, GridOps also includes a verified segmented path. It downloads GHCR blobs with resumable ranges, verifies the compressed and uncompressed layer digests, and assembles the same local VM before the preparation step:

```sh
brew install aria2
./scripts/pull-tart-image-resumable.sh \
  ghcr.io/cirruslabs/macos-tahoe-base:latest \
  gridops-macos-tahoe-base
./scripts/prepare-tart-runner-image.sh \
  ghcr.io/cirruslabs/macos-tahoe-base:latest \
  gridops-macos-tahoe-base
```

Interrupted segmented pulls resume from `~/Library/Caches/GridOps/tart-images`. The default uses four simultaneous layers with eight ranged connections each. Tune the bounded `GRIDOPS_TART_DOWNLOAD_PARALLELISM` and `GRIDOPS_TART_DOWNLOAD_CONNECTIONS` values when necessary.

Use an `-xcode` Tart image instead when workflows require a preinstalled Xcode toolchain. The pool's **Tart base VM** field must contain the local prepared VM name, not the remote OCI reference.

When you create a pool in **Runner pools → New pool** and select **macOS · Tart** under **Providers** with **macOS execution** set to **Virtual machine**, the form asks for the **Tart base VM**. It starts with the control plane's `GRIDOPS_TART_RUNNER_IMAGE` (or `gridops-macos-tahoe-base`); replace it with the pool's own prepared VM before you select **Create pool**, because a new pool starts runners as soon as it is saved and a matching queued job clones whatever VM the pool names. **Native on host** clones no VM, so the field is hidden. A pool with both Docker and Tart asks for a Docker image and a Tart base VM. Change it later in the pool's **Settings → Tart base VM**.

A pool keeps an image only for the providers it uses. When you add **macOS · Tart** to an existing pool in its **Settings**, the **Tart base VM** field starts empty and must be filled in before you save; no default is put in for you. The same goes for the **Docker image** when you add **Linux · Docker** to a Tart pool. Taking a provider out of a pool drops its image.

## Network isolation

GridOps starts VMs without graphics, audio, clipboard sharing, or shared host directories. It supports two Tart network modes:

- `softnet` is the production default. [Softnet](https://github.com/openai/softnet) blocks guest access to private addresses, including the host, while retaining internet egress. It is a separate privileged component; install and review it according to its project documentation before enabling the agent.
- `nat` uses Tart's default networking. It is useful for initial testing but the guest can reach services bound on the host, so it is not the recommended untrusted-CI boundary.

The installer fails closed when `softnet` is selected but its binary is unavailable. GridOps never silently falls back to NAT.

Tart currently requires the Softnet executable to be owned by root with its SUID bit set for unattended VM launches. After installing the Homebrew dependency, resolve the versioned binary and apply the documented privilege:

```sh
softnet_binary="$(realpath "$(command -v softnet)")"
sudo chown root:wheel "${softnet_binary}"
sudo chmod u+s "${softnet_binary}"
```

The installer verifies both conditions. Recheck them after a Softnet upgrade because Homebrew installs a new versioned executable.

## Install the native agent

Generate a token independently from the manager token. The native agent stores its copy in a mode-0600 file; the Docker manager receives the same value through the deployment environment.

For a manager running directly on the Mac, keep the default loopback bind:

```sh
export GRIDOPS_TART_AGENT_TOKEN="$(openssl rand -hex 32)"
./scripts/install-tart-agent.sh
```

For a containerized manager, bind the agent on a host interface reachable as `host.docker.internal`. Keep the random bearer token secret and restrict port 8790 with the host firewall when the machine is on an untrusted LAN:

```sh
export GRIDOPS_TART_AGENT_TOKEN="$(openssl rand -hex 32)"
export GRIDOPS_TART_AGENT_BIND=0.0.0.0:8790
./scripts/install-tart-agent.sh
```

Then configure the control plane with the same token:

```dotenv
GRIDOPS_TART_RUNNER_IMAGE=gridops-macos-tahoe-base
GRIDOPS_TART_AGENT_URL=http://host.docker.internal:8790
GRIDOPS_TART_AGENT_TOKEN=<same independently generated token>
```

Restart the manager after changing these values. Its health response reports Docker and Tart independently, and the pool form will offer **macOS · Tart** under **Providers**.

The installer builds `gridops-tart-agent`, installs it under `~/.gridops/tart-agent/bin`, and registers a per-user launchd agent. Inspect its status and logs with:

```sh
launchctl print "gui/${UID}/dev.gridops.tart-agent"
tail -f ~/.gridops/tart-agent/logs/agent.log
```

Tart starts `softnet` by looking it up on its own PATH, and launchd gives an agent only `/usr/bin:/bin:/usr/sbin:/sbin` unless told otherwise. The installer therefore writes a `PATH` into the launchd agent that starts with the directories it found Tart and Softnet in, and the agent itself puts Tart's directory, and Softnet's when it finds it next to Tart, on its PATH or in Homebrew's `bin` directory, at the front of the PATH of every `tart` command. Its log states that PATH when it starts. To upgrade an agent installed before this, run `./scripts/install-tart-agent.sh` again; it rewrites the launchd agent and restarts it.

### Running the installer again

Run `./scripts/install-tart-agent.sh` again to upgrade the agent or to change a setting; `GRIDOPS_TART_AGENT_TOKEN` must be set every time. When an agent is already installed, every setting you do not pass keeps the value in the installed launchd agent (`~/Library/LaunchAgents/dev.gridops.tart-agent.plist`), even where the installer's default has changed since. Only a first install uses the defaults. This covers every setting the installer writes: `GRIDOPS_TART_AGENT_HOME`, `GRIDOPS_TART_AGENT_INSTALL_DIR` (when it is not the home's `bin` directory), `GRIDOPS_TART_AGENT_BIND`, `GRIDOPS_TART_BINARY`, `GRIDOPS_TART_RUNNER_ROOT`, `GRIDOPS_TART_NETWORK_MODE`, `GRIDOPS_TART_CPU_BUDGET`, `GRIDOPS_TART_MEMORY_BUDGET_MB`, `GRIDOPS_TART_MAX_RUNNERS`, `GRIDOPS_TART_MIN_FREE_DISK_MB` and `GRIDOPS_TART_VM_SLOT_WAIT_SECONDS`. An empty variable counts as not passed. The agent's `PATH` is worked out again from where Tart and Softnet are.

**To change a setting, pass it.** For example, to move an agent installed before 0.2.0 to the new defaults:

```sh
export GRIDOPS_TART_AGENT_TOKEN=<the agent's token>
GRIDOPS_TART_MAX_RUNNERS=2 GRIDOPS_TART_VM_SLOT_WAIT_SECONDS=60 ./scripts/install-tart-agent.sh
```

The installer prints one line for each value it keeps and one for each value it changes, before it builds anything. Shortened:

```text
kept GRIDOPS_TART_CPU_BUDGET=4 from the existing install
set GRIDOPS_TART_MAX_RUNNERS 1 -> 2
set GRIDOPS_TART_VM_SLOT_WAIT_SECONDS 180 -> 60
```

A kept value that is no longer the default says so, for example `kept GRIDOPS_TART_MAX_RUNNERS=1 from the existing install (the default is 2)`. A setting the installed agent does not have yet (it was installed by an older installer) gets the default: `set GRIDOPS_TART_VM_SLOT_WAIT_SECONDS to 60 (not in the existing install)`. A kept value is checked like a passed one, and an error about it says `(from the existing install)`; pass the setting to replace it.

The installer reads the installed launchd agent with `python3` (part of the Xcode Command Line Tools, which Homebrew and Rust need anyway). If it cannot read it, it stops with `Cannot read the existing launchd agent …` and changes nothing, rather than guess the settings; fix the file, or remove it (`launchctl bootout "gui/${UID}/dev.gridops.tart-agent"`, then delete the plist) to install with the defaults and the settings you pass.

### Troubleshooting

- **`Error: InitializationFailed(why: "softnet not found in PATH")`**, which GridOps shows as `tart run exited (exit status: 1) before the guest agent became ready`: Tart could not find `softnet` on the agent's PATH. Every start fails this way, and three in a row open the pool's provisioning circuit for a cool-down. Run `command -v softnet` in a shell to check that Softnet is installed, then re-run `./scripts/install-tart-agent.sh` so the launchd agent gets a PATH that includes its directory.

## Host guardrails

The Tart agent independently enforces a CPU budget, memory budget, runner-count ceiling, and minimum free-disk reserve before cloning a VM. Conservative defaults are:

```dotenv
GRIDOPS_TART_CPU_BUDGET=4
GRIDOPS_TART_MEMORY_BUDGET_MB=6144
GRIDOPS_TART_MAX_RUNNERS=2
GRIDOPS_TART_MIN_FREE_DISK_MB=40960
```

The runner cap is 2 because macOS runs at most two macOS VMs at once and GridOps shares them out itself (next section). Until 0.2.0 it was 1; an agent installed with that default keeps it, also when `scripts/install-tart-agent.sh` is run again, until it is passed: `GRIDOPS_TART_MAX_RUNNERS=2 ./scripts/install-tart-agent.sh` (see [Running the installer again](#running-the-installer-again)).

These are native-host limits, separate from the Docker container limits. The central manager also includes active Tart capacity when admitting any new runner, so Docker and macOS work cannot independently claim the same CPU envelope.

### Sharing the Mac's two VM slots

macOS runs at most **two** macOS virtual machines at once on one Mac. The limit is system wide: VMs started by other software on the same Mac, such as another project's CI runner VMs, take the same two slots. GridOps owns this limit: it is part of the Tart host's budget, next to CPU, memory and disk, and the runner manager enforces it when it admits a runner, before it asks the Tart agent for a VM. The design is in [docs/design/mac-vm-slots.md](design/mac-vm-slots.md).

**The limit.** **Settings → General → macOS VM slots on the Tart host** (API: `macosVmSlots` in `PUT /api/v1/settings`) is 2 by default and can be 1 or 2. GridOps sends it to the runner manager with the rest of its policy on every reconciliation pass. It counts every VM on the Mac, not only GridOps's: set it to 1 only for a Mac that should run one VM at a time. `GRIDOPS_TART_MAX_RUNNERS` still caps GridOps's own runners on the Mac (VM and native); it is 2 by default, so GridOps can use both slots. When the agent's cap is lower than the slot setting, the lower one wins: the runner manager counts the cap in GridOps's own budget on the Mac, so a pool waits with `Waiting for room on the Tart host` while a slot is free. GridOps also says so next to the setting: Settings → General shows a warning under the slot setting (for example **The Tart agent allows 1 runner; slots are set to 2**), and the runner manager's health reports the agent's cap as `providers.tart.vmSlots.agentMaxRunners`, with the same advice in `providers.tart.vmSlots.warning` (null when the cap is at or above the slot setting). Run the installer again with a higher `GRIDOPS_TART_MAX_RUNNERS` (for example `GRIDOPS_TART_MAX_RUNNERS=2 ./scripts/install-tart-agent.sh`; a re-run without it keeps the installed value), or lower the slot setting.

**What counts as a slot in use.** A start is admitted only while, in the manager's view, a slot is free:

- GridOps VM runners whose VM is running (a clone waiting to boot holds none, and native runners need none);
- VMs being started now, from admission until the agent reports the VM running or the start request ends, whichever comes first. A VM is counted once at every moment: while it boots its admission holds the slot, and from the moment the agent lists it running it is counted as a running VM, even if the start request (which still sets up the runner inside the VM) has not returned yet. This needs an agent from 0.2.0 or later, which names the runner of each running VM (`runnerIds` in `capacity.vmSlots`); with an older agent the manager cannot tell a starting VM from a running one and counts it twice for the rest of its start, so `inUse` can read one more than the Mac runs and a pool can wait for a slot that is free. The lease of a start is also given back when the caller abandons the start request;
- VMs of other software that the agent sees. The agent's health (`capacity.vmSlots`) lists them, with the user and how they were found: `tart-process`, a `tart run` process of any user and any Tart home on the Mac (from `ps`), or `tart-list`, a running VM in the agent's own Tart home that is not one of its runners. VMs of other virtualisation tools (UTM, Parallels, VirtualBuddy, or Tart run through a renamed binary) are not seen; for those the agent's wait below is the safety net.

**The queue.** A start that finds no free slot is not attempted. It waits in one queue for the Mac, shared by every pool: first come, first served by when each pool began waiting. When a slot frees, it is kept for the pool at the head of the queue, whichever pool happens to ask first, so two pools that always have macOS work take turns and neither starves. A pool that asks for a slot and gets it goes to the back of the queue for its next one. A pool that stops asking for a whole reconciliation pass (its job ran elsewhere or was cancelled, or it was paused) leaves the queue. A runner started by hand from the pool page does not join the queue and is refused while a pool is waiting, so it cannot jump the queue.

The same queue covers GridOps's own budget on the Mac (`GRIDOPS_TART_MAX_RUNNERS`, `GRIDOPS_TART_CPU_BUDGET`, `GRIDOPS_TART_MEMORY_BUDGET_MB`). That budget can be full before both slots are (for example with a runner cap of 1, the default before 0.2.0, or large VMs against the CPU and memory budget), and a VM start it refuses waits in line too, with the reason `Waiting for room on the Tart host: position … in the queue.` and the budget it hit, so the pool asked first still cannot take the next runner from a pool that has waited longer. Native macOS runners (runtime `native`) take no VM slot and do not wait in this queue.

**What the operator sees.** While a pool waits, its state is **waiting** and its page says **Waiting for a macOS VM slot**, with its place in the queue and who holds the slots, for example: `Waiting for a macOS VM slot: position 2 of 2 in the queue. 2 of 2 slots in use (GridOps runners: 1; other software: 1 — runner-warm-1 (ci)).` The same reason is in the pool API (`provisionWaitReason`) and in the pool's **Runner provisioning waiting** event. The runner manager's health shows the slots and the queue under `providers.tart.vmSlots` (`limit`, `inUse`, `free`, `gridops`, `starting`, `external`, `queue`, and the agent's runner cap `agentMaxRunners` with its `warning`). The **Overview** has a **Tart host** card with the same picture: the slot limit, who holds each slot (a GridOps runner with its pool, its name and how long ago it was started; a VM started outside GridOps, with its name and user; a start in progress; or free), the pools waiting for a slot in line order with how long each has waited, and the agent cap warning when it applies. Pools you cannot see are shown without their names. The card is hidden when the runner manager has no Tart agent, and says so when it has one it cannot reach. Its data is `GET /api/v1/overview/tart-host` (a session or a token with `pools:read`); the runner manager's health now also names the runners of running VMs (`runnerIds`) and each start in flight with its runner and pool (`startingVms`).

Waiting for a slot is not a failure. It never counts towards the pool's provisioning failures or its provisioning circuit, and the pool asks again on the next pass (not after the usual 30-second pause), so it keeps its place in the queue.

A pool is shown **waiting** whenever its latest start was refused for a slot or for room on the host, even if earlier starts failed or its provisioning circuit is open. Only a real block is shown instead: the pool is paused, provisioning is paused, or the built-in runner host it depends on is off. A pool whose circuit is open waits like any other once the circuit's cool-down has passed and it asks again; the circuit stays open while it waits (the pool page says so, and the pool API still returns `provisionCircuitOpen: true` and the earlier `provisionFailureCount`), because a wait shows nothing about whatever made the earlier starts fail. The start the pool is waiting for decides: if it succeeds the circuit closes, and if it fails the circuit opens for another cool-down.

**The agent's wait is a safety net.** Between admission and `tart run`, other software can still take a slot, or a VM the agent cannot see may hold one. Tart then fails with `The number of VMs exceeds the system limit`, and the agent keeps the cloned VM and retries `tart run`, after 2 seconds at first and then less often, up to every 30 seconds, until a slot frees up or the wait runs out:

```dotenv
GRIDOPS_TART_VM_SLOT_WAIT_SECONDS=60
```

The wait counts from when the runner start request reaches the agent, so cloning the VM is part of it. The default is 60 seconds and the maximum is 600 (10 minutes). `0` turns waiting off. The default was 180 seconds until 0.2.0; it is shorter now because admission holds back starts while the slots are full, so a pool waits in the fair queue rather than inside the agent. An agent installed with the old default keeps it, also when `scripts/install-tart-agent.sh` is run again, until it is passed. Set it in the environment of `scripts/install-tart-agent.sh`, which writes it into the launchd agent, for example `GRIDOPS_TART_VM_SLOT_WAIT_SECONDS=60 ./scripts/install-tart-agent.sh`; a re-run without it keeps the installed value.

The maximum follows from how long the runner manager waits for a macOS runner start: 900 seconds (15 minutes), counted from when the request reaches the manager. A slot can free up at the very end of the wait, and the VM then still has up to 180 seconds to boot and some time to start the runner, so the agent keeps 300 seconds back: 600 + 300 = 900. With a longer wait the manager would give up first and the start would fail as a timeout instead of saying no slot became free, so the agent refuses to start with a value above 600, and so does the installer, each saying why. If a start ever does outlast the manager's 900 seconds, for example because cloning the base VM took that long, it fails with `the Tart agent did not finish starting the macOS runner within 900 seconds, ...` (code `tart_start_timeout`). A Docker runner start has its own limit, 720 seconds (12 minutes): the 10 minutes the manager allows for pulling the runner image plus 2 minutes to start the container.

While a runner waits there:

- the agent's log (`~/.gridops/tart-agent/logs/agent.log`) says every macOS VM slot is in use and that it is waiting, and says again when a slot frees up and the VM starts;
- the agent's runner list (`GET /v1/runners`, which the manager passes on) shows the runner with state `created` and status `waiting for a free macOS VM slot since <time>`;
- in GridOps the runner stays **starting**.

If no slot frees up in time, the agent deletes the clone and the start fails with `no macOS VM slot became free within N seconds; the Mac allows two VMs at once, including VMs of other software`. GridOps records that as the runner's failure reason, but, like a start refused at admission, it does not count towards the provisioning circuit: the pool waits and tries again after half a minute. Any other `tart run` failure ends the start at once, without waiting, and counts as a failed start; the agent's log shows Tart's own message.

The wait happens inside the runner start request, as cloning and booting already do. The reconciler waits for that request, with no time limit of its own, before moving on, and the manager starts no other macOS runner meanwhile, so a long wait also delays the rest of the reconciler's work. Docker runners and agent sandboxes have a start lock of their own in the manager, so a macOS start does not hold them up there. Now that admission holds back starts while the slots are full, the wait only covers the short race and VMs the agent cannot see, so it can be kept short.

Starting a runner by hand from the GridOps UI or API is different: the GridOps API gives each request 30 seconds to answer, so a macOS runner that has to wait for a slot, or takes longer than that to clone and boot, is better left to the reconciler.

### Moving another project's macOS CI onto GridOps

A project that keeps its own runner VMs on the Mac (for example one pre-warmed idle VM per slot) holds both slots whether or not a job runs. Until it moves, GridOps sees its `tart run` VMs and counts them; to share the slots fairly, move it onto a GridOps pool:

1. **Prepare its image.** If the project has its own base VM, prepare it with `scripts/prepare-tart-runner-image.sh` (it must contain the GitHub Actions runner at `GRIDOPS_TART_RUNNER_ROOT`) under a name in the agent's Tart home, or clone it there with `tart clone`. Phase 1 of the design uses one Tart home per agent; reading another home's images is a later phase.
2. **Create a pool for it.** Provider **macOS · Tart** with **macOS execution** set to **Virtual machine**, the prepared image as its **Tart base VM**, scoped to the project's repositories (one pool per client), minimum 0 so it holds no slot while idle, and its own label, for example `runs-on: [self-hosted, macOS, ARM64, project-macos]`.
3. **Switch its workflows** to the new label and check that a job runs on a GridOps runner.
4. **Retire its own runners.** Stop its warm-VM scripts and services (launchd agents or cron jobs), `tart stop` and `tart delete` its runner VMs, and remove its self-hosted runner registrations in GitHub. Its slots are then GridOps's to share.
5. **Check** that `providers.tart.vmSlots.external` in the runner manager's health no longer lists its VMs.

Its jobs now start on demand: a cold clone took about 28 seconds to boot in one measured case, which is the cost of not holding an idle VM. A per-pool keep-warm setting is a later phase of the design.

Each macOS runner receives only a short-lived GitHub JIT configuration over stdin. It is never placed in a command-line argument or persisted by the Tart agent. When the job ends, the VM is stopped; the reconciler deletes the clone and its retained management record. The base VM remains stopped and unchanged.

## What a release needs from the macOS runner

Every GridOps release builds the macOS agents (`gridops-host-agent-macos-arm64`, `-macos-x64` and `gridops-tart-agent-macos-arm64`) in the `agent-macos` job of `.github/workflows/release.yml`, and the release is not published without them ([Releasing](releasing.md#the-macos-agent)). The job asks for `runs-on: [self-hosted, macOS, ARM64]`, which every Tart runner advertises, so any pool with the Tart provider can take it. Inside the guest the job needs:

- **Apple silicon, natively.** The first step fails under Rosetta or on an Intel Mac.
- **The Xcode command line tools** (`cc`, `ld`, `lipo`, `xcrun`, `git` and the macOS SDK, which builds both the arm64 and the x86_64 agent). The Cirrus Labs `-base` and `-xcode` images have them; a `-vanilla` image does not.
- **`curl` and `shasum`**, which macOS has in `/usr/bin`.
- **10 GB free** for the Rust toolchain, the crates and two release builds.
- **Internet access** to `sh.rustup.rs`, `static.rust-lang.org`, crates.io and GitHub. Softnet allows it; it blocks only private addresses.

It needs no Rust in the image (it installs its own toolchain under the job's temporary directory), no Homebrew and no Docker. Its first step, **Check the runner**, checks the list above and names what is missing.

The job's limit is 75 minutes once it starts. With the default guardrails above (4 CPUs, two runners at a time) CI jobs that hold both macOS slots, or the agent's whole CPU budget, delay the release, so start the release's dry run when the pool is idle. If no runner starts the job within 60 minutes, the release workflow's `macos-runner-watchdog` job cancels the run and says so in the run summary, rather than leaving it queued for GitHub's 24-hour limit.

Before a release, `scripts/release-preflight.sh` checks that GridOps sees this Mac: its `macos-machine` check needs a machine in the fleet list with `hostOs` `macos` and `architecture` `arm64` that is approved, `active`, has a fresh heartbeat and is not set aside for failing starts. That means the Mac is enrolled as a fleet machine ([Fleet machines](fleet-agent.md)) as well as hosting the Tart agent. Pause or drain it during a release only if you mean to publish without the macOS agent, which needs the repository variable `RELEASE_MACOS` set to `false`.

The release's control plane images (repository variable `RELEASE_IMAGES`) are a different job: it builds Linux images through Docker and buildx, which a Tart guest does not have, so for now it needs a Mac runner with OrbStack or Docker Desktop. Moving it to a Linux runner is [issue #80](https://github.com/Performant-Labs/gridops/issues/80).

## Bitbucket Cloud workspaces

GridOps can attach Bitbucket Cloud workspaces to the same mixed Docker/Tart pool. The workspace runner remains a macOS Tart VM, uses the pool's existing CPU, memory, disk, and runner-count guardrails, and is isolated from GitHub runner credentials and workspaces.

Bitbucket's runner protocol differs from GitHub's JIT protocol: Bitbucket creates a runner with a one-time OAuth secret, then keeps that runner online. GridOps therefore treats a Bitbucket macOS runner as a persistent VM within the pool and deletes its Bitbucket registration before deleting the VM. This is a platform lifecycle distinction, not a separate physical host or pool.

Prepare a stopped Tart base image containing the Bitbucket runner. In Bitbucket Cloud, begin adding a macOS runner and copy only the runner archive URL from the setup dialog—do not copy its registration command or secret into a shell history. Then run:

```sh
./scripts/prepare-tart-bitbucket-runner-image.sh \
  gridops-macos-tahoe-base \
  gridops-macos-tahoe-bitbucket \
  'https://product-downloads.atlassian.com/.../atlassian-bitbucket-pipelines-runner-<version>.tar.gz'
```

The guest needs the Java version currently required by Bitbucket and Git. The preparation script checks both rather than installing packages implicitly. Configure `GRIDOPS_TART_BITBUCKET_RUNNER_ROOT` only if the runner is installed somewhere other than `/Users/admin/atlassian-bitbucket-pipelines-runner`.

In **System → Platform connections**, add an API token restricted to the target workspace with workspace read and Pipelines runner read/write permissions. GridOps verifies the workspace before storing the token encrypted at rest. In the pool form, select the workspace under **Bitbucket Cloud workspaces** and choose the prepared Bitbucket Tart image. Bitbucket adds its own `self.hosted` and `macos` labels; GridOps adds `gridops.<pool-name>` plus the pool's dot-separated custom labels.

## Workflow labels

One pool can include both Docker and Tart. The pool keeps a separate image for each provider it uses and one shared minimum, target, and maximum runner count. GridOps matches each queued job to a compatible provider, then replaces an idle runner from the wrong provider when the pool is already at its maximum.

Tart runners automatically advertise `self-hosted`, `macOS`, `ARM64`, and the pool name. Select the exact pool explicitly:

```yaml
runs-on: [self-hosted, macOS, ARM64, macos-arm64]
```

Docker runners advertise `self-hosted`, `Linux`, `self-hosted-linux`, the host architecture, and the same pool name. `self-hosted-linux` is built in: every Linux runner carries it whether or not its pool lists it, so a workflow asking for it can run on any Linux runner. GridOps only routes a queued job when every requested label matches one selected provider, so Linux and macOS jobs share the pool limit without consuming an incompatible runner slot.
