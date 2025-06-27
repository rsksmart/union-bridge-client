# --build-arg to specify the Rust features to use (ie. anvil)
ARG FEATURES

# --- BUILDER STAGE ---

FROM union-client-builder:rust-1.86-risc0-v1 AS builder
ARG FEATURES

# setup SSH for private repositories
RUN mkdir -p -m 0700 ~/.ssh && \
    ssh-keyscan github.com >> ~/.ssh/known_hosts

WORKDIR /app

# copy only dependency files for better caching
COPY Cargo.toml ./
COPY Cargo.lock ./

# TODO create crates folder and move crates there, so we can copy it instead of the individual crates
# copy workspace members:
COPY common common
COPY block-indexer block-indexer
COPY log-indexer log-indexer
COPY transaction-dispatcher transaction-dispatcher
COPY key-manager key-manager
COPY check-fork check-fork
COPY coordinator coordinator
COPY actors-mocking actors-mocking

# SSH mount for private repos
RUN --mount=type=ssh,id=default mkdir -p .cargo && \
    echo '[net]\ngit-fetch-with-cli = true' > .cargo/config.toml && \
    cargo fetch # TODO(Jira) re-add --locked in scope of https://rsklabs.atlassian.net/browse/UB-176

# required by Risc0, otherwise it points to ~/.risc0
ENV RISC0_HOME=/opt/risc0

RUN --mount=type=ssh,id=default cargo build --release -p coordinator ${FEATURES:+ --features $FEATURES};

# --- RUNTIME STAGE ---

FROM debian:bookworm-slim AS runtime

# install runtime dependencies
RUN apt-get update -y && \
    DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends \
        ca-certificates \
        procps \
    && apt-get clean \
    && rm -rf /var/lib/apt/lists/*

# copy the crate built binary
COPY --from=builder /app/target/release/coordinator /app/coordinator

# set environment variables
ENV RUST_BACKTRACE=1
ENV RUST_LOG=debug

# ENTRYPOINT is handled in docker-compose.yml