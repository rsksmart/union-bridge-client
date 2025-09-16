#!/usr/bin/env bash

set -e

# TODO Improve receiving parameters and using "docker buildx bake"

TAG="latest"
PLATFORM="linux/amd64"
DRY_RUN=0
HELP=0

show_help() {
  cat << EOF

📥 Union Services Pull from GHCR Script

Usage: 
  $(basename "$0") [options]

Options:
  --tag=<tag>         Specify the image tag to pull [default: latest]
  --platform=<arch>   Specify the target platform [default: linux/amd64]
  --dry-run           Show what would be pulled without actually pulling
  --help, -h          Show this help

Before running this script:
  echo \$GITHUB_REGISTRY_TOKEN | docker login ghcr.io -u <your_user> --password-stdin

Examples:
  $(basename "$0")                           Pull all images with 'latest' tag for linux/amd64
  $(basename "$0") --tag=v1.0.0              Pull all images with 'v1.0.0' tag for linux/amd64
  $(basename "$0") --platform=linux/arm64    Pull all images with 'latest' tag for linux/arm64
  $(basename "$0") --dry-run                 Show what would be pulled
  $(basename "$0") --tag=v1.0.0 --dry-run    Show what would be pulled with custom tag

EOF
  exit 0
}

# Parse command line arguments
while [[ $# -gt 0 ]]; do
  case "$1" in
    --tag=*)
      TAG="${1#*=}"
      ;;
    --platform=*)
      PLATFORM="${1#*=}"
      ;;
    --dry-run)
      DRY_RUN=1
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

if [[ $DRY_RUN -eq 1 ]]; then
  echo "🔍 DRY RUN: Would pull images with tag: $TAG for platform: $PLATFORM"
  echo ""
  echo "Commands that would be executed:"
  echo "  docker pull ghcr.io/rsksmart/union-client-builder:rust-1.86-v1 --platform $PLATFORM"
  echo "  docker pull ghcr.io/rsksmart/union-client-block-indexer:$TAG --platform $PLATFORM"
  echo "  docker pull ghcr.io/rsksmart/union-client-log-indexer:$TAG --platform $PLATFORM"
  echo "  docker pull ghcr.io/rsksmart/union-client-transaction-dispatcher:$TAG --platform $PLATFORM"
  echo "  docker pull ghcr.io/rsksmart/union-client-coordinator:$TAG --platform $PLATFORM"
  echo "  docker pull ghcr.io/rsksmart/union-client-user-api:$TAG --platform $PLATFORM"
  echo ""
  echo "✅ Dry run completed - no images were actually pulled"
else
  echo "📥 Pulling images with tag: $TAG for platform: $PLATFORM"

  # order seems to matter (same order as defined in compose file)
  if [[ "$PLATFORM" == "linux/amd64" ]]; then
    # Default platform - don't specify platform parameter
    docker pull ghcr.io/rsksmart/union-client-builder:rust-1.86-v1
    docker pull ghcr.io/rsksmart/union-client-block-indexer:$TAG
    docker pull ghcr.io/rsksmart/union-client-log-indexer:$TAG
    docker pull ghcr.io/rsksmart/union-client-transaction-dispatcher:$TAG
    docker pull ghcr.io/rsksmart/union-client-coordinator:$TAG
    docker pull ghcr.io/rsksmart/union-client-user-api:$TAG
  else
    # Custom platform - specify platform parameter
    docker pull ghcr.io/rsksmart/union-client-builder:rust-1.86-v1 --platform $PLATFORM
    docker pull ghcr.io/rsksmart/union-client-block-indexer:$TAG --platform $PLATFORM
    docker pull ghcr.io/rsksmart/union-client-log-indexer:$TAG --platform $PLATFORM
    docker pull ghcr.io/rsksmart/union-client-transaction-dispatcher:$TAG --platform $PLATFORM
    docker pull ghcr.io/rsksmart/union-client-coordinator:$TAG --platform $PLATFORM
    docker pull ghcr.io/rsksmart/union-client-user-api:$TAG --platform $PLATFORM
  fi

  echo "✅ All images pulled successfully"
fi
