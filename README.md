# GridOps

[![GridOps — Self-hosted GitHub Actions runner control plane](public/social-preview.jpg)](https://github.com/iamngoni/gridops)

[![CI](https://github.com/iamngoni/gridops/actions/workflows/ci.yml/badge.svg)](https://github.com/iamngoni/gridops/actions/workflows/ci.yml)

GridOps is a self-hosted control plane for GitHub Actions runners. Connect a GitHub App, select repositories or organizations, and operate isolated Linux container or macOS virtual-machine runner pools from one interface.

## Capabilities

- GitHub App OAuth, installations, encrypted user tokens, and short-lived installation tokens
- Multi-repository and organization-scoped pools that can share capacity across multiple runner providers
- Ephemeral and persistent Docker runners with labels, CPU, memory, PID, and capability limits
- Ephemeral Apple Silicon macOS runners in copy-on-write Tart virtual machines
- Editable pool configuration with generation-tracked, busy-safe rolling runner replacement
- Provision, scale, reconcile, pause, resume, stop, restart, rebuild, drain, and delete controls
- Queue-driven autoscaling and idle scale-down
- Host-wide CPU, memory, runner-count, disk, and log-storage guardrails with capacity leases
- Provisioning backoff, per-pool circuit breaking, and a global provisioning pause
- Minute-level available, busy, and queued capacity history with 24-hour, 7-day, and 30-day views
- Workflow runs, jobs, cancellation, reruns, downloadable logs, and runner log streaming
- Webhook-driven updates plus installation-token polling for localhost and webhook-outage recovery
- Signed, idempotent GitHub webhooks with delivery retry and audit history
- SQLite WAL persistence, retention policy, and consistent downloadable backups
- A continuously running reconciler that repairs desired versus actual runner state
- A fix agent that diagnoses failed jobs in an isolated sandbox and opens a draft pull request with the fix, using a ChatGPT or Claude subscription or an OpenAI, Anthropic, or OpenRouter API key ([docs/fix-agent.md](docs/fix-agent.md))

## Architecture

- TanStack Router, React 19, Tailwind CSS 4, and shadcn-style UI components in the browser
- Rust/Axum control-plane API
- Rust/SQLx with SQLite migrations
- Rust/Bollard runner manager as the only service with Docker socket access and the provider gateway
- Authenticated native macOS agent for Tart VM lifecycle, capacity, and clean job-log streaming
- Rust background reconciler
- Axum serves the compiled client, same-origin API, and OAuth traffic; Traefik handles private ingress and TLS

TypeScript is confined to the browser application. All authentication, GitHub credentials, persistence, webhooks, runner orchestration, Docker access, and reconciliation live in Rust.

## GitHub App setup

Create a GitHub App with these repository permissions:

- Actions: read and write
- Administration: read and write
- Contents: read and write (fix agent)
- Metadata: read-only
- Pull requests: read and write (fix agent)

For organization-scoped pools, grant organization self-hosted runners read and write access. Subscribe to `workflow_job` and `workflow_run`; GitHub Apps receive `installation`, `installation_repositories`, and `github_app_authorization` automatically, so those events must not be included in a manifest's `default_events`. Enable expiring user access tokens.

Grant organization members read access as well. GridOps uses that permission to distinguish organization owners, who may manage runner infrastructure, from members with read-only visibility.

Configure:

- Callback URL: `${GRIDOPS_BASE_URL}/auth/github/callback`
- Webhook URL: `${GRIDOPS_BASE_URL}/api/webhooks/github`

After signing in, Settings can launch GitHub's App-manifest flow with the exact permissions and webhook events GridOps needs. GitHub returns the App ID, private key, OAuth credentials, slug, and webhook secret directly to GridOps; they are authenticated-encrypted in SQLite and become active without a restart. GridOps then reauthorizes with the newly-created App and sends completed installations through the same state-verified OAuth flow before synchronizing their repositories. Environment values remain supported as bootstrap or deployment-managed overrides.

For localhost development the generated manifest leaves webhook delivery disabled because GitHub cannot reach a loopback URL. Set `GRIDOPS_BASE_URL` to a public HTTPS origin before enabling deliveries. For a private HTTPS origin reachable only through Tailscale or another VPN, set `GRIDOPS_GITHUB_WEBHOOK_ACTIVE=false`; installation-token polling continues to synchronize workflow runs and jobs without inbound GitHub delivery. OAuth, App credentials, and runner control continue to work in both modes.

## Install

Run the control plane from the published images. Download the release's
compose file and settings template:

```sh
curl -fsSLO https://github.com/iamngoni/gridops/releases/latest/download/compose.release.yaml
curl -fsSL -o .env https://github.com/iamngoni/gridops/releases/latest/download/env.example
# Fill the required values in .env, then:
docker compose -f compose.release.yaml up -d
```

To add a machine, open **Machines → Add machine** and run the one-line
installer it shows. The installer fetches the host agent built for the
machine's OS and architecture, and it matches your GridOps version. See
[docs/fleet-agent.md](docs/fleet-agent.md), and
[docs/releasing.md](docs/releasing.md) for how releases are made.

## Run from source

```sh
cp .env.example .env
# Fill the required values, then build with the build number and start:
eval "$(scripts/build-info.sh --export)"
docker compose up --build -d
```

Every build carries a build number (the commit count), shown in the bar at the top of the UI, in `GET /api/health` and in `gridops-host-agent --version`; the Docker build stops without one. See [docs/releasing.md](docs/releasing.md#version-and-build-number).

Open `http://localhost:3000`. For a different public origin, set `GRIDOPS_BASE_URL` and configure the same URL in the GitHub App.

Fleet enrollment requires HTTPS at that origin. Local development can explicitly enable `GRIDOPS_FLEET_ALLOW_LOOPBACK_HTTP=true` (included in `.env.example`); this permits only `localhost` or a literal loopback address. Set it to `false` for remote deployments. The base URL cannot include credentials, a query, or a fragment.

The published API/UI port binds to `127.0.0.1` by default. Set `GRIDOPS_BIND_ADDRESS` explicitly only when direct host exposure is intended; reverse-proxy deployments should attach `api` to an ingress network instead.

For local credentials already stored in `.env.local`:

```sh
eval "$(scripts/build-info.sh --export)"
GRIDOPS_ENV_FILE=.env.local docker compose --env-file .env.local up --build
```

Only `manager` receives `/var/run/docker.sock`. Runner containers do not receive it. The API and reconciler share the `gridops-data` volume for SQLite and retained logs.

The manager is the authoritative capacity boundary. Before GitHub runner credentials are issued, the API or reconciler reserves a short-lived capacity lease; container creation consumes that lease and revalidates the configured runner network and disk watermark. By default GridOps reserves 25% of Docker-host CPU and memory, stops admitting runners below 10 GB of free disk (10% of the disk on disks under 100 GB), limits runner logs to five 20 MB files, and derives a conservative global runner count from a 2 CPU / 2 GB runner shape. Override the `GRIDOPS_RUNNER_*` and `GRIDOPS_MIN_FREE_DISK_*` values from `.env.example` when the Docker host has a deliberately different envelope.

Apple Silicon hosts may additionally run ephemeral macOS pools through Tart. The provider is optional and remains disabled unless both its URL and bearer token are configured. See [docs/macos-runners.md](docs/macos-runners.md) for image preparation, native-agent installation, isolation, and capacity limits.

## Known issues

Last reviewed for v0.1.4. What changed in each release is in the
[changelog](CHANGELOG.md).

- A workflow re-run can leave the previous attempt's jobs shown as queued or in
  progress, so queue alerts stay up until they are cleared
  ([#33](https://github.com/Performant-Labs/gridops/issues/33)).
- The "waiting for over five minutes" alert is not checked against GitHub, so
  it can name a job that GitHub no longer has queued
  ([#34](https://github.com/Performant-Labs/gridops/issues/34)).
- The run page can say a pool with autoscaling off matches a queued job, while
  the Overview counts that job as having no matching pool
  ([#63](https://github.com/Performant-Labs/gridops/issues/63)).
- The runner host built into GridOps and fleet machines have separate budgets
  that draw on the same hardware if the host is both. Keep the sum within what
  you want GridOps to use
  ([#60](https://github.com/Performant-Labs/gridops/issues/60)).
- A retired or duplicate machine cannot be deleted from the Fleet page
  ([#55](https://github.com/Performant-Labs/gridops/issues/55)).
- API tokens are not rate limited
  ([#19](https://github.com/Performant-Labs/gridops/issues/19)).
- Releases from this repository publish the Linux x64 agent and `install.sh`
  only. The macOS and Linux arm64 agents, the container images and
  `compose.release.yaml` are built only when the repository variables
  `RELEASE_MACOS` and `RELEASE_LINUX_ARM64` are `true`, so the download links in
  [Install](#install), which point at the upstream project, do not give you
  this repository's build.

## Develop

Requirements: Node.js 22+, Rust 1.96+, Docker, and the values from `.env.example`.

Run the services in separate terminals:

```sh
set -a; source .env.local; set +a
cargo run -p gridops-manager
cargo run -p gridops-api
cargo run -p gridops-reconciler
npm run dev
```

The Vite server proxies `/api` and `/auth` to Axum at `127.0.0.1:8080`.

## Verify

```sh
npm run lint
npm test
npm run build
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
sh scripts/test-build-info.sh
docker compose --env-file .env.local config
```

Back up the `gridops-data` volume or use the database backup control in Settings. SQLite runs in WAL mode; the download endpoint uses SQLite's consistent `VACUUM INTO` snapshot operation. Backups contain encrypted GitHub credentials, so treat them as sensitive and retain the matching `GRIDOPS_ENCRYPTION_KEY`; without that key the credentials cannot be recovered.

See [the disaster-recovery guide](docs/disaster-recovery.md) for backup verification, guarded restore steps, and recovery drills.
