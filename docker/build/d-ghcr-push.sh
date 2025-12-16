#!/usr/bin/env bash

set -e

# TODO Improve receiving parameters and using "docker buildx bake"

UC_TAG="latest"
DRY_RUN=0
HELP=0

show_help() {
  cat << EOF

🚀 Union Services Push to GHCR Script

Usage: 
  $(basename "$0") [options]

Options:
  --tag=<tag>         Specify the image tag to push [default: latest]
  --dry-run           Show what would be pushed without actually pushing
  --help, -h          Show this help

Before running this script:
  echo \$GITHUB_REGISTRY_TOKEN | docker login ghcr.io -u <your_user> --password-stdin

Examples:
  $(basename "$0")                    Push all images with 'latest' tag
  $(basename "$0") --tag=v1.0.0       Push all images with 'v1.0.0' tag
  $(basename "$0") --dry-run           Show what would be pushed
  $(basename "$0") --tag=v1.0.0 --dry-run  Show what would be pushed with custom tag

EOF
  exit 0
}

# Parse command line arguments
while [[ $# -gt 0 ]]; do
  case "$1" in
    --tag=*)
      UC_TAG="${1#*=}"
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
  echo "🔍 DRY RUN: Would push images with tag: $UC_TAG"
  echo ""
  echo "Commands that would be executed:"
  echo "  docker push ghcr.io/rsksmart/union-client-builder:rust-1.86-v1"
  echo "  docker push ghcr.io/rsksmart/union-client-block-indexer:$UC_TAG"
  echo "  docker push ghcr.io/rsksmart/union-client-log-indexer:$UC_TAG"
  echo "  docker push ghcr.io/rsksmart/union-client-coordinator:$UC_TAG"
  echo "  docker push ghcr.io/rsksmart/union-client-user-api:$UC_TAG"
  echo ""
  echo "✅ Dry run completed - no images were actually pushed"
else
  echo "🚀 Pushing images with tag: $UC_TAG"

  # order seems to matter (same order as defined in compose file)
  # Note: Images should already be tagged by the build scripts
  docker push ghcr.io/rsksmart/union-client-builder:rust-1.86-v1
  docker push ghcr.io/rsksmart/union-client-block-indexer:$UC_TAG
  docker push ghcr.io/rsksmart/union-client-log-indexer:$UC_TAG
  docker push ghcr.io/rsksmart/union-client-coordinator:$UC_TAG
  docker push ghcr.io/rsksmart/union-client-user-api:$UC_TAG

  echo "✅ All images pushed successfully"
fi
