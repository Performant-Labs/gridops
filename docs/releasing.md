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

Start by creating the release's checklist issue from the template (see
[Release checklist](#release-checklist)); it holds the record of the steps
below, the dry run and the deployment. Its
[Prerequisites](release-checklist-template.md#prerequisites) list what the
person or agent cutting the release needs (GitHub rights, the
[release operator token](fleet-automation.md#the-release-operator-token),
the deploy host, root on system-service machines through a narrow sudo rule),
and `scripts/release-preflight.sh` checks them before step 1.

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

Run the workflow by hand on the release commit before tagging. The checklist
asks for it.

The images carry the repository as their source, so GHCR links each package
to the repository and gives it the repository's visibility. For a public
repository, anyone can pull them without logging in.

## Release checklist

Each release has one issue, `Release checklist: vX.Y.Z`, made from
[`docs/release-checklist-template.md`](release-checklist-template.md). It lists
the steps from the version bump through the dry run, the tag, the control plane
deployment and the agent upgrades to the rollback plan, and it is where the
results of the checks are recorded. The template carries the literal version
`X.Y.Z`; the copy has the real one. Check boxes on the copy, never on the
template file.

To create the issue for a new version, run this from a checkout of the
repository:

```sh
VERSION=0.2.0   # the version being released, without the leading v
sed "s/X\.Y\.Z/$VERSION/g" docs/release-checklist-template.md |
  gh issue create --repo Performant-Labs/gridops \
    --title "Release checklist: v$VERSION" --body-file -
```

To see the issue first without creating it, leave out the `gh` half:

```sh
sed "s/X\.Y\.Z/$VERSION/g" docs/release-checklist-template.md | less
```

The `sed` replaces every `X.Y.Z` (the one in `vX.Y.Z` too). The other
placeholders, `<previous version>`, `<deployment>`, `<merge commit sha>` and the
`___` blanks, are filled in by hand on the issue as the steps are reached. The
repository is public, so the issue gets no private hostnames or internal URLs:
write `<deployment>`. Links in the template are absolute, because relative
links do not resolve inside an issue. The title is always `Release checklist: v` followed by the
version, with no labels.

If a step in the template is wrong or missing, fix the template in a pull
request as part of the release, so the next copy has it.

## Running a release with an agent

A release can be cut end to end by an agent, once the
[prerequisites](release-checklist-template.md#prerequisites) at the top of
the checklist are in place: GitHub rights on the repository, the
[release operator token](fleet-automation.md#the-release-operator-token),
a login on the deploy host, the narrow sudo rule on each machine that runs
the agent as a system service, and a healthy fleet.
`scripts/release-preflight.sh` checks the ones it can reach and fails, with
a list, before step 1 if any is missing. With them, the agent:

- creates the checklist issue and works through it with `gh` (steps 1 to 13
  and 23): the version bump, the changelog, the dry run, the tag, the
  published assets, the close-out;
- pauses, drains or activates a machine with the token
  (`POST /api/v1/fleet/hosts/{host_id}/lifecycle`), for example one the
  preflight or the dry run shows failing starts;
- deploys over ssh on the deploy host (steps 14 to 17): backup, rollback
  tags, `docker compose`, health;
- upgrades each system-service machine with
  `ssh <machine> sudo -n /usr/local/sbin/gridops-agent-upgrade` (step 18),
  and each per-user agent with the installer as that user (step 19);
- checks the fleet and smoke-tests (steps 20 to 22).

What still needs a person:

- **Setting up the prerequisites.** Only an administrator can create the
  token (the token routes are cookie-only), and only someone with root can
  install the sudo rule on a machine. Both are done before the release, and
  the sudo rule only once per machine.
- **Reviews the repository requires.** If a branch rule requires an approving
  review, merging the version bump of step 5 waits for one.
- **Anything the preflight cannot see.** It cannot read the token's expiry or
  tell whether its owner is still an administrator (see
  [the release operator token](fleet-automation.md#the-release-operator-token)),
  it only checks the machines listed in `PREFLIGHT_UPGRADE_HOSTS`, and it
  checks the deploy host's disk but not its Docker or compose access.
- **Decisions.** Whether to roll back (step 22), and which machines stay on
  the old version (step 20), are the release owner's call.

Not built yet: **agent self-update**, a control-plane command that has an
agent download the pinned release, check its SHA-256, swap its binary and
restart itself, so upgrading a system-service machine would need no root at
all. Until it exists, the sudo rule is the supported way to let a releaser
upgrade those machines, and without it step 18 needs a person at each
machine.

## Changelog and known issues

Two files in the repository tell operators what a release changed and what is
still wrong. Both are updated in the pull request that bumps the version, so
the commit the tag points at contains them.

- **`CHANGELOG.md`** has one section per release, newest first, headed
  `## X.Y.Z - <date of the tag>`. Under it, **Upgrading** (what an operator
  must do or will notice), **Added**, **Changed** and **Fixed**, one line per
  change with its pull request number. Write it from the merged pull requests
  since the previous tag and from what was checked, not from titles alone. It
  complements GitHub's generated release notes, which are only the list of
  merged pull requests.
- **README `## Known issues`** lists problems an operator can run into in the
  current release, each linked to its issue (file one first if there is none).
  At each release remove what was fixed, add what is new, and update the "Last
  reviewed for" line. A problem found after a release is added in a pull
  request and noted on that release's checklist issue.

Checklist steps 5, 7 and 8 cover both.

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
