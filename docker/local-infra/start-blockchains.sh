#!/usr/bin/env bash

# This script manages the local blockchain stack defined in docker-compose.blockchains.yaml
# It intentionally focuses ONLY on the blockchains stack (bitcoind, anvil, deploy-contracts).

DOCKER_COMPOSE_ARGS=()

# Resolve script directory (for referencing compose files reliably)
SCRIPT_DIR=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" &>/dev/null && pwd)
COMPOSE_FILE="${SCRIPT_DIR}/docker-compose.blockchains.yaml"
ENV_PATH="${SCRIPT_DIR}/.env.local"

CONTRACTS_IMAGE_BASE="ghcr.io/rsksmart/union-bridge-contracts"
CONTRACTS_TAG_LOCAL_BUILD="local-build"

# Display help message
print_help() {
  echo "Usage: $0 [OPTIONS] [DOCKER_COMPOSE_ARGS...]"
  echo ""
  echo "Options:"
  echo "  --help                     Display this help message"
  echo "  --fresh                    Tear down local blockchains (and volumes). Can be used standalone or with 'up'"
  echo "  --contracts-tag TAG         Override contracts image tag (e.g. v0.2.0-alpha.1 or ${CONTRACTS_TAG_LOCAL_BUILD})"
  echo ""
  echo "Contracts image:"
  echo "  Default: derived from Cargo.toml (union-contracts tag) — pulls from ${CONTRACTS_IMAGE_BASE}"
  echo "  Override: use --contracts-tag flag"
  echo "    ${CONTRACTS_TAG_LOCAL_BUILD}  → build from CONTRACTS_CONTEXT_PATH (e.g. for contract development); use --fresh for clean deploy when contracts change"
  echo "    <tag>       → use that registry tag (always pulls; if digest or tag changed, runs fresh)"
  echo ""
  echo "Common Docker Compose Arguments can be used, examples:"
  echo "  up                         Create and start containers"
  echo "  down                       Stop and remove containers, networks"
  echo "  ps                         List containers"
  echo "  logs                       View output from containers"
  echo "  --force-recreate           Recreate containers even if configuration and image haven't changed"
  echo ""
  echo "Examples:"
  echo "  $0 up -d                            # Start (uses contracts version from Cargo.toml)"
  echo "  $0 --fresh up -d                    # Clean and start local blockchains"
  echo "  $0 --contracts-tag ${CONTRACTS_TAG_LOCAL_BUILD} up -d # Build deploy-contracts from local path"
  echo "  $0 --contracts-tag v0.2.0-alpha.1 up -d   # Use specific registry tag"
  echo "  $0 down                             # Stop blockchains"
  echo "  $0 ps                               # Check status"
  echo ""
  echo "Any additional arguments will be passed directly to docker compose."
  exit 0
}

