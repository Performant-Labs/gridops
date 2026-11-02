# Releasing GridOps

Releases are built by `.github/workflows/release.yml` on the project's own
runners. Nothing is compiled on a user's machine.

| Artifact | Built on | Published as |
|---|---|---|
| `gridops-host-agent-linux-x64`, `-linux-arm64` (static, musl) | the Linux Docker runners, in the CI image | release assets |
| `install.sh`, `SHA256SUMS` | the Linux runner | release assets |
| `gridops-host-agent-macos-arm64`, `-macos-x64`, `gridops-tart-agent-macos-arm64` | the Mac's native macOS runner | release assets (only with a Mac runner) |
| `api`, `manager`, `reconciler` images (`linux/amd64`, `linux/arm64`) | the Mac's native macOS runner, through its Docker | `ghcr.io/<owner>/gridops-<service>:<version>`, `:<major.minor>`, `:latest` (only with a Mac runner) |
| `compose.release.yaml`, `env.example` | the Linux runner | release assets (only when the images were built) |

Windows is not a deployment target: nothing is built or published for it.

## Without a Mac runner

The macOS agents and the control plane images need a self-hosted macOS ARM64
runner. They run only when the repository variable `RELEASE_MACOS` is `true`
(Settings → Secrets and variables → Actions → Variables). Without it the
release carries the Linux agents and `install.sh`, which is enough to enroll and
upgrade Linux fleet machines. Deployments that build their images from source
(`docker compose build`) don't need the images at all.

## Cut a release

1. Set the version in `Cargo.toml` (`[workspace.package] version`), refresh
   `Cargo.lock` with `cargo check`, and merge that to `master`.
2. Tag the merge commit and push the tag:

   ```sh
   git tag v0.2.0
   git push origin v0.2.0
   ```

   **Publish the release before deploying the new version.** A deployment
   serves `/install.sh` pinned to its own version, so one running 0.2.0 sends
   machines to `v0.2.0`; until that release exists, installs fail.

The workflow refuses a tag that doesn't match the workspace version. To check
that everything builds without publishing, run the workflow by hand from the
Actions tab.

The images carry the repository as their source, so GHCR links each package
to the repository and gives it the repository's visibility. For a public
repository, anyone can pull them without logging in.

## Version and build number

Two numbers name a GridOps build, and they change for different reasons.

| | Example | Changes | Used for |
|---|---|---|---|
| **Version** (semver) | `0.1.3` | when you cut a release | release tags, the `v0.1.3` assets installers download, the version `/install.sh` is pinned to |
| **Build number** | `412` | with every commit | telling builds apart: each commit merged to `master` is a new build, so two builds of the same version are never confused |

The build number is the number of commits in the history of the commit being
built (`git rev-list --count HEAD`). It rises with every merge and is the same
for everyone who builds the same commit. It never goes into a release tag, an
installer or a release asset name: the version alone names those, which is why
the version is not bumped per build (a bump would point installers at a release
that has not been cut yet).

Where the number shows:

- **Web UI:** the bar at the top of every page, `v0.1.3 · build 412 · 9d92a33`
  (version, build number, commit).
- **API:** `GET /api/health` (no login) returns `version`, `buildNumber` and
  `commit`. The API logs the same at startup.
- **Host agent:** `gridops-host-agent --version` prints
  `gridops-host-agent 0.1.3+build.412 (9d92a33)`, and the agent reports
  `0.1.3+build.412` to GridOps at every handshake, so the Machines pages show
  which build each machine runs. After you upgrade an agent, its build number
  rises even though the version does not.

`0.1.3+build.412` is the version with the build number as semver *build
metadata*: still a valid semver string, still sorts as `0.1.3`, and the fleet
protocol accepts it (it stores the agent version as opaque text, and
compatibility is decided by the separate protocol version).

### Building with a number

`scripts/build-info.sh` is the one place the number is read. It prints
`GRIDOPS_BUILD_NUMBER` and `GRIDOPS_COMMIT`, and fails with a message in a
shallow clone or outside a git checkout (where the count would be wrong or
missing) instead of inventing one. Variables already set in the environment win.

```sh
# Docker images: a build without a number stops right away.
env $(scripts/build-info.sh) docker compose build
# or export both into the shell once, then use compose as usual:
eval "$(scripts/build-info.sh --export)"
docker compose build api manager reconciler && docker compose up -d --no-deps api manager reconciler

# Plain cargo and npm builds read git themselves, or take the variables:
cargo build --release -p gridops-host-agent
GRIDOPS_BUILD_NUMBER=412 GRIDOPS_COMMIT=9d92a33 cargo build --release -p gridops-host-agent
```

Docker builds have no `.git`, so the Dockerfile requires `GRIDOPS_BUILD_NUMBER`
and `GRIDOPS_COMMIT` as build arguments (compose passes them from the
environment) and refuses to build without them. Cargo and npm builds outside
Docker fall back to git; with no git history they say `dev` / `build unknown`
rather than guess. The release workflow counts the number once, in its
`version` job, and gives it to every agent build and image build, so the
artifacts of one run agree.

Rebuilding the same commit gives the same number. To get a new number, commit.

## Runner requirements

- **Linux:** any runner with the `self-hosted-linux` label, which every Linux
  pool takes. The job runs as root in the runner container and installs
  `musl-tools` and the musl Rust target itself, so the image needs only Rust,
  `gcc` and `apt-get`. The x64 agent always builds. The arm64 agent builds only
  when the repository variable `RELEASE_LINUX_ARM64` is `true`, which needs an
  `ARM64` runner in a pool.
- **macOS:** Xcode command line tools and Docker (OrbStack or Docker Desktop)
  with buildx on the Mac. The job installs its own Rust toolchain under the
  job's temporary directory.

## How installs use a release

A GridOps deployment serves `/install.sh` with its own address and version
filled in. The Add machine sheet shows:

```sh
curl -fsSL https://gridops.example.com/install.sh | sh
```

The installer downloads the agent for the machine's OS and architecture from
the matching release, checks it against `SHA256SUMS`, asks for the enrollment
code, and keeps the agent running as a user service: a LaunchAgent on macOS or
a systemd user unit on Linux. Running it again upgrades the agent in place.

A Linux machine that runs the agent as a system service is upgraded in place
instead, with `curl -fsSL https://gridops.example.com/install.sh | sudo sh`; see
[Fleet machines](fleet-agent.md#upgrade-an-agent-that-runs-as-a-system-service).
`scripts/test-install.sh` tests that path and runs in CI.
