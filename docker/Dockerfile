ARG CRATE
ARG UNION_CONTRACTS_TAG
ARG FEATURES=""
ARG JUST_CRATE=""

# --- BUILDER STAGE ---

FROM rust:1.86-slim-bookworm AS builder

ARG CRATE
ARG FEATURES=""
ARG JUST_CRATE=""

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

# copy only dependency files for caching
COPY Cargo.toml Cargo.lock ./

# copy full workspace:
# - assumes correct usage of .dockerignore to exclude unnecessary files)
# - this invalidates cache only when source changes
COPY . .

# build and cache cargo dependencies only
RUN mkdir -p .cargo && \
    echo '[net]\ngit-fetch-with-cli = true' > .cargo/config.toml

# SSH mount for private repos
RUN --mount=type=ssh,id=default cargo fetch

# build the entire workspace and reuse among containers, to reduce build time
# optionally builds with --features if provided
RUN --mount=type=ssh,id=default bash -c '\
  if [ -z "$JUST_CRATE" ]; then \
    cargo build --release '"${FEATURES:+--features $FEATURES}"'; \
  fi'

# build just one crate for speed
# optionally builds with --features if provided
RUN --mount=type=ssh,id=default bash -c '\
  if [ -n "$JUST_CRATE" ]; then \
    cargo build --release -p "$JUST_CRATE" '"${FEATURES:+--features $FEATURES}"'; \
  fi'

# --- RUNTIME STAGE ---

FROM debian:bookworm-slim
ARG CRATE
ARG UNION_CONTRACTS_TAG
LABEL contracts_tag=${UNION_CONTRACTS_TAG}

WORKDIR /app

# install runtime dependencies
RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    && rm -rf /var/lib/apt/lists/*

# copy the crate built binary
COPY --from=builder /app/target/release/${CRATE} /app/${CRATE}

# create directories for data
RUN mkdir -p /app/db/${CRATE}

# if deploying contracts (checked within the script): clone, build and deploy them
COPY docker/contracts-setup.sh /app/contracts-setup.sh
ENV FOUNDRY_VERSION=1.1.0
RUN chmod +x /app/contracts-setup.sh
RUN --mount=type=ssh,id=default /app/contracts-setup.sh
ENV PATH="/opt/foundry/bin:$PATH"

# set environment variables
ENV RUST_BACKTRACE=1
ENV RUST_LOG=debug

# ENTRYPOINT is handled in docker-compose.yml