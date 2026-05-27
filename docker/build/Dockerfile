# docker-compose.yml arg to specify the crate to build and run
ARG CRATE
# --build-arg to specify the Rust features to use (ie. anvil)
ARG FEATURES
# --build-arg to specify just one crate to build in the builder stage
ARG JUST_CRATE
ARG BUILDER_IMAGE=ghcr.io/rsksmart/union-client-builder:rust-1.95-v1

# --- BUILDER STAGE ---

FROM ${BUILDER_IMAGE} AS builder
ARG JUST_CRATE=""
ARG FEATURES=""
ARG EXTRA_BUILD_PACKAGES=""

WORKDIR /app

RUN apt-get update -y && \
    DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends \
        ca-certificates \
        procps \
        ${EXTRA_BUILD_PACKAGES} \
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
COPY op-funding op-funding
COPY protocol-params protocol-params
COPY coordinator coordinator
COPY user-api user-api

ENV RUST_LOG=error
ENV CARGO_TERM_QUIET=true

RUN cargo fetch --locked

# two build modes:
# 1) build the entire workspace and reuse among containers, to reduce build time
# 2) build just one crate for speed
# optionally builds with --features if provided
# we also exclude key-manager from runtime image builds
ARG EXCLUDES="--exclude check-fork --exclude check-fork-zkp --exclude check-fork-tester --exclude key-manager"
# TODO when the zkp feature is re-enabled, use this to build coordinator in an image with amd64 support (required by Risc0 in Macs with M chips) via Dockerfile_coordinator
# EXCLUDES="--exclude coordinator--exclude check-fork --exclude check-fork-zkp --exclude key-manager"
RUN cargo build --workspace --quiet --release --locked ${EXCLUDES}${JUST_CRATE:+ -p $JUST_CRATE}${FEATURES:+ --features $FEATURES} && \
    (rm -rf target/release/incremental 2>/dev/null || true) && \
    (find target/release/deps -name "*.rmeta" -delete 2>/dev/null || true) && \
    (find target/release/build -type f -name "*.d" -delete 2>/dev/null || true)

# --- RUNTIME STAGE ---

FROM debian:bookworm-slim@sha256:67b30a61dc87758f0caf819646104f29ecbda97d920aaf5edc834128ac8493d3 AS runtime
ARG CRATE

# install runtime dependencies
RUN apt-get update -y && \
    DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends \
        ca-certificates \
        gosu \
        procps \
        openssl \
    && apt-get clean \
    && rm -rf /var/lib/apt/lists/*

# copy the crate built binary
COPY --from=builder /app/target/release/${CRATE} /app/${CRATE}
WORKDIR /app

# set environment variables
ENV RUST_BACKTRACE=1
# Default tracing filter: project crates at `debug`, everything else at `info`.
# Override at build time with `--build-arg RUST_LOG="..."`, or per-service at
# run time via the compose `environment:` block (run-time wins). See docs/LOGGING.md.
ARG RUST_LOG="info,\
common=debug,\
block_indexer=debug,\
log_indexer=debug,\
transaction_dispatcher=debug,\
op_funding=debug,\
protocol_params=debug,\
coordinator=debug,\
user_api=debug,\
tarpc=warn,\
alloy_provider=warn,alloy_pubsub=warn,alloy_rpc_client=warn,alloy_json_rpc=warn,\
hyper=warn,hyper_util=warn,h2=warn,\
reqwest=warn,rustls=warn,tower_http=warn,tungstenite=warn"
ENV RUST_LOG=${RUST_LOG}
# [coordinator Specific] Required by Risc0, otherwise it points to ~/.risc0
# it does not harm to have it always defined, so we leave it common
ENV RISC0_HOME=/opt/risc0

COPY docker/build/entrypoint_sh/db-entrypoint.sh /app/db-entrypoint.sh
COPY docker/build/entrypoint_sh/keystore-entrypoint.sh /app/keystore-entrypoint.sh

RUN groupadd -r appuser && useradd -r -g appuser appuser

LABEL org.opencontainers.image.source=https://github.com/rsksmart/union-bridge-client

ENTRYPOINT ["sh", "/app/db-entrypoint.sh"]
