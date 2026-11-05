# Changelog

All notable changes to GridOps are listed here, newest first. Each release has
one section, written in the pull request that bumps the version (see
[Releasing](docs/releasing.md#changelog-and-known-issues)). Pull request numbers
refer to <https://github.com/Performant-Labs/gridops/pulls>.

Known problems in the current release are in the README's
[Known issues](README.md#known-issues).

## 0.2.0 - 2026-11-04

### Upgrading

- Three database migrations run when the services start on the new version:
  `0043` (an attempt number on workflow jobs, so a re-run's earlier attempts
  can be told apart), `0044` (clears the default image stored on a provider a
  pool does not use) and `0045` (an index on jobs without a conclusion). No
  existing table is dropped. (#109, #110, #112)
- Upgrade the API, runner manager and reconciler together. Tart starts now run
  in the reconciler, and the manager reports the slot queue the new Tart host
  card shows. (#106, #113, #115)
- **Reinstall the Tart agent on the Mac** with `scripts/install-tart-agent.sh`.
  `GRIDOPS_TART_MAX_RUNNERS` now defaults to 2 and
  `GRIDOPS_TART_VM_SLOT_WAIT_SECONDS` to 60 (was 180). Re-running the installer
  now keeps the settings of an installed agent unless you pass a variable.
  (#106, #114)
- **The API no longer starts Tart runners itself.** `POST
  /api/v1/runner-pools` and the pool scale and reconcile calls leave the
  missing Tart runners to the reconciler and say how many in
  `handedToReconciler`. Docker pools start as before. (#119)

### Added

- **A Tart host card on the Overview**: the Mac's macOS VM slots, who holds
  each one, the slot queue, and the warning when the agent's cap is below the
  slot setting. (#113)
- The manager's health reports `agentMaxRunners` and a `warning` for the Tart
  agent's cap. (#106)

### Changed

- **The Create pool form asks for the Tart base VM** with a hint on how to
  prepare one. (#99)
- **Each provider has its own start lock**, and Docker starts cover the image
  pull, so a slow start on one provider no longer holds up another. (#107)
- **Tart starts run beside the reconciler pass**, and a macOS VM slot that
  frees goes to the head of the line at once. (#115)
- A pool stores no image for a provider it does not use, nor a base VM for a
  native Tart pool, and editing a pool applies the same image agreement check
  as creating one. (#110, #116)

### Fixed

- **A re-run no longer leaves the earlier attempt's jobs queued or in
  progress**, and the "waiting for over five minutes" alert is confirmed with
  GitHub before it shows. The jobs it closes get their real conclusion. (#33,
  #34, #109, #112)
- **Each Mac VM is counted once** while it starts, and a pool waiting for a
  slot shows as waiting, not failed. (#101, #102, #108)
- A finished start gives its slot lease back before the Tart lock is
  released. (#111)
- A pool blocked by the provisioning circuit leaves the blocked state in the
  same pass as a successful probe. (#118)
- A flaky reconciler test no longer races the Tart start. (#117)

## 0.1.5 - 2026-11-03

### Upgrading

- Four database migrations run when the services start on the new version:
  `0039` (the built-in runner host setting), `0040` (`fleet_hosts.deleted_at`,
  for deleting machines), `0041` (three columns on `workload_intents` and
  two indexes, for retrying failed starts) and `0042` (a new table,
  `runner_pool_waits`, for why a pool waits for a macOS VM slot; no existing
  table changes). (#68, #71, #74, #98)
- **The built-in runner host can be turned off, and migration `0039` picks its
  first value.** It stays **on** if anything may still use it: a pool that can
  start Docker runners and has no enabled fleet profile, an enabled fleet
  profile with placement *Machines first* or *Wherever there's room*, or a live
  runner with a container on the host. Otherwise, and on a new install, it
  starts **off**. Change it under **Settings → Runner host** (administrators)
  or with `PUT /api/v1/settings/runner-host`. With it off, nothing starts a
  runner on the host GridOps runs on, idle runners there are removed, and a
  pool with nowhere else to run shows **Cannot run: built-in runner host is
  off**. (#68)
- Upgrade the API, runner manager and reconciler together. The manager now
  treats the built-in host as off until the reconciler or the API sends it the
  setting, which the reconciler does on every pass. (#68)
- The built-in runner host's CPU and memory budget is now set under
  **Settings → Runner host → Budget** or with
  `PUT /api/v1/settings/runner-host/budget`. The manager's
  `GRIDOPS_RUNNER_CPU_BUDGET` and `GRIDOPS_RUNNER_MEMORY_BUDGET_MB` are only the
  fallback for a figure that is not set there; with nothing set, the budget in
  force is unchanged. (#72)
- Optional: `compose.no-docker-socket.yaml` runs the runner manager without
  the Docker socket (`GRIDOPS_MANAGER_DOCKER=off`). The default `compose.yaml`
  keeps the socket, which the built-in host and the fix agent's sandboxes
  need. (#68)
- **Every release now carries the macOS agents** (`gridops-host-agent-macos-arm64`,
  `-macos-x64`, `gridops-tart-agent-macos-arm64`), built on a self-hosted
  macOS ARM64 runner, and is not published without them. Cutting a release
  therefore needs that runner online; the repository variable `RELEASE_MACOS`
  set to exactly `false` is an emergency opt-out that publishes a release
  without a macOS agent. The control plane images and `compose.release.yaml`
  now have their own variable, `RELEASE_IMAGES`, and are built only when it is
  `true`. (#83)
- **Reinstall the Tart agent on the Mac from this version** with
  `scripts/install-tart-agent.sh`. It now reports the macOS VMs other software
  runs, which the runner manager counts against the Mac's slots (an older agent
  is read as "every active runner is a VM"), and the installer writes a `PATH`
  into the launchd agent so Tart finds Softnet. The new agent binary alone
  already fixes the `PATH`; the rest needs the new manager too. (#87, #93, #98)
- Tart runners wait for a free macOS VM slot for up to
  `GRIDOPS_TART_VM_SLOT_WAIT_SECONDS` (default 180, maximum **600**, `0` turns
  the wait off). It is a setting of the Tart agent on the Mac: set it before
  running `scripts/install-tart-agent.sh`, which writes it into the launchd
  agent. The agent and the installer refuse a value above 600, which is the
  most that fits inside the runner manager's 15-minute limit for a Tart runner
  start. With #98 this wait is only a safety net: the runner manager now
  decides who gets a slot before the agent is asked. (#87, #96, #98)
- New setting **Settings → General → macOS VM slots on the Tart host** (`macosVmSlots`, 1 or 2,
  default 2): how many macOS VMs may run on the Tart host at once, counting
  VMs started by other software. Upgrade the API, runner manager and
  reconciler together; the reconciler sends the value to the manager on every
  pass. Raise the Tart agent's own `GRIDOPS_TART_MAX_RUNNERS` (default 1) to 2
  if GridOps should be able to use both slots. (#98)
- Placement failures are counted from this version on; failures from before
  the upgrade are not in the new counts. (#67)
- **A request with an `Authorization: Bearer …` header to a route that needs a
  signed-in user but takes no API token now gets `403 token_not_accepted`**,
  whether or not the token is valid. Before, such routes answered `200` with a
  signed-out body (for example `/api/v1/settings` and `/api/v1/runner-pools`)
  or `401`. A script that sent a token there must use a route that accepts it;
  `docs/fleet-api.md` has a **Who may call what** table for every route. The
  web app itself is not affected. If a reverse proxy forwards a `Bearer`
  header to GridOps on its own, it would now be refused. (#90)
- New optional setting `GRIDOPS_TOKEN_RETIRE_MIN_OFFLINE_MINUTES` (default 15,
  a whole number from 2 to 10080; the API does not start on any other value):
  how long a machine must have been silent before a token may retire it. There
  is no migration, and existing API tokens keep exactly the scopes they have.
  (#90)

### Added

- **Delete a machine** from its page or with
  `DELETE /api/v1/fleet/hosts/{host_id}` (`fleet:operate`): a retired machine,
  or one that never finished enrolling and has not reported in for 90 seconds.
  It disappears from lists, counts and capacity; its history stays. A refusal
  is `409 host_not_deletable` with every reason. The machine read also lists
  other machines with the same hardware (`duplicateHardwareOf`). (#71)
- **A failed runner start is retried on another machine**, at most three
  attempts per runner, with the machines that failed tried last. **A machine
  with three start failures in 15 minutes is set aside** from new placements
  until a start on it succeeds or the failures age out; its state and running
  jobs are unchanged. It shows on the machine page, as a "Not accepting new
  runners" badge on the Fleet page, as `startGate` on the machine read, and
  as `machines_failing_starts` when a runner waits because every machine is
  set aside. (#74)
- An Overview alert, for administrators, naming each machine set aside for
  failing starts and when it will be reconsidered. (#79)
- **Why a placement failed is recorded**: a `placement_failed` fleet event with
  a reason, the runner's failure reason, a "Recent placement failures" section
  on the machine page, an "N failed in the last hour" badge on the Fleet page,
  and `failedPlacementsLastHour` and `recentPlacementFailures` on the machine
  read. (#67)
- **Settings → Preferences → Live updates → Refresh**: 5 seconds, 10 seconds
  (the default, today's timing), 30 seconds, 1 minute or Off, for this browser.
  With Off, live pages show when they were last updated and a Refresh button.
  Job log viewers are not affected. (#82)
- **Queued jobs** in the sidebar (shortcut `g q`); the queued-jobs count badge
  moved there from Workflow runs. (#76)
- A **Prerequisites** section in the release checklist and
  `scripts/release-preflight.sh`, which checks them before a release, including
  a macOS ARM64 machine for the macOS agent build. (#77, #83)
- **Three API token scopes: `pools:read`, `pools:manage` and `fleet:retire`.**
  No scope includes another, and `fleet:operate` grants none of them. Existing
  tokens are unchanged. (#90)
  - With the pool scopes a token lists, reads, creates, edits and removes runner
    pools and reads their events and capacity history, as its owner (who must
    be a system administrator) and with the web form's validation. Pool
    actions (pause, resume, scale, retry, reconcile) stay browser-only.
  - `POST /api/v1/fleet/hosts/{host_id}/retire` takes a token with
    `fleet:retire` only for a machine that is not active, has reported nothing
    for the quiet period above, and has no live placement, unfinished runner or
    pending command. Otherwise it answers `409 host_not_retirable` listing
    every reason. A retire from the browser is unchanged. The audit row names
    the token.
  - Creating, listing and revoking tokens, recovering a machine, rotating a
    credential and the built-in runner host settings stay browser-only.
- A **Who may call what** table in `docs/fleet-api.md` covers every route, and
  tests check it against the router and the OpenAPI file. (#90)
- **GridOps decides who gets a Mac's two macOS VM slots** (phase 1 of #95).
  The runner manager admits a Tart VM start only when a slot is free, counting
  GridOps's running VMs, starts in progress and the VMs other software runs on
  the Mac. Every pool waits in one first-come, first-served line for the Mac,
  and a pool that just got a slot goes to the back, so pools take turns. A
  waiting pool shows **Waiting for a macOS VM slot** with its place in line
  and who holds the slots, on the pool page and as `provisionWaitReason`; the
  runner manager's health shows the slots and the line under
  `providers.tart.vmSlots`. Waiting is not a failure and never counts towards
  the provisioning circuit. The design and the later phases are in
  `docs/design/mac-vm-slots.md`, and `docs/macos-runners.md` has a guide for
  moving another project's macOS CI onto GridOps. (#98)

### Changed

- A queued job that only a manual-mode pool can run is counted as **on a
  manual pool** (the Overview's queued tile, `metrics.queuedManual`, the
  Queued jobs reason "Manual pool, no runner started"), not as having no
  matching pool, and the run page says the pool is in manual mode instead of
  calling it ready. The Overview, the Queued jobs list and the run page now
  use one rule. Closes #63. (#75)
- A queued job asking for a platform no pool runs (for example
  `self-hosted-macos` with only Linux pools) is now flagged with that label in
  the Overview alert, and each Queued jobs row names the labels no pool
  offers (`missingLabels`). Closes #32. (#73)
- A Tart runner whose Mac already runs two macOS VMs waits for a slot instead
  of failing; after the wait it fails with `vm_slot_unavailable` and a clear
  message. Other `tart run` failures now fail as soon as `tart run` exits.
  Fixes #86. (#87) Since #98 the wait's timeout (`vm_slot_unavailable`), like
  a refused admission (`vm_slot_busy`), is deferred and no longer counts as a
  failed start. (#98)
- The runner manager allows a Tart runner start 15 minutes (Docker runner
  starts keep 3), and the Tart agent's slot wait counts from when the start
  request reaches it, clone included. (#96)
- `install.sh`, the CI and release workflows, the Dockerfiles and the Tart
  image scripts retry transient network errors and time out hung steps. A
  wrong version in `install.sh` now fails after about 30 seconds of retries
  instead of at once. (#66)
- Routes that take no API token refuse one with `403 token_not_accepted`
  instead of answering as a signed-out caller; see Upgrading. (#90)

### Fixed

- An agent that reconnects while its Docker engine is unreachable is no longer
  refused with `409 revision_conflict`; a missing runtime identity is not a
  conflict, and the recorded one is kept. Fixes #84. (#85)
- A Tart runner using Softnet networking no longer fails with `softnet not
  found in PATH` when the Tart agent runs under launchd: every `tart` command
  gets a `PATH` that starts with Tart's and Softnet's directories. Fixes #92.
  (#93)
- A Tart runner start that waited for a VM slot no longer ends in an empty
  `408 Request Timeout`: the runner manager's limit now fits the agent's wait,
  so the agent's `503 vm_slot_unavailable` and its message reach the operator,
  and any request the manager does time out answers with a body that says
  what timed out. Fixes #94. (#96)

### Internal

- Release workflow: a watchdog cancels the run when no macOS runner has started
  the macOS job after 60 minutes, and the publish job also runs on a dry run
  and fails rather than publish without a required build. (#83)
- A regression test that a pool keeps a full pool's jobs across reconcile
  ticks (#42). (#70)
- `scripts/test-install-tart-agent.sh` runs the Tart agent installer against
  stand-ins and checks the launchd plist it writes; it is not yet run by CI.
  (#93)
- The Tart agent's end-to-end tests no longer fail now and then with `Text
  file busy` when another test forks while the fake `tart` script is being
  written. (#96, #98)

## 0.1.4 - 2026-11-02

### Upgrading

- Upgrade fleet machines that run the agent as a system service in place, on
  each machine: `curl -fsSL https://<your GridOps>/install.sh | sudo sh`. It
  keeps the old binary as `gridops-host-agent.previous`. Machines with a
  per-user agent run the same command without `sudo`. Agents report their
  version as `0.1.4+build.<number>`. (#48)
- Windows is no longer a release target: `install.ps1`, its route and the
  Windows agent build are gone. (#48)
- Releases from this repository publish the Linux x64 agent and `install.sh`.
  The macOS and Linux arm64 agents, the container images and
  `compose.release.yaml` are built only when the repository variables
  `RELEASE_MACOS` and `RELEASE_LINUX_ARM64` are `true`. (#48)
- The installer and release artifacts now point at this repository. (#37)
- Docker builds need a build number (`scripts/build-info.sh`); a build without
  one fails. (#56)

### Added

- Scoped API tokens for the fleet and operations routes, so scripts and bots
  can work without a browser: an admin API to create, list and revoke tokens,
  an audit entry naming the token for each action, and the time each token
  was last used. (#39)
- **Settings → API tokens** page. (#41)
- An OpenAPI 3.1 specification for `/api/v1/fleet` and `/api/v1/operations`.
  (#46, #54)
- Disk watch: the agent probes disk use, budgets are backfilled for existing
  machines, and the dashboard raises disk alarms. (#40)
- **Queued jobs** page and an Overview panel that list each queued job with its
  repository, workflow, labels, owning pool and how long it has waited. (#57)
- The app version and build number in a bar at the top of every page; the
  build number is also in `/api/health`. (#53, #56)
- A machine's CPU, memory and disk budget can be edited from its page and from
  `PUT /api/v1/fleet/hosts/{id}/budget`. The capacity warning on pool create
  and pool settings now names the built-in runner host. (#61)
- Every Linux runner carries the label `self-hosted-linux` and every macOS
  runner `self-hosted-macos`. (#51)
- An audit event when a fleet execution profile is saved. (#12)
- A release checklist template and the steps to copy it per release. (#58)

### Changed

- Overview tiles now add up: runners are busy, idle, starting or unavailable,
  and queued jobs are waiting for a pool or have no matching pool, with an
  alert naming the missing label. (#59)
- Success rate covers the last 7 days, leaves out skipped and cancelled runs,
  and shows the period and the number of runs. (#47, #50)
- Needs-attention alerts are at the top of Overview. (#1)
- When a pool is full, its queued jobs are given to the next compatible pool
  that has room instead of waiting. (#43)
- The CI runner image was updated past the end of life of its
  `actions-runner` release. (#38)

### Fixed

- The host agent's Docker runtime matches the runner image's requirements.
  (#11)
- Repositories of organization-scoped pools are discovered for polling without
  webhook delivery. (#13)
- Replaced fleet profiles count as stale rather than forbidden, and orphaned
  queued jobs are reaped. (#44)
- A shared Docker work folder and the pool capacity API. (#45)
