# syntax=docker/dockerfile:1.7

FROM node:22.13.0-bookworm-slim AS node-toolchain
FROM rust:1.96-slim-bookworm AS rust-toolchain

FROM ghcr.io/actions/actions-runner:2.336.0

LABEL org.opencontainers.image.source="https://github.com/iamngoni/gridops"

USER root

# Jobs can't use sudo here (no-new-privileges), so the tools workflows commonly
# expect from GitHub's hosted runners are installed now: zip, pip and venv, and
# the GitHub CLI from GitHub's own apt repository.
RUN install -d -m 0755 /etc/apt/keyrings \
    && curl -fsSL https://cli.github.com/packages/githubcli-archive-keyring.gpg \
        -o /etc/apt/keyrings/githubcli-archive-keyring.gpg \
    && chmod go+r /etc/apt/keyrings/githubcli-archive-keyring.gpg \
    && echo "deb [arch=$(dpkg --print-architecture) signed-by=/etc/apt/keyrings/githubcli-archive-keyring.gpg] https://cli.github.com/packages stable main" \
        > /etc/apt/sources.list.d/github-cli.list \
    && apt-get update \
    && apt-get install -y --no-install-recommends build-essential ca-certificates musl-tools pkg-config \
        clang cmake gh python3-pip python3-venv zip \
        libasound2-dev libdbus-1-dev libegl-dev libfontconfig-dev libfreetype-dev libssl-dev \
        libvulkan-dev libwayland-dev libxkbcommon-dev libxkbcommon-x11-dev libzstd-dev \
        libx11-dev libx11-xcb-dev libxcb1-dev libxcb-randr0-dev libxcb-shape0-dev \
        libxcb-shm0-dev libxcb-xfixes0-dev \
    && rm -rf /var/lib/apt/lists/*

COPY --from=node-toolchain /usr/local/bin/node /usr/local/bin/node
COPY --from=node-toolchain /usr/local/lib/node_modules /usr/local/lib/node_modules

COPY --from=rust-toolchain /usr/local/cargo /usr/local/cargo
COPY --from=rust-toolchain /usr/local/rustup /usr/local/rustup

ENV CARGO_HOME=/usr/local/cargo
ENV RUSTUP_HOME=/usr/local/rustup
ENV PATH=/usr/local/cargo/bin:/usr/local/bin:${PATH}

# The runner user owns rustup and cargo's homes, so a job can install the Rust
# a repository asks for (rust-toolchain.toml, setup actions) and `cargo install`
# tools without sudo.
RUN ln -s ../lib/node_modules/npm/bin/npm-cli.js /usr/local/bin/npm \
    && ln -s ../lib/node_modules/npm/bin/npx-cli.js /usr/local/bin/npx \
    && rustup component add rustfmt clippy \
    && rustup target add "$(uname -m)-unknown-linux-musl" \
    && chown -R runner:runner /usr/local/cargo /usr/local/rustup \
    && node --version \
    && npm --version \
    && rustc --version \
    && cargo --version \
    && gh --version \
    && zip --version | head -n 2 \
    && python3 -m venv --help > /dev/null

USER runner