FRESH=false
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
      if [[ $# -lt 2 || -z "${2:-}" ]]; then
        echo "Error: --contracts-tag requires a non-empty value (e.g. v0.2.0-alpha.1 or ${CONTRACTS_TAG_LOCAL_BUILD})"
        exit 1
      fi
      CONTRACTS_TAG_ARG="$2"
      shift 2
      ;;
    *)
      DOCKER_COMPOSE_ARGS+=("$1")
      shift
      ;;
  esac
done

# Check env file exists
if [[ ! -f "$ENV_PATH" ]]; then
  echo "Error: env file not found at $ENV_PATH"
  exit 1
fi

source "${ENV_PATH}"

wait_for_bitcoind_rpc() {
  local timeout_secs="${1:-60}"
  local elapsed=0

  echo "Waiting for ${BITCOIND_CONTAINER} RPC..."
  while [[ "${elapsed}" -lt "${timeout_secs}" ]]; do
    if docker exec "${BITCOIND_CONTAINER}" bitcoin-cli -regtest -rpcuser="${BITCOIND_USER}" -rpcpassword="${BITCOIND_PASSWORD}" getblockcount >/dev/null 2>&1; then
      echo "${BITCOIND_CONTAINER} RPC is ready."
      return 0
    fi
    sleep 1
    elapsed=$((elapsed + 1))
  done

  echo "Error: ${BITCOIND_CONTAINER} RPC was not ready after ${timeout_secs}s."
  return 1
}

wait_for_bitcoind_wallet() {
  local wallet_name="$1"
  local timeout_secs="${2:-60}"
  local elapsed=0

  echo "Waiting for wallet '${wallet_name}' to load..."
  while [[ "${elapsed}" -lt "${timeout_secs}" ]]; do
    if docker exec "${BITCOIND_CONTAINER}" bitcoin-cli -regtest -rpcuser="${BITCOIND_USER}" -rpcpassword="${BITCOIND_PASSWORD}" -rpcwallet="${wallet_name}" getwalletinfo >/dev/null 2>&1; then
      echo "Wallet '${wallet_name}' is ready."
      return 0
    fi
    sleep 1
    elapsed=$((elapsed + 1))
  done

  echo "Error: wallet '${wallet_name}' was not ready after ${timeout_secs}s."
  return 1
}

# Resolve CONTRACTS_IMAGE_TAG: --contracts-tag > Cargo.toml (no env var override)
PROJECT_ROOT="$(cd -- "${SCRIPT_DIR}/../.." && pwd)"
CARGO_TOML="${PROJECT_ROOT}/Cargo.toml"

if [[ -n "$CONTRACTS_TAG_ARG" ]]; then
  CONTRACTS_IMAGE_TAG="$CONTRACTS_TAG_ARG"
else
  if [[ ! -f "$CARGO_TOML" ]]; then
    echo "Error: Cargo.toml not found at $CARGO_TOML" >&2
    exit 1
  fi
  # Extract union-contracts tag (must be on a single line in Cargo.toml)
  CONTRACTS_IMAGE_TAG=$(sed -n 's/.*union-contracts.*tag[[:space:]]*=[[:space:]]*"\([^"]*\)".*/\1/p' "$CARGO_TOML" | head -1)
  if [[ -z "$CONTRACTS_IMAGE_TAG" ]]; then
    echo "Error: Could not extract union-contracts tag from $CARGO_TOML." >&2
    echo "       Expected format: union-contracts = { ..., tag = \"<version>\", ... } on a single line." >&2
    exit 1
  fi
  # Map git tag to image tag when they differ (e.g. v0.2.0-alpha -> v0.2.0-alpha.1)
  case "$CONTRACTS_IMAGE_TAG" in
    v0.2.0-alpha) CONTRACTS_IMAGE_TAG="v0.2.0-alpha.1" ;;
  esac
fi
export CONTRACTS_IMAGE_BASE
export CONTRACTS_IMAGE_TAG

# Disallow user-provided --build (script injects it when using ${CONTRACTS_TAG_LOCAL_BUILD})
for arg in "${DOCKER_COMPOSE_ARGS[@]}"; do
  if [[ "$arg" == "build" || "$arg" == "--build" || "$arg" == "-b" ]]; then
    echo "Error: --build flag is not supported. Use --contracts-tag ${CONTRACTS_TAG_LOCAL_BUILD} to build from source."
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

# When ${CONTRACTS_TAG_LOCAL_BUILD} + up: inject --build after 'up' (compose requires it as up's option)
if [[ "${IS_UP_COMMAND}" == true && "${CONTRACTS_IMAGE_TAG}" == "${CONTRACTS_TAG_LOCAL_BUILD}" ]]; then
  NEW_ARGS=()
  for arg in "${DOCKER_COMPOSE_ARGS[@]}"; do
    NEW_ARGS+=("$arg")
    [[ "$arg" == "up" ]] && NEW_ARGS+=("--build")
  done
  DOCKER_COMPOSE_ARGS=("${NEW_ARGS[@]}")
fi

