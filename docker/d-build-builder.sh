#!/usr/bin/env bash

set -e

show_help() {
  cat << EOF

🔨 Union Builder Image Build Script

Usage: 
  $(basename "$0") [docker build arguments...]

This script builds the Union Client builder images.
All arguments are passed directly to 'docker build'.

Examples:
  $(basename "$0")                                              Build with default settings
  $(basename "$0") --platform=linux/arm64                       Build for ARM64 platform
  $(basename "$0") --no-cache                                   Build without cache
  $(basename "$0") --platform=linux/amd64,linux/arm64 --push    Build multi-platform and push

EOF
  exit 0
}

# Check for help
if [[ "$1" == "--help" ]] || [[ "$1" == "-h" ]]; then
  show_help
fi

# Build builder images
echo "🔨 Building Union Client Builder images..."

# Build standard builder image
cmd=(docker build --ssh default "$@" -t ghcr.io/rsksmart/union-client-builder:rust-1.86-v1 -f Dockerfile_builder .)
echo "🔨 Building Standard Builder image with command: ${cmd[@]}"
"${cmd[@]}"

# NOTE: in MacOS with M chips, when the zkp feature is re-enabled, coordinator has to be built on top of a linux/amd64 image (required by Risc0 in Macs with M chips) via Dockerfile_coordinator

echo "✅ Base images ready"
