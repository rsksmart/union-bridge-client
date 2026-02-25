#!/usr/bin/env bash

# This script manages the local blockchain stack defined in docker-compose.blockchains.yaml
# It intentionally focuses ONLY on the blockchains stack (bitcoind, anvil, deploy-contracts).

DOCKER_COMPOSE_ARGS=()

# Resolve script directory (for referencing compose files reliably)
SCRIPT_DIR=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" &>/dev/null && pwd)
COMPOSE_FILE="${SCRIPT_DIR}/docker-compose.blockchains.yaml"
ENV_PATH="${SCRIPT_DIR}/.env.local"

# Contracts image: ghcr.io/temp-rsk/deploy-contracts (tag from CONTRACTS_IMAGE_TAG)
CONTRACTS_IMAGE_BASE="ghcr.io/temp-rsk/deploy-contracts"

# Display help message
print_help() {
  echo "Usage: $0 [OPTIONS] [DOCKER_COMPOSE_ARGS...]"
  echo ""
  echo "Options:"
  echo "  --help                     Display this help message"
  echo "  --fresh                    Tear down local blockchains (and volumes). Can be used standalone or with 'up'"
  echo "  --contracts-tag TAG        Use deploy-contracts image tag (e.g. v0.2.0-alpha.1 or local-build)"
  echo "                             Default: from .env.local CONTRACTS_IMAGE_TAG, or local-build"
  echo "  --new-contracts-version   Force rebuild of the 'deploy-contracts' image before running"
  echo ""
  echo "Common Docker Compose Arguments can be used, examples:"
  echo "  up                         Create and start containers"
  echo "  down                       Stop and remove containers, networks"
  echo "  ps                         List containers"
  echo "  logs                       View output from containers"
  echo "  --force-recreate           Recreate containers even if configuration and image haven't changed"
  echo ""
  echo "Examples:"
  echo "  $0 up -d                            # Start local blockchains"
  echo "  $0 --fresh up -d                    # Clean and start local blockchains"
  echo "  $0 --contracts-tag v0.2.0-alpha.1 up -d   # Use registry image (must exist locally)"
  echo "  $0 --new-contracts-version up -d    # Rebuild deploy-contracts image and start"
  echo "  $0 down                             # Stop blockchains"
  echo "  $0 ps                               # Check status"
  echo ""
  echo "Any additional arguments will be passed directly to docker compose."
  exit 0
}

FRESH=false
NEW_CONTRACTS_VERSION=false
CONTRACTS_TAG_ARG=""

# Parse args
while [[ $# -gt 0 ]]; do
  case $1 in
    --help)
      print_help
      ;;
    --fresh)
      FRESH=true
      shift
      ;;
    --contracts-tag)
      if [[ $# -lt 2 ]]; then
        echo "Error: --contracts-tag requires a value (e.g. v0.2.0-alpha.1 or local-build)"
        exit 1
      fi
      CONTRACTS_TAG_ARG="$2"
      shift 2
      ;;
    --new-contracts-version)
      NEW_CONTRACTS_VERSION=true
      shift
      ;;
    *)
      DOCKER_COMPOSE_ARGS+=("$1")
      shift
      ;;
  esac
done

# Check env file exists
if [[ ! -f "$ENV_PATH" ]]; then
  echo "Error: .env not found at $ENV_PATH"
  exit 1
fi

# Resolve CONTRACTS_IMAGE_TAG: --new-contracts-version forces local-build; else --contracts-tag; else .env.local; else local-build
if [[ "${NEW_CONTRACTS_VERSION}" == true ]]; then
  CONTRACTS_IMAGE_TAG="local-build"
elif [[ -n "$CONTRACTS_TAG_ARG" ]]; then
  CONTRACTS_IMAGE_TAG="$CONTRACTS_TAG_ARG"
else
  CONTRACTS_IMAGE_TAG=$(grep -E "^CONTRACTS_IMAGE_TAG=" "$ENV_PATH" 2>/dev/null | cut -d= -f2- | tr -d '"' | tr -d "'" | sed 's/^[[:space:]]*//;s/[[:space:]]*$//')
  if [[ -z "$CONTRACTS_IMAGE_TAG" ]]; then
    CONTRACTS_IMAGE_TAG="local-build"
  fi
fi
export CONTRACTS_IMAGE_TAG

# Disallow builds (except the deploy-contracts service that is defined with build in compose)
for arg in "${DOCKER_COMPOSE_ARGS[@]}"; do
  if [[ "$arg" == "build" || "$arg" == "--build" || "$arg" == "-b" ]]; then
    echo "Error: Building arbitrary images from source is not supported with this script."
    exit 1
  fi
