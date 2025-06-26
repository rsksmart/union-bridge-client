# docker-compose.yml arg to specify the crate to build and run
ARG CRATE
# docker-compose.yml arg to specify the tag of union-contracts repository to use - if defined, contracts will be deployed for that service
ARG UNION_CONTRACTS_TAG
# docker-compose.yml arg to specify the Foundry version - if defined, Foundry will be installed for that service
ARG FOUNDRY_VERSION
# --build-arg to specify the Rust features to use (ie. anvil)
ARG FEATURES
# --build-arg to specify just one crate to build in the builder stage
ARG JUST_CRATE

# --- BUILDER STAGE ---

FROM union-client-builder:rust-1.86-v1 AS builder
ARG JUST_CRATE=""
ARG FEATURES=""

# setup SSH for private repositories
RUN mkdir -p -m 0700 ~/.ssh && \
    ssh-keyscan github.com >> ~/.ssh/known_hosts

WORKDIR /app

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
COPY actors-mocking actors-mocking

RUN apt-get update -y && \
    DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends \
        ca-certificates \
        procps \
    && apt-get clean \
    && rm -rf /var/lib/apt/lists/*

# SSH mount for private repos
RUN --mount=type=ssh,id=default mkdir -p .cargo && \
    echo '[net]\ngit-fetch-with-cli = true' > .cargo/config.toml && \
    cargo fetch # TODO(Jira) re-add --locked in scope of https://rsklabs.atlassian.net/browse/UB-176

# two build modes:
# 1) build the entire workspace and reuse among containers, to reduce build time
# 2) build just one crate for speed
# optionally builds with --features if provided
# check-fork needs amd64 architecture due to Risc0, so we exclude it and also coordinator, which is built in a separate stage, as it has it as dependency
# we also exclude key-manager, which is built always in the next step
RUN --mount=type=ssh,id=default cargo build --workspace --exclude coordinator --exclude check-fork --exclude check-fork-zkp --exclude key-manager --release${JUST_CRATE:+ -p $JUST_CRATE}${FEATURES:+ --features $FEATURES};
# build key-manager always (even if $JUST_CRATE provided), as it might be used by other services
RUN --mount=type=ssh,id=default cargo build --release -p key-manager;

# --- RUNTIME STAGE ---

FROM debian:bookworm-slim AS runtime
ARG CRATE

# install runtime dependencies
RUN apt-get update -y && \
    DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends \
        ca-certificates \
        procps \
    && apt-get clean \
    && rm -rf /var/lib/apt/lists/*

# copy the crate built binary
COPY --from=builder /app/target/release/${CRATE} /app/${CRATE}
# copy always, but only used by transaction-dispatcher
COPY --from=builder /app/target/release/key-manager /app/key-manager

# if Foundry required (checked within the script): install it
ARG FOUNDRY_VERSION
COPY docker/dockerfile-foundry-setup.sh /app/foundry-setup.sh
RUN chmod +x /app/foundry-setup.sh && /app/foundry-setup.sh
# required by contracts-setup and also by actors-mocking that spins up anvil
ENV PATH="/opt/foundry/bin:$PATH"

# if contracts required (checked within the script): clone, build and deploy them
ARG UNION_CONTRACTS_TAG
COPY docker/dockerfile-contracts-setup.sh /app/contracts-setup.sh
RUN --mount=type=ssh,id=default chmod +x /app/contracts-setup.sh && /app/contracts-setup.sh

# set environment variables
ENV RUST_BACKTRACE=1
ENV RUST_LOG=debug

# ENTRYPOINT is handled in docker-compose.yml