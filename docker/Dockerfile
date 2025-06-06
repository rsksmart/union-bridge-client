# docker-compose.yml arg to specify the crate to build and run
ARG CRATE
# docker-compose.yml arg to specify the tag of union-contracts repository to use - if defined, contracts will be deployed for that service
ARG UNION_CONTRACTS_TAG
# --build-arg to specify the Rust features to use (ie. anvil)
ARG FEATURES
# --build-arg to specify just one crate to build in the builder stage
ARG JUST_CRATE

# --- BUILDER STAGE 1: build the deps ---

FROM rust:1.86-slim-bookworm AS builder
ARG JUST_CRATE=""
ARG FEATURES=""

WORKDIR /app

# Install build tools
RUN apt-get update && apt-get install -y \
    clang \
    libclang-dev \
    llvm-dev \
    pkg-config \
    libssl-dev \
    git \
    build-essential \
    curl \
    && rm -rf /var/lib/apt/lists/*

# setup SSH for private repositories
RUN mkdir -p -m 0700 ~/.ssh && \
    ssh-keyscan github.com >> ~/.ssh/known_hosts

# copy only dependency files for better caching
COPY Cargo.toml Cargo.lock ./

# TODO create crates folder and move crates there, so we can copy it instead of the individual crates
# copy workspace members:
COPY qa-tools qa-tools
COPY common common
COPY block-indexer block-indexer
COPY log-indexer log-indexer
COPY transaction-dispatcher transaction-dispatcher
COPY key-manager key-manager
COPY check-fork check-fork
COPY coordinator coordinator
COPY sc-event-mocking sc-event-mocking

# SSH mount for private repos
RUN --mount=type=ssh,id=default mkdir -p .cargo && \
    echo '[net]\ngit-fetch-with-cli = true' > .cargo/config.toml && \
    cargo fetch --locked

# two build modes:
# 1) build the entire workspace and reuse among containers, to reduce build time
# 2) build just one crate for speed
# optionally builds with --features if provided
RUN --mount=type=ssh,id=default cargo build --release${JUST_CRATE:+ -p $JUST_CRATE}${FEATURES:+ --features $FEATURES};
# build key-manager always (even if $JUST_CRATE provided), as it might be used by other services
RUN --mount=type=ssh,id=default cargo build --release -p key-manager;

# --- RUNTIME STAGE ---

FROM debian:bookworm-slim AS runtime
ARG CRATE

WORKDIR /app

# install runtime dependencies
RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    && rm -rf /var/lib/apt/lists/*

# copy the crate built binary
COPY --from=builder /app/target/release/${CRATE} /app/${CRATE}
COPY --from=builder /app/target/release/key-manager /app/key-manager

# create directories for data
RUN mkdir -p /app/db/${CRATE}

# if deploying contracts (checked within the script): clone, build and deploy them
ENV FOUNDRY_VERSION=1.1.0
ARG UNION_CONTRACTS_TAG
COPY docker/dockerfile-contracts-setup.sh /app/contracts-setup.sh
RUN --mount=type=ssh,id=default chmod +x /app/contracts-setup.sh && /app/contracts-setup.sh
ENV PATH="/opt/foundry/bin:$PATH"

# set environment variables
ENV RUST_BACKTRACE=1
ENV RUST_LOG=debug

# ENTRYPOINT is handled in docker-compose.yml