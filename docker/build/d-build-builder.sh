#!/usr/bin/env bash

set -e

# Default tag and platform
UC_TAG="rust-1.91-v1"
PLATFORM="linux/amd64"

show_help() {
  cat << EOF

🔨 Union Builder Image Build Script

Usage: 
  $(basename "$0") [options] [docker build arguments...]

Options:
  --tag=UC_TAG                       Tag for the builder image (default: rust-1.91-v1)
  --platform=PLATFORM             Target platform (default: linux/amd64)
  --help, -h                      Show this help message

This script builds the Union Client builder images.
All arguments are passed directly to 'docker build'.

Examples:
  $(basename "$0")                                              Build with default settings
  $(basename "$0") --tag=rust-1.91-v1                          Build with custom tag
  $(basename "$0") --platform=linux/arm64                       Build for ARM64 platform
  $(basename "$0") --no-cache                                   Build without cache
  $(basename "$0") --platform=linux/amd64,linux/arm64 --push    Build multi-platform and push

EOF
  exit 0
}

# Parse arguments
DOCKER_ARGS=()
while [[ $# -gt 0 ]]; do
  case $1 in
    --tag=*)
      UC_TAG="${1#*=}"
      shift
      ;;
    --platform=*)
      PLATFORM="${1#*=}"
      shift
      ;;
    --help|-h)
      show_help
      ;;
    *)
      DOCKER_ARGS+=("$1")
      shift
      ;;
  esac
done

# Build builder images
echo "🔨 Building Union Client Builder images with tag: $UC_TAG and platform: $PLATFORM"

# Build standard builder image
cmd=(docker build --platform "$PLATFORM" "${DOCKER_ARGS[@]}" -t "ghcr.io/rsksmart/union-client-builder:$UC_TAG" -f Dockerfile_builder .)
echo "🔨 Building Standard Builder image with command: ${cmd[@]}"
"${cmd[@]}"

# NOTE: in MacOS with M chips, when the zkp feature is re-enabled, coordinator has to be built on top of a linux/amd64 image (required by Risc0 in Macs with M chips) via Dockerfile_coordinator

echo "✅ Base images ready"
