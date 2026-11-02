# Changelog

All notable changes to GridOps are listed here, newest first. Each release has
one section, written in the pull request that bumps the version (see
[Releasing](docs/releasing.md#changelog-and-known-issues)). Pull request numbers
refer to <https://github.com/Performant-Labs/gridops/pulls>.

Known problems in the current release are in the README's
[Known issues](README.md#known-issues).

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