done

# Check if we're using the 'up' command
IS_UP_COMMAND=false
for arg in "${DOCKER_COMPOSE_ARGS[@]}"; do
  if [[ "$arg" == "up" ]]; then
    IS_UP_COMMAND=true
    break
  fi
done

echo "IS_UP_COMMAND: ${IS_UP_COMMAND} | FRESH: ${FRESH} | NEW_CONTRACTS_VERSION: ${NEW_CONTRACTS_VERSION} | CONTRACTS_IMAGE_TAG: ${CONTRACTS_IMAGE_TAG}"

# Guard: when using a registry tag (not local-build), image must exist — no silent build fallback
if [[ "${IS_UP_COMMAND}" == true && "${CONTRACTS_IMAGE_TAG}" != "local-build" ]]; then
  CONTRACTS_IMAGE="${CONTRACTS_IMAGE_BASE}:${CONTRACTS_IMAGE_TAG}"
  if ! docker image inspect "$CONTRACTS_IMAGE" >/dev/null 2>&1; then
    echo "Error: Contracts image '$CONTRACTS_IMAGE' not found."
    echo "Pull it first (e.g. docker pull $CONTRACTS_IMAGE) or use --contracts-tag local-build to build from source."
    exit 1
  fi
fi

# If requested, clean local blockchains regardless of the main command
if [[ "${FRESH}" == true ]]; then
  echo "Cleaning local blockchains stack (down -v)..."
  cmd="docker compose -p blockchains --env-file \"$ENV_PATH\" -f \"$COMPOSE_FILE\" down --volumes || true"
  echo "Running: $cmd"
  eval "$cmd"
fi

# Optionally rebuild the deploy-contracts image before proceeding
if [[ "${NEW_CONTRACTS_VERSION}" == true ]]; then
  echo "Forcing rebuild of 'deploy-contracts' image..."
  cmd="docker compose -p blockchains --env-file \"$ENV_PATH\" -f \"$COMPOSE_FILE\" down || true"
  echo "Running: $cmd"
  eval "$cmd"
  CONTRACTS_IMAGE="${CONTRACTS_IMAGE_BASE}:local-build"
  echo "Removing ${CONTRACTS_IMAGE} image"
  docker rmi "${CONTRACTS_IMAGE}" 2>/dev/null || true
fi

BITCOIND_CONTAINER="bitcoind"
RUNNING_COUNT=$(docker compose -p blockchains --env-file "$ENV_PATH" -f "$COMPOSE_FILE" --profile local ps --status running -q ${BITCOIND_CONTAINER} anvil | wc -l | tr -d ' ')

echo "Detected $RUNNING_COUNT running containers in the local blockchains stack."

if [[ "${NEW_CONTRACTS_VERSION}" == false && "${IS_UP_COMMAND}" == true && "${RUNNING_COUNT}" -ge 2 ]]; then
  echo "Local blockchains stack already running; skipping 'up'. Run 'down' to start again" && exit 0
fi

# Finally, run the requested docker compose command
if ! docker compose -p blockchains --env-file "$ENV_PATH" -f "$COMPOSE_FILE" --profile local "${DOCKER_COMPOSE_ARGS[@]}"; then
  echo "Error: docker compose command failed"
  exit 1
fi

# If using 'up' command after a fresh teardown, create the Bitcoin wallet and deploy contracts
if [[ "${IS_UP_COMMAND}" == true && "${FRESH}" == true ]]; then
  echo "Waiting 5 seconds for bitcoind initialization..."
  sleep 5

  source "${ENV_PATH}"

  # Create wallet
  echo "Creating wallet 'mainwallet' in ${BITCOIND_CONTAINER}..."
  if ! docker exec "${BITCOIND_CONTAINER}" bitcoin-cli -regtest -rpcuser="${BITCOIND_USER}" -rpcpassword="${BITCOIND_PASSWORD}" createwallet mainwallet; then
    echo "Error: Failed to create wallet 'mainwallet'"
    exit 1
  fi

  echo
  echo "Restarting ${BITCOIND_CONTAINER} after wallet creation."
  if ! docker restart "${BITCOIND_CONTAINER}" 1>/dev/null; then
    echo "Error: Failed to restart ${BITCOIND_CONTAINER}"
    exit 1
  fi
fi

echo
echo "Blockchains ready!!!"
