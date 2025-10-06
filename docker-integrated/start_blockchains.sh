#!/usr/bin/env bash

# This script manages the local blockchain stack defined in docker-compose.blockchains.yaml
# It intentionally focuses ONLY on the blockchains stack (bitcoind, anvil, deploy-contracts).
# Operators should be managed with start_operators.sh.

DOCKER_COMPOSE_ARGS=()

# Resolve script directory (for referencing compose files reliably)
SCRIPT_DIR=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" &>/dev/null && pwd)
COMPOSE_FILE="${SCRIPT_DIR}/docker-compose.blockchains.yaml"

# Display help message
print_help() {
  echo "Usage: $0 [OPTIONS] [DOCKER_COMPOSE_ARGS...]"
  echo ""
  echo "Options:"
  echo "  --help                        Display this help message"
  echo "  --env <ENV>                   Set environment (local only)"
  echo "  --fresh                       Tear down local blockchains (and volumes). Can be used standalone or with 'up'"
  echo "  --new-contracts-version    Force rebuild of the 'deploy-contracts' image before running"
  echo ""
  echo "Common Docker Compose Arguments can be used, examples:"
  echo "  up                            Create and start containers"
  echo "  down                          Stop and remove containers, networks"
  echo "  ps                            List containers"
  echo "  logs                          View output from containers"
  echo "  --force-recreate              Recreate containers even if configuration and image haven't changed"
  echo "  Note: Building from source is not supported in this script. Only registry images should be used (except 'deploy-contracts' build stage)."
  echo ""
  echo "Examples:"
  echo "  $0 --env local up -d                               # Start local blockchains"
  echo "  $0 --env local --fresh up -d                       # Clean and start local blockchains"
  echo "  $0 --env local --new-contracts-version up -d    # Rebuild deploy-contracts image and start"
  echo "  $0 --env local down                                # Stop blockchains"
  echo "  $0 --env local ps                                  # Check status"
  echo ""
  echo "Any additional arguments will be passed directly to docker compose."
  exit 0
}

FRESH=false
NEW_CONTRACTS_VERSION=false

# Parse args
while [[ $# -gt 0 ]]; do
  case $1 in
    --help)
      print_help
      ;;
    --env)
      if [[ "$2" == "local" ]]; then
        ENV_FILE=".env.local"
      else
        echo "Invalid environment. Only 'local' is supported"
        exit 1
      fi
      shift 2
      ;;
    --fresh)
      FRESH=true
      shift
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

if [[ -z "$ENV_FILE" ]]; then
  echo "Usage: $0 --env <local> [--fresh] [compose args]"
  exit 1
fi

# Disallow builds (except the deploy-contracts service that is defined with build in compose)
for arg in "${DOCKER_COMPOSE_ARGS[@]}"; do
  if [[ "$arg" == "build" || "$arg" == "--build" || "$arg" == "-b" ]]; then
    echo "Error: Building arbitrary images from source is not supported with this script."
    exit 1
  fi
done

ENV_PATH="${SCRIPT_DIR}/$ENV_FILE"

# Only the local env has a blockchains stack here. For testnet, we no-op with a message.
if [[ "$ENV_FILE" != ".env.local" ]]; then
  echo "No local blockchain stack to manage for '$ENV_FILE'. If you intended to run local blockchains, use --env local."
  exit 0
fi

# Check if we're using the 'up' command
IS_UP_COMMAND=false
for arg in "${DOCKER_COMPOSE_ARGS[@]}"; do
  if [[ "$arg" == "up" ]]; then
    IS_UP_COMMAND=true
    break
  fi
done

echo "IS_UP_COMMAND: ${IS_UP_COMMAND} | FRESH: ${FRESH} | NEW_CONTRACTS_VERSION: ${NEW_CONTRACTS_VERSION}"

# If requested, clean local blockchains regardless of the main command
if [[ "${FRESH}" == true ]]; then
  echo "Cleaning local blockchains stack (down -v)..."
  cmd="docker compose --env-file \"$ENV_PATH\" -f \"$COMPOSE_FILE\" down --volumes || true"
  echo "Running: $cmd"
  eval "$cmd"
fi

# Optionally rebuild the deploy-contracts image before proceeding
if [[ "${NEW_CONTRACTS_VERSION}" == true ]]; then
  echo "Forcing rebuild of 'deploy-contracts' image..."
  cmd="docker compose --env-file \"$ENV_PATH\" -f \"$COMPOSE_FILE\" down || true"
  echo "Running: $cmd"
  eval "$cmd"
  echo "Removing blockchains-deploy-contracts image"
  docker rmi blockchains-deploy-contracts
fi

BITCOIND_CONTAINER="bitcoind"
RUNNING_COUNT=$(docker compose --env-file "$ENV_PATH" -f "$COMPOSE_FILE" --profile local ps --status running -q ${BITCOIND_CONTAINER} anvil | wc -l | tr -d ' ')

echo "Detected $RUNNING_COUNT running containers in the local blockchains stack."

if [[ "${NEW_CONTRACTS_VERSION}" == false && "${IS_UP_COMMAND}" == true && "${RUNNING_COUNT}" -ge 2 ]]; then
  echo "Local blockchains stack already running; skipping 'up'. Run 'down' to start again" && exit 0
fi

# Finally, run the requested docker compose command
docker compose --env-file "$ENV_PATH" -f "$COMPOSE_FILE" --profile local "${DOCKER_COMPOSE_ARGS[@]}"

# If using 'up' command after a fresh teardown, create the Bitcoin wallet and deploy contracts
if [[ "${IS_UP_COMMAND}" == true && "${FRESH}" == true ]]; then
  echo "Waiting 5 seconds for bitcoind initialization..."
  sleep 5

  # concrete filename, to avoid SC1090 complain
  source "${SCRIPT_DIR}/.env.local"

  # Create wallet
  echo "Creating wallet 'mainwallet' in ${BITCOIND_CONTAINER}..."
  docker exec "${BITCOIND_CONTAINER}" bitcoin-cli -regtest -rpcuser="${BITCOIND_USER}" -rpcpassword="${BITCOIND_PASSWORD}" createwallet mainwallet

  echo
  echo "Restarting ${BITCOIND_CONTAINER} after wallet creation."
  docker restart "${BITCOIND_CONTAINER}" 1>/dev/null
fi

echo
echo "Blockchains ready!!!"
