ARG CRATE

FROM rust:1.86-slim-bookworm AS builder
ARG CRATE

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
    touch ~/.ssh/known_hosts && \
    ssh-keyscan github.com >> ~/.ssh/known_hosts

# add the SSH key and set permissions
RUN mkdir -p -m 0700 /root/.ssh

# copy only Cargo.toml and Cargo.lock files first for better caching
COPY Cargo.toml Cargo.lock ./

# assumes correct usage of .dockerignore to exclude unnecessary files
COPY ../ .

# build and cache dependencies only
RUN --mount=type=ssh,id=default \
    mkdir -p .cargo && \
    echo '[net]\ngit-fetch-with-cli = true' > .cargo/config.toml && \
    cargo fetch

# now copy the entire project and build
COPY . .

# Build with SSH mounting
RUN --mount=type=ssh,id=default \
    cargo build --release -p ${CRATE}

# create a smaller runtime image
FROM debian:bookworm-slim
ARG CRATE

WORKDIR /app

# Install dependencies
RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    && rm -rf /var/lib/apt/lists/*

# copy the built binary
COPY --from=builder /app/target/release/${CRATE} /app/${CRATE}

# copy configuration files
COPY config /app/config
# uses the template log4rs.yaml
# if you want a real file you must:
#   1. copy it instead
#   2. use '"--logger-path", "/app/log4rs.yaml"' in RUN
COPY log4rs.yaml /app/log4rs.yaml

# create directories for data
RUN mkdir -p /app/db/${CRATE}

# set environment variables
ENV RUST_BACKTRACE=1
ENV RUST_LOG=debug

# ENTRYPOINT in docker-compose.yml