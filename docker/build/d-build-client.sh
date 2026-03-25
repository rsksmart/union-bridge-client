#!/usr/bin/env bash

set -e

FEATURES=""
PLATFORM="linux/amd64"
UC_TAG="latest"
HELP=0

show_help() {
  cat << EOF

🔨 Union Services Build Script

Usage: 
  $(basename "$0") [options] [service] [docker compose build arguments...]

Options:
  --features=<list>   Build the workspace with specified features (comma-separated list)
  --platform=<arch>   Specify the target platform (sets PLATFORM environment variable for docker-compose.yml) [default: linux/amd64]
  --tag=<tag>         Specify the image tag for building images (default: latest)
  --help, -h          Show this help

Service Names (optional):
  You can specify a service name to build only that service.
  Service names are defined in your docker-compose.yml file.

All additional arguments (including --no-cache, -d, etc.) are passed directly to 'docker compose' commands.

Notes:
  - --features: You can specify multiple features separated by commas (e.g., --features=anvil,debug). You may need to rebuild to reflect feature changes.
  - Service names: When a service is specified, the build process is optimized to build only that service's crate.
  - --platform: Sets the PLATFORM environment variable used by docker-compose.yml, not passed as a command flag. Defaults to linux/amd64.
  - --tag: Sets the image tag for building images. Defaults to 'latest' if not specified.
  - Standard docker compose arguments like --no-cache, -d, etc. are fully supported.

Examples:
  $(basename "$0")                                                  Build all services
  $(basename "$0") log-indexer                                      Build a specific service
  $(basename "$0") --features=anvil                                 Build with anvil feature  
  $(basename "$0") --features=anvil,debug                           Build with multiple features
  $(basename "$0") --platform=linux/arm64                           Build for ARM64 platform
  $(basename "$0") --tag=v1.0.0                                     Build with custom tag
  $(basename "$0") coordinator --no-cache                           Build coordinator without cache

EOF
  exit 0
}

# Helper function to export environment variables
use_env_vars() {
  local vars_to_export=()
  [[ -n $PLATFORM ]] && vars_to_export+=(PLATFORM)
  [[ -n $UC_TAG ]] && vars_to_export+=(UC_TAG)
  
  if [[ ${#vars_to_export[@]} -gt 0 ]]; then
    echo "Using environment variables: ${vars_to_export[*]}"
    export "${vars_to_export[@]}"
  fi
}

# Parse command line arguments
if [[ $# -gt 0 ]] && ([[ "$1" == "--help" ]] || [[ "$1" == "-h" ]]); then
  show_help
fi

DOCKER_ARGS=()

while [[ $# -gt 0 ]]; do
  case "$1" in
    --features=*)
      FEATURES="${1#*=}"
      ;;
    --platform=*)
      PLATFORM="${1#*=}"
      ;;
    --tag=*)
      UC_TAG="${1#*=}"
      ;;
    -h|--help)
      HELP=1
      ;;
    *)
      # Pass unknown arguments to docker compose
      DOCKER_ARGS+=("$1")
      ;;
  esac
  shift
done

if [[ $HELP -eq 1 ]]; then
  show_help
fi

# Build service containers
build_services() {
  echo "🔨 Building service container(s)..."

  use_env_vars

  cmd=(docker compose build)

  # Get actual service names from docker-compose.yml and check if any argument matches
  local service_found=""
  if command -v docker >/dev/null 2>&1; then
    # Try to get service names dynamically (suppress errors in case docker-compose.yml is invalid)
    local available_services
    if available_services=$(docker compose config --services 2>/dev/null); then
      # Check if any argument matches an available service
      for arg in "${DOCKER_ARGS[@]}"; do
        if echo "$available_services" | grep -q "^${arg}$"; then
          service_found="$arg"
          break
        fi
      done
    fi
  fi

  # Add build arguments
  [[ -n $service_found ]] && cmd+=(--build-arg JUST_CRATE="$service_found")
  [[ -n $FEATURES ]] && cmd+=(--build-arg FEATURES="$FEATURES")
  [[ -n $UC_TAG ]] && cmd+=(--build-arg UC_TAG="$UC_TAG")

  # Add any additional docker compose arguments
  cmd+=("${DOCKER_ARGS[@]}")

  echo "Building with command: ${cmd[@]}"

  "${cmd[@]}"
  echo "✅ Build completed"
}

build_services
