# syntax=docker/dockerfile:1.7

# Every build is numbered. scripts/build-info.sh prints GRIDOPS_BUILD_NUMBER and GRIDOPS_COMMIT (the image has no .git to read them from),
# and compose passes them through from the environment. This stage refuses a build without them, before anything slow runs, and hands the
# checked values to the web and Rust stages as a file (so an ARG is declared in one place, and a changed number reaches only the layers that use it).
FROM debian:bookworm-slim AS build-info
ARG GRIDOPS_BUILD_NUMBER=""
ARG GRIDOPS_COMMIT=""
COPY scripts/docker-build-info.sh /docker-build-info.sh
RUN sh /docker-build-info.sh /build-info.env

FROM node:22-bookworm-slim AS ui-dependencies
WORKDIR /app
# Retry a failed registry fetch more often than npm's default of 2 (fetch-retries). Build stages only: no runtime image inherits this.
ENV npm_config_fetch_retries=5
COPY package.json package-lock.json ./
RUN npm ci --no-audit --no-fund

FROM ui-dependencies AS ui-build
# Cargo.toml: the version bar reads the workspace version. The build number and commit come from the build-info stage (this stage has no .git).
COPY index.html tsconfig.json vite.config.ts Cargo.toml ./
COPY src ./src
COPY --from=build-info /build-info.env /build-info.env
RUN set -a && . /build-info.env && set +a && npm run build

FROM rust:1.96-slim-bookworm AS rust-build
WORKDIR /app
# Retry a possibly spurious network error more often than cargo's default of 3 (net.retry). Build stage only: no runtime image inherits this.
ENV CARGO_NET_RETRY=10
# Acquire::Retries: apt retries a failed download or connection (default 0).
RUN apt-get -o Acquire::Retries=5 update \
    && apt-get -o Acquire::Retries=5 install -y --no-install-recommends build-essential ca-certificates pkg-config \
    && rm -rf /var/lib/apt/lists/*
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
COPY migrations ./migrations
# The API embeds the OpenAPI spec at compile time (include_str!).
COPY docs/openapi ./docs/openapi
COPY scripts/install.sh ./scripts/
# gridops-core's build script compiles the number in and reruns when GRIDOPS_BUILD_NUMBER or GRIDOPS_COMMIT changes, so the
# target cache below never serves a binary with an older number.
COPY --from=build-info /build-info.env /build-info.env
RUN --mount=type=cache,target=/usr/local/cargo/registry,sharing=locked \
    --mount=type=cache,target=/app/target,sharing=locked \
    set -a && . /build-info.env && set +a \
    && find crates migrations scripts -type f -exec touch {} + \
    && cargo build --workspace --release --locked \
    && mkdir -p /app/bin \
    && cp /app/target/release/gridops-api /app/bin/gridops-api \
    && cp /app/target/release/gridops-manager /app/bin/gridops-manager \
    && cp /app/target/release/gridops-reconciler /app/bin/gridops-reconciler

FROM debian:bookworm-slim AS rust-runtime
RUN apt-get -o Acquire::Retries=5 update \
    && apt-get -o Acquire::Retries=5 install -y --no-install-recommends ca-certificates curl \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --create-home --uid 10001 gridops \
    && mkdir -p /app/data/logs \
    && chown -R gridops:gridops /app
WORKDIR /app

FROM rust-runtime AS api
COPY --from=rust-build /app/bin/gridops-api /usr/local/bin/gridops-api
COPY --from=ui-build /app/dist /app/web
USER gridops
EXPOSE 8080
CMD ["gridops-api"]

FROM rust-runtime AS reconciler
COPY --from=rust-build /app/bin/gridops-reconciler /usr/local/bin/gridops-reconciler
USER gridops
CMD ["gridops-reconciler"]

FROM rust-runtime AS manager
COPY --from=rust-build /app/bin/gridops-manager /usr/local/bin/gridops-manager
EXPOSE 8788
CMD ["gridops-manager"]
