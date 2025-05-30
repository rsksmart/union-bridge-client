# Accepts the crate name and optional features to build
ARG CRATE
ARG FEATURES=""

FROM rust:1.86-slim-bookworm AS builder
ARG CRATE
ARG FEATURES

WORKDIR /app

# install git and SSH for fetching private/public git dependencies
RUN apt-get update && apt-get install -y \
    clang \
    libclang-dev \
    llvm-dev \
    pkg-config \
    libssl-dev \
    git \
    build-essential \
    && rm -rf /var/lib/apt/lists/*

# setup SSH for private repositories
RUN mkdir -p -m 0700 ~/.ssh && \
    ssh-keyscan github.com >> ~/.ssh/known_hosts

# copy only dependency files for caching
COPY Cargo.toml Cargo.lock ./

# copy workspace:
# - assumes correct usage of .dockerignore to exclude unnecessary files)
# - this invalidates cache only when source changes
COPY . .

# build and cache dependencies only
RUN mkdir -p .cargo && \
    echo '[net]\ngit-fetch-with-cli = true' > .cargo/config.toml
RUN --mount=type=ssh,id=default cargo fetch

# build with SSH mounting to be able to clone private repository dependencies
# optionally builds with --features if FEATURES is non-empty
RUN --mount=type=ssh,id=default \
    cargo build --release -p ${CRATE} ${FEATURES:+--features ${FEATURES}}

# create a smaller runtime image
FROM debian:bookworm-slim
ARG CRATE

WORKDIR /app

# Install runtime dependencies
RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    && rm -rf /var/lib/apt/lists/*

# copy the built binary
COPY --from=builder /app/target/release/${CRATE} /app/${CRATE}

# create directories for data
RUN mkdir -p /app/db/${CRATE}

# set environment variables
ENV RUST_BACKTRACE=1
ENV RUST_LOG=debug

# ENTRYPOINT is handled in docker-compose.yml