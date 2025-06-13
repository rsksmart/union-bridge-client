#!/usr/bin/env bash

# Build the builder image
echo "Building Builder image..."
docker build --ssh default -t union-client-builder:rust-1.86-a -f Dockerfile_builder .
docker build --ssh default --platform linux/amd64 -t union-client-builder:rust-1.86-risc0-a -f Dockerfile_builder_risc0 .
echo "Builder image ready..."