# When using a registry tag (not ${CONTRACTS_TAG_LOCAL_BUILD}): always pull, compare digest, set FRESH if changed
if [[ "${IS_UP_COMMAND}" == true && "${CONTRACTS_IMAGE_TAG}" != "${CONTRACTS_TAG_LOCAL_BUILD}" ]]; then
  CONTRACTS_IMAGE="${CONTRACTS_IMAGE_BASE}:${CONTRACTS_IMAGE_TAG}"
  DIGEST_BEFORE=""
  if docker image inspect "$CONTRACTS_IMAGE" >/dev/null 2>&1; then
    DIGEST_BEFORE=$(docker image inspect --format '{{index .RepoDigests 0}}' "$CONTRACTS_IMAGE" 2>/dev/null || true)
  fi
  echo "Pulling contracts image '$CONTRACTS_IMAGE'..."
  if ! docker pull --platform linux/amd64 "$CONTRACTS_IMAGE" ; then
    echo "Error: Failed to pull contracts image '$CONTRACTS_IMAGE'."
    echo "  The image may not exist in the registry for this tag."
    echo "  To build from source instead, use --contracts-tag ${CONTRACTS_TAG_LOCAL_BUILD}"
    exit 1
  fi
  DIGEST_AFTER=$(docker image inspect --format '{{index .RepoDigests 0}}' "$CONTRACTS_IMAGE" 2>/dev/null || true)
  if [[ -n "$DIGEST_BEFORE" && -n "$DIGEST_AFTER" && "$DIGEST_BEFORE" != "$DIGEST_AFTER" ]]; then
    echo "Contracts image digest changed; forcing fresh deploy (down --volumes before up)"
    FRESH=true
  elif [[ -z "$DIGEST_BEFORE" && -n "$DIGEST_AFTER" ]]; then
    echo "Local image had no registry digest (likely built locally); forcing fresh deploy after pull"
    FRESH=true
  fi
fi

# When switching contracts tag: force fresh deploy so new contracts deploy to clean chain
if [[ "${IS_UP_COMMAND}" == true ]]; then
  CURRENT_IMAGE=$(docker inspect deploy-contracts --format '{{.Config.Image}}' 2>/dev/null || true)
  EXPECTED_IMAGE="${CONTRACTS_IMAGE_BASE}:${CONTRACTS_IMAGE_TAG}"
  if [[ -n "$CURRENT_IMAGE" && "$CURRENT_IMAGE" != "$EXPECTED_IMAGE" ]]; then
    echo "Contracts tag changed ($CURRENT_IMAGE -> $EXPECTED_IMAGE); forcing fresh deploy"
    FRESH=true
  fi
fi

echo "IS_UP_COMMAND: ${IS_UP_COMMAND} | FRESH: ${FRESH} | CONTRACTS_IMAGE_TAG: ${CONTRACTS_IMAGE_TAG}"

# If requested (or digest changed), clean local blockchains
if [[ "${FRESH}" == true ]]; then
  echo "Cleaning local blockchains stack (down -v)..."
  cmd="docker compose -p blockchains --env-file \"$ENV_PATH\" -f \"$COMPOSE_FILE\" down --volumes || true"
  echo "Running: $cmd"
  eval "$cmd"
fi

BITCOIND_CONTAINER="bitcoind"
RUNNING_COUNT=$(docker compose -p blockchains --env-file "$ENV_PATH" -f "$COMPOSE_FILE" --profile local ps --status running -q ${BITCOIND_CONTAINER} anvil | wc -l | tr -d ' ')

echo "Detected $RUNNING_COUNT running containers in the local blockchains stack."

if [[ "${IS_UP_COMMAND}" == true && "${RUNNING_COUNT}" -ge 2 ]]; then
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

if [[ "${IS_UP_COMMAND}" == true ]]; then
  wait_for_bitcoind_rpc
fi

# If using 'up' command after a fresh teardown, create the Bitcoin wallet and deploy contracts
if [[ "${IS_UP_COMMAND}" == true && "${FRESH}" == true ]]; then
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

if [[ "${IS_UP_COMMAND}" == true ]]; then
  wait_for_bitcoind_rpc
  wait_for_bitcoind_wallet "mainwallet"
fi

echo
echo "Done!!!"
