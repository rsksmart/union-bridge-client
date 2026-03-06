#!/usr/bin/env bash

# This script manages the local blockchain stack defined in docker-compose.blockchains.yaml
# It intentionally focuses ONLY on the blockchains stack (bitcoind, anvil, deploy-contracts).

DOCKER_COMPOSE_ARGS=()

# Resolve script directory (for referencing compose files reliably)
SCRIPT_DIR=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" &>/dev/null && pwd)
COMPOSE_FILE="${SCRIPT_DIR}/docker-compose.blockchains.yaml"
ENV_PATH="${SCRIPT_DIR}/.env.local"

# Display help message
print_help() {
  echo "Usage: $0 [OPTIONS] [DOCKER_COMPOSE_ARGS...]"
  echo ""
  echo "Options:"
  echo "  --help                     Display this help message"
  echo "  --fresh                    Tear down local blockchains (and volumes). Can be used standalone or with 'up'"
  echo "  --new-contracts-version    Force rebuild of the 'deploy-contracts' image before running"
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
  echo "  $0 --new-contracts-version up -d    # Rebuild deploy-contracts image and start"
  echo "  $0 down                             # Stop blockchains"
  echo "  $0 ps                               # Check status"
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

# Check env file exists
if [[ ! -f "$ENV_PATH" ]]; then
  echo "Error: .env not found at $ENV_PATH"
  exit 1
fi

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

echo "IS_UP_COMMAND: ${IS_UP_COMMAND} | FRESH: ${FRESH} | NEW_CONTRACTS_VERSION: ${NEW_CONTRACTS_VERSION}"

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
  echo "Removing blockchains-deploy-contracts image"
  docker rmi blockchains-deploy-contracts
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

# Wait for deploy-contracts to finish and verify success
if [[ "${IS_UP_COMMAND}" == true ]]; then
  echo "Waiting for contract deployment to complete..."
  DEPLOY_CONTAINER="deploy-contracts"
  DEPLOY_TIMEOUT=120
  DEPLOY_ELAPSED=0
  DEPLOY_INTERVAL=5

  while [[ $DEPLOY_ELAPSED -lt $DEPLOY_TIMEOUT ]]; do
    DEPLOY_STATUS=$(docker inspect -f '{{.State.Status}}' "$DEPLOY_CONTAINER" 2>/dev/null || echo "not_found")
    if [[ "$DEPLOY_STATUS" == "exited" ]]; then
      DEPLOY_EXIT_CODE=$(docker inspect -f '{{.State.ExitCode}}' "$DEPLOY_CONTAINER" 2>/dev/null)
      if [[ "$DEPLOY_EXIT_CODE" -eq 0 ]]; then
        echo "Contract deployment completed successfully."
      else
        echo "Error: Contract deployment failed with exit code $DEPLOY_EXIT_CODE"
        echo "Last 20 lines from deploy-contracts:"
        docker logs --tail 20 "$DEPLOY_CONTAINER"
        exit 1
      fi
      break
    fi
    sleep "$DEPLOY_INTERVAL"
    DEPLOY_ELAPSED=$((DEPLOY_ELAPSED + DEPLOY_INTERVAL))
    echo "  Still deploying... (${DEPLOY_ELAPSED}s)"
  done

  if [[ $DEPLOY_ELAPSED -ge $DEPLOY_TIMEOUT ]]; then
    echo "Error: Contract deployment timed out after ${DEPLOY_TIMEOUT}s"
    docker logs --tail 20 "$DEPLOY_CONTAINER"
    exit 1
  fi
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
