# GridOps CI runner

GridOps' own CI uses an isolated runner image with Node.js 22.13 and Rust 1.96 already installed, plus the musl toolchain for the host's architecture so release builds of the host agent are static. It also carries clang, CMake and the X11/Wayland/Vulkan, audio, D-Bus, font and OpenSSL headers that native GUI apps (such as kanso) need to build, plus `zip`, `pip`/`venv` and the GitHub CLI. The runtime runner remains capability-free with `no-new-privileges`, so `sudo` doesn't work in a job; build tooling is installed while the image is built, never by a privileged workflow step. The runner user owns `/usr/local/cargo` and `/usr/local/rustup`, so a job can still install another Rust version (a `rust-toolchain.toml`, `rustup toolchain install`) and `cargo install` tools, which land on `PATH`.

Build the image on the GridOps Docker host:

Build it for both architectures, since the pool runs on arm64 and x64 machines:

```sh
docker buildx build \
  --platform linux/amd64,linux/arm64 \
  --file .github/runner/Dockerfile \
  --tag ghcr.io/iamngoni/gridops-ci-runner:node22-rust196 \
  --load .github/runner
```

Push the image to GHCR, then set every Linux Docker platform of the pool to that image, allow it on each machine's agent (`GRIDOPS_AGENT_IMAGES`), and give the pool the `gridops-ci` label:

```sh
docker push ghcr.io/iamngoni/gridops-ci-runner:node22-rust196
```

The CI workflow requests the `gridops-ci` label so jobs cannot land on a generic runner without the required toolchain.
