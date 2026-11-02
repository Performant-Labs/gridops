# GridOps CI runner

GridOps' own CI uses an isolated runner image with Node.js 22.13 and Rust 1.96 already installed, plus the musl toolchain for the host's architecture so release builds of the host agent are static. It also carries clang, CMake and the X11/Wayland/Vulkan, audio, D-Bus, font and OpenSSL headers that native GUI apps (such as kanso) need to build, plus `zip`, `pip`/`venv` and the GitHub CLI. The runtime runner remains capability-free with `no-new-privileges`, so `sudo` doesn't work in a job; build tooling is installed while the image is built, never by a privileged workflow step. The runner user owns `/usr/local/cargo` and `/usr/local/rustup`, so a job can still install another Rust version (a `rust-toolchain.toml`, `rustup toolchain install`) and `cargo install` tools, which land on `PATH`.

Build the image on the GridOps Docker host:

Build it on a Docker host of the fleet. This fork publishes to its own Harbor registry (the fleet machines already hold pull credentials for it); build for the architectures the fleet actually runs:

```sh
docker buildx build \
  --platform linux/amd64 \
  --file .github/runner/Dockerfile \
  --tag harbor.performantlabs.com/performantlabs/gridops-ci-runner:node22-rust196 \
  --load .github/runner

docker push harbor.performantlabs.com/performantlabs/gridops-ci-runner:node22-rust196
```

Then set every Linux Docker platform of the build pool to that image and give the pool the `gridops-ci` label.

The CI workflow requests the `gridops-ci` label so jobs cannot land on a generic runner without the required toolchain.
