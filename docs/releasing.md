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

## Runner requirements

- **Linux:** the `gridops-ci` image (`.github/runner/Dockerfile`) has the musl
  toolchain for the host's architecture. Both Linux Docker platforms must run
  it.
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
