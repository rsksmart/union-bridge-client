# docker-compose.yml arg to specify the crate to build and run
ARG CRATE
# --build-arg to specify the Rust features to use (ie. anvil)
ARG FEATURES
# --build-arg to specify just one crate to build in the builder stage
ARG JUST_CRATE

# --- BUILDER STAGE ---

FROM ghcr.io/rsksmart/union-client-builder:rust-1.88-v1 AS builder
ARG JUST_CRATE=""
ARG FEATURES=""

# setup SSH for private repositories
RUN mkdir -p -m 0700 ~/.ssh && \
    ssh-keyscan github.com >> ~/.ssh/known_hosts

WORKDIR /app

RUN apt-get update -y && \
    DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends \
        ca-certificates \
        procps \
    && apt-get clean \
    && rm -rf /var/lib/apt/lists/*

# copy only dependency files for better caching
COPY Cargo.toml Cargo.lock ./

# TODO create crates folder and move crates there, so we can copy it instead of the individual crates
# copy workspace members:
COPY common common
COPY block-indexer block-indexer
COPY log-indexer log-indexer
COPY transaction-dispatcher transaction-dispatcher
COPY key-manager key-manager
COPY check-fork check-fork
COPY coordinator coordinator
COPY user-api user-api
# Workspace members required for cargo fetch (integration test crates; excluded from release build)
COPY tests/block-indexer-runner tests/block-indexer-runner
COPY tests/block-indexer-validator tests/block-indexer-validator

ENV RUST_LOG=error
ENV CARGO_TERM_QUIET=true

# SSH mount for private repos
RUN --mount=type=ssh,id=default mkdir -p .cargo && \
    echo '[net]\ngit-fetch-with-cli = true' > .cargo/config.toml && \
    cargo fetch --locked

# two build modes:
# 1) build the entire workspace and reuse among containers, to reduce build time
# 2) build just one crate for speed
# optionally builds with --features if provided
# we also exclude key-manager, which is built always in the next step even if only a couple of crates require it
ARG EXCLUDES="--exclude check-fork --exclude check-fork-zkp --exclude key-manager --exclude block-indexer-runner --exclude block-indexer-validator --exclude mocks"
# TODO when the zkp feature is re-enabled, use this to build coordinator in an image with amd64 support (required by Risc0 in Macs with M chips) via Dockerfile_coordinator
# EXCLUDES="--exclude coordinator--exclude check-fork --exclude check-fork-zkp --exclude key-manager --exclude mocks"
RUN --mount=type=ssh,id=default cargo build --workspace --quiet --release ${EXCLUDES}${JUST_CRATE:+ -p $JUST_CRATE}${FEATURES:+ --features $FEATURES};
# build key-manager always, even if only a couple of crates require it
RUN --mount=type=ssh,id=default cargo build --quiet --release -p key-manager

# --- RUNTIME STAGE ---

FROM debian:bookworm-slim AS runtime
ARG CRATE

# install runtime dependencies
RUN apt-get update -y && \
    DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends \
        ca-certificates \
        procps \
        openssl \
    && apt-get clean \
    && rm -rf /var/lib/apt/lists/*

# copy the crate built binary
COPY --from=builder /app/target/release/${CRATE} /app/${CRATE}
# copy key-manager always, even if only a couple of crates require it
COPY --from=builder /app/target/release/key-manager /app/key-manager

WORKDIR /app

# set environment variables
ENV RUST_BACKTRACE=1
ENV RUST_LOG=debug
# [coordinator Specific] Required by Risc0, otherwise it points to ~/.risc0
# it does not harm to have it always defined, so we leave it common
ENV RISC0_HOME=/opt/risc0

# copy always even if only needed for one crate, as it's cheap
COPY docker/build/entrypoint_sh/key-setup.sh /app/key-setup.sh

LABEL org.opencontainers.image.source=https://github.com/rsksmart/union-bridge-client

# ENTRYPOINT is handled in docker-compose.yml
