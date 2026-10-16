# Releasing GridOps

Releases are built by `.github/workflows/release.yml` on the project's own
runners. Nothing is compiled on a user's machine.

| Artifact | Built on | Published as |
|---|---|---|
| `api`, `manager`, `reconciler` images (`linux/amd64`, `linux/arm64`) | the Mac's native macOS runner, through its Docker | `ghcr.io/iamngoni/gridops-<service>:<version>`, `:<major.minor>`, `:latest` |
| `gridops-host-agent-linux-x64`, `-linux-arm64` (static, musl) | the Linux Docker runners, in the CI image | release assets |
| `gridops-host-agent-macos-arm64`, `-macos-x64`, `gridops-tart-agent-macos-arm64` | the Mac's native macOS runner | release assets |
| `gridops-host-agent-windows-x64.exe` | the Windows machine's native runner | release assets |
| `install.sh`, `install.ps1`, `compose.release.yaml`, `env.example`, `SHA256SUMS` | the Linux runner | release assets |

## Cut a release

1. Set the version in `Cargo.toml` (`[workspace.package] version`), refresh
   `Cargo.lock` with `cargo check`, and merge that to `master`.
2. Tag the merge commit and push the tag:

   ```sh
   git tag v0.2.0
   git push origin v0.2.0
   ```

The workflow refuses a tag that doesn't match the workspace version. To check
that everything builds without publishing, run the workflow by hand from the
Actions tab.

The images carry the repository as their source, so GHCR links each package
to the repository and gives it the repository's visibility. For a public
repository, anyone can pull them without logging in.

## Runner requirements

- **Linux:** the `gridops-ci` image (`.github/runner/Dockerfile`) has the musl
  toolchain for the host's architecture. Both Linux Docker platforms must run
  it and allow it in `GRIDOPS_AGENT_IMAGES`.
- **macOS:** Xcode command line tools and Docker (OrbStack or Docker Desktop)
  with buildx on the Mac. The job installs its own Rust toolchain under the
  job's temporary directory.
- **Windows:** Visual Studio Build Tools with "Desktop development with C++",
  for the MSVC linker. The job installs its own Rust toolchain.

## How installs use a release

A GridOps deployment serves `/install.sh` and `/install.ps1` with its own
address and version filled in. The Add machine sheet shows:

```sh
curl -fsSL https://gridops.example.com/install.sh | sh
```

```powershell
irm https://gridops.example.com/install.ps1 | iex
```

The installer downloads the agent for the machine's OS and architecture from
the matching release, checks it against `SHA256SUMS`, asks for the enrollment
code, and keeps the agent running as a user service: a LaunchAgent on macOS,
a systemd user unit on Linux, or a scheduled task at logon on Windows.
Running it again upgrades the agent in place.
