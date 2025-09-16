#!/usr/bin/env bash

set -e

PLATFORM="linux/amd64"
HELP=0

show_help() {
  cat << EOF

🚀 Union Multi-Client Setup Script

Usage: 
  $(basename "$0") [options]

Options:
  --platform=<arch>   Specify the target platform [default: linux/amd64]
  --help, -h          Show this help

This script starts 4 instances of the union client with different ports:
  - uc-1: BITVMX_PORT=22222
  - uc-2: BITVMX_PORT=33333  
  - uc-3: BITVMX_PORT=44444
  - uc-4: BITVMX_PORT=55554

Examples:
  $(basename "$0")                           Start 4 clients for linux/amd64
  $(basename "$0") --platform=linux/arm64    Start 4 clients for linux/arm64

EOF
  exit 0
}

# Parse command line arguments
while [[ $# -gt 0 ]]; do
  case "$1" in
    --platform=*)
      PLATFORM="${1#*=}"
      ;;
    -h|--help)
      HELP=1
      ;;
    *)
      echo "Unknown option: $1"
      show_help
      ;;
  esac
  shift
done

if [[ $HELP -eq 1 ]]; then
  show_help
fi

echo "🚀 Starting 4 union client instances for platform: $PLATFORM"

ROOTSTOCK_URL=ws://host.docker.internal:8545 BITVMX_HOST=host.docker.internal BITVMX_PORT=22222 PLATFORM=$PLATFORM docker compose -p uc-1 -f docker-compose.yml up -d
ROOTSTOCK_URL=ws://host.docker.internal:8545 BITVMX_HOST=host.docker.internal BITVMX_PORT=33333 PLATFORM=$PLATFORM docker compose -p uc-2 -f docker-compose.yml up -d
ROOTSTOCK_URL=ws://host.docker.internal:8545 BITVMX_HOST=host.docker.internal BITVMX_PORT=44444 PLATFORM=$PLATFORM docker compose -p uc-3 -f docker-compose.yml up -d
ROOTSTOCK_URL=ws://host.docker.internal:8545 BITVMX_HOST=host.docker.internal BITVMX_PORT=55554 PLATFORM=$PLATFORM docker compose -p uc-4 -f docker-compose.yml up -d

echo "✅ All 4 union client instances started successfully"
