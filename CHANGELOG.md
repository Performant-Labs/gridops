# Changelog

All notable changes to GridOps are listed here, newest first. Each release has
one section, written in the pull request that bumps the version (see
[Releasing](docs/releasing.md#changelog-and-known-issues)). Pull request numbers
refer to <https://github.com/Performant-Labs/gridops/pulls>.

Known problems in the current release are in the README's
[Known issues](README.md#known-issues).

## Unreleased

### Upgrading

- Migration `0050` adds the fleet health tables (job category rules, the
  hourly job and machine rollups, the recompute list and the score
  snapshots), a nullable `category` on workflow and GitLab jobs, the GitLab
  job's name, project path and start and finish times, and indexes on both
  job tables' `completed_at`. Existing jobs keep their rows and stay
  unclassified; nothing is rebuilt. Timed on a copy of a 3 GB production
  database: under a second. (#197)
- Migration `0052` adds the fleet placement finish queue: the ended
  placements the reconciler still has to finish, queued by triggers when a
  placement ends (#259). It writes nothing to the existing placements, so it
  takes the same time on any database: 1 ms on a synthetic one with 60,000
  placements. After the upgrade the reconciler queues the placements that
  ended earlier and still have work left, 500 placements per short write
  transaction (about 5 ms each), at most 40 batches (20,000 placements) a
  pass, with a 25 ms pause between batches. Until the sweep reaches them,
  those earlier placements wait: a database with 100,000 placements is caught
  up after 5 passes, about 2.5 minutes at the default 30 s interval.

### Added

- **Fleet health, the groundwork.** A **Health** page under Machines
  (`/fleet/health`, administrators only) with a section for each part of the
  page, served from a fixed example fleet until the fleet health issues of
  #198 fill them in; `GET /api/v1/fleet/health?window=1h|24h|7d`, which takes a
  `fleet:read` token like the workload chart; and a **Job categories**
  placeholder under **Settings → General**. New GitHub and GitLab jobs get a
  `category` column, and the reconciler keeps 90 days of fleet health
  rollups. See [Fleet health](docs/design/fleet-health.md). (#197)

### Fixed

- **The reconciler no longer runs out of memory as the fleet's history
  grows.** Every pass read and finished every fleet placement that had ever
  ended, nearly every placement ever made, so each pass's work and the
  reconciler's memory grew with the history until the kernel killed it at its
  512 MiB limit, every 15 to 60 minutes. A pass now reads only the ended
  placements it has not finished yet, and uncertain ones until they are
  cleaned. (#259)

## 0.2.0 - 2026-11-22

The version was set to 0.2.0 on 2026-11-04 (#120), but that tag was never
published; this release is the first `v0.2.0`, and it carries everything below.

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
- **The control plane images are built on a Linux runner** (`self-hosted-linux`),
  which needs `linux/arm64` emulation (QEMU) registered once on the Docker host;
  the job checks it first. They are still built only when the repository
  variable `RELEASE_IMAGES` is `true`. (#105)
- **The API no longer starts Tart runners itself.** `POST
  /api/v1/runner-pools` and the pool scale and reconcile calls leave the
  missing Tart runners to the reconciler and say how many in
  `handedToReconciler`. Docker pools start as before. (#119)

- Migration `0047` adds GitLab connections, GitLab jobs and the GitLab
  columns on runners and fleet CI targets. It rebuilds the `fleet_ci_targets`
  and `runners` tables, copying every row. (#150)
- GitLab runners on the built-in runner host need the runner manager's
  `GRIDOPS_RUNNER_DOCKER_SOCKET`; on an enrolled machine, the agent's
  `GRIDOPS_AGENT_DOCKER_SOCKET` and a profile that shares the socket. (#158)
- The API and the reconciler now log any database statement that runs for
  100 ms or more as a WARN on the `sqlx::query` target. If you set `RUST_LOG`,
  include `sqlx::query=warn` to see them. Expect a few at startup while
  migrations run, and from the reconciler's retention deletes; during normal
  agent traffic there should be none. (#179)
- Migration `0048` adds `retired_at` to fleet CI targets. It rebuilds the
  `fleet_ci_targets` table, copying every row; existing targets stay
  unretired. (#183)
- Machines whose Docker runner profiles share the Docker socket need the new
  host agent build so that GitHub `container:` jobs work there. The agent now
  keeps a runner externals cache for each image under
  `/var/lib/gridops-agent/externals` and a runner folder for each runner under
  `/var/lib/gridops-agent/runners`. (#126)
- Migration `0049` adds an index on fleet inventory snapshots' `received_at`.
  The index is built once, when the services start on the new version, while
  the database is locked for writes. Expect a second or two for a 700 MB
  snapshot table. (#152)
- The first reconcile passes after the upgrade delete the backlog of old
  inventory snapshots, at most 20,000 rows a pass. The database file does not
  shrink: SQLite reuses the freed space. To get the space back, `VACUUM` the
  database while GridOps is stopped. (#152)

### Added

- **GitLab CI runners.** Connect a gitlab.com or self-managed GitLab project
  or group under **Settings → Connections**, add it to a runner pool, and
  GridOps creates a one-job GitLab runner for each runner it starts, on Docker
  or in a Tart macOS VM, and deletes it from GitLab afterwards. Pending jobs
  scale pools through the Job events webhook or a pending-jobs poll. Pool
  labels are the runners' tags, and **Run untagged jobs** opts a pool into
  untagged jobs. A GitLab Docker pool may leave its Docker image empty to run
  `gitlab/gitlab-runner:ubuntu-v19.6.0`, and has no runner group. See
  [GitLab runners](docs/gitlab-runners.md). (#150, #153, #157, #158, #160,
  #161, #162, #163, #164, #166, #172)

- **A Tart host card on the Overview**: the Mac's macOS VM slots, who holds
  each one, the slot queue, and the warning when the agent's cap is below the
  slot setting. (#113)
- **The Tart installer test runs in CI** on the macOS runner, when the
  installer, its plist template or the Tart agent change. (#105)
- The manager's health reports `agentMaxRunners` and a `warning` for the Tart
  agent's cap. (#106)

### Changed

- **Old fleet inventory snapshots are deleted.** Snapshots grew without
  bound, by about 27 MB a day on an 8-machine fleet. The reconciler now
  deletes those older than the new `inventorySnapshotRetentionDays` system
  setting (7 days by default, 1 to 3,650; set through `PUT /api/v1/settings`).
  It always keeps the current snapshot of a session that has not ended, and it
  deletes in small batches so that agent requests are not locked out. See
  [Inventory snapshot retention](docs/fleet-api.md#inventory-snapshot-retention).
  (#152)
- Runner pool API: `runnerGroupId` may be left out. A GitHub pool without one
  gets the 400 "Runner group ID must be positive." instead of a
  422. An invalid update sent to a pool that does not exist, or that the caller
  cannot see, is now a 404 instead of a 400. (#172)

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

- **GitHub `container:` jobs run on Docker runners that share the Docker
  socket.** Every such job failed at its first JavaScript action with
  `exec: "/__e/node24/bin/node": ... no such file or directory`. The runner's
  `externals` and tool cache existed only inside the runner container, so the
  host's daemon mounted empty folders into the job container. The host agent
  now copies each image's `externals` once, when it prepares the runner. It
  runs the runner from a folder that exists at the same path on the host, and
  keeps the tool cache in the runner's work folder. A prepare that cannot copy
  the `externals` fails, and so does a start whose image has no copy, each
  with the reason in the agent's log. See
  [Docker actions and container jobs](docs/fleet-agent.md#docker-actions-and-container-jobs).
  (#126)
- **A fleet runner that cannot be registered no longer holds its machine's
  capacity forever.** A prepared placement whose runner could not be
  registered with GitHub or GitLab and started (for example, GitHub kept
  answering 409 because a live runner held the name, the App credentials were
  unavailable, or the machine had lost its work authority) was retried every
  pass with its capacity held and its runner counted as starting. After 10
  minutes in `registering` it is now given up: its capacity is released, it
  records a `placement_failed` event with the new reason
  `registration_timed_out` and the last error, and the pool replaces the
  runner. Any registration already made upstream is removed by the usual
  runner cleanup. The failure is not counted against the machine and is not
  retried on another machine. See
  [the fleet API](docs/fleet-api.md). (#130)
- **A GitLab connection used on the fleet can be removed again.** Once the
  fleet's target list had been opened, every GitLab connection had a fleet
  target, and removing the connection was refused for good with "is a CI
  target of the fleet". Removing a connection now retires its fleet targets:
  their machine grants and pool platform links are revoked, they are no longer
  offered for new grants or work, and their fleet history is kept. Removal is
  still refused while a pool uses the connection or a runner registered
  through it is live, and is now also refused, naming the pool, while fleet
  work on its targets is running or queued. See
  [Removing a connection](docs/gitlab-runners.md#removing-a-connection). (#183)
- The GitLab and Bitbucket Tart image scripts
  (`scripts/prepare-tart-gitlab-runner-image.sh`,
  `scripts/prepare-tart-bitbucket-runner-image.sh`) flush the guest's disk
  before stopping the VM, so the runner they install is kept, and check the
  finished image in a fresh clone before reporting success. An image whose
  clone has no runner is deleted, and the script exits non-zero. (#173)
- A Tart GitLab runner whose image has no `gitlab-runner` now fails with the
  exit reason `runner_missing`: its failure reason says to prepare the image
  with `scripts/prepare-tart-gitlab-runner-image.sh`, the pool records an
  error event, and the pool's provisioning failure count and backoff go up,
  where before the pool kept starting VMs and reported each one provisioned.
  (#173)
- **A GitLab connection now needs the `manage_runner` scope.** GitLab 17.1 and
  later refuse to delete a runner without it, so a token with only
  `create_runner` and `read_api` left every runner registered in GitLab.
  Saving a connection, replacing its token and **Verify** now refuse such a
  token, and a runner deletion GitLab refuses shows as a **GitLab runner not
  deleted** warning in the pool's events. The token help now asks for the
  Maintainer role or higher on a project and the Owner role on a group.
  (#171, #174)
- **A GitLab pool on fleet machines now scales up for pending jobs.** Its
  pending GitLab jobs raised the pool's desired count but never counted as
  fleet demand, so a pool with a `min_count` of 0 placed no runner and one
  above 0 never grew past it. Pending jobs the pool owns now place fleet
  runners up to `max_count`, scaled by the queue scale factor, as GitHub jobs
  do. Bitbucket pools are unchanged: GridOps has no queued-job signal for
  Bitbucket, and Bitbucket runners don't run on fleet machines. (#181)
- **A busy database no longer drives agents into a reconnect storm.** An
  agent's command poll, and every heartbeat, read every command the machine
  was ever sent while holding the database's write lock. As the command
  history grew this took several hundred milliseconds per request, the lock
  was never free, other writers timed out and the API answered agents 503.
  These reads now see only a machine's open commands, and the reconciler's
  start retry reads only its machine's events. An agent whose heartbeat or
  inventory refresh gets a 5xx, a 429 or no answer now resends it, unchanged,
  until its observation lease runs out, instead of starting a new session on
  the first failure. (#179)

- **A re-run no longer leaves the earlier attempt's jobs queued or in
  progress**, and the "waiting for over five minutes" alert is confirmed with
  GitHub before it shows. The jobs it closes get their real conclusion. (#33,
  #34, #109, #112)
- **Each Mac VM is counted once** while it starts, and a pool waiting for a
  slot shows as waiting, not failed. (#101, #102, #108)
- The README's banner, CI badge and Install downloads point at this
  repository instead of upstream. (#105)
- A finished start gives its slot lease back before the Tart lock is
  released. (#111)
- A pool blocked by the provisioning circuit leaves the blocked state in the
  same pass as a successful probe. (#118)
- Runners the API asked to start (create, scale, reconcile) can no longer
  start a Tart VM beside the reconciler's own start. (#119)
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
