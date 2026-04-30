#!/usr/bin/env bash

# This script manages the local blockchain stack defined in docker-compose.blockchains.yaml.
# It intentionally focuses ONLY on the blockchains stack (bitcoind and predeployed anvil).

DOCKER_COMPOSE_ARGS=()

# Resolve script directory (for referencing compose files reliably)
SCRIPT_DIR=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" &>/dev/null && pwd)
COMPOSE_FILE="${SCRIPT_DIR}/docker-compose.blockchains.yaml"
ENV_PATH="${SCRIPT_DIR}/.env.local"

CONTRACTS_TAG_LOCAL_BUILD="local-build"

# Display help message
print_help() {
  echo "Usage: $0 [OPTIONS] [DOCKER_COMPOSE_ARGS...]"
  echo ""
  echo "Options:"
  echo "  --help                     Display this help message"
  echo "  --fresh                    Tear down local blockchains (and volumes). Can be used standalone or with 'up'"
  echo "  --contracts-tag TAG         Override contracts image tag (e.g. v0.2.0-alpha.1 or ${CONTRACTS_TAG_LOCAL_BUILD})"
  echo "  --pull-contracts            Pull predeployed Anvil image from registry even if it exists locally"
  echo ""
  echo "Predeployed Anvil image:"
  echo "  Default: derived from Cargo.toml (union-contracts tag) — uses local image if present, otherwise pulls from PREDEPLOYED_ANVIL_IMAGE_BASE"
  echo "  Override: use --contracts-tag flag"
  echo "    ${CONTRACTS_TAG_LOCAL_BUILD}  → build from CONTRACTS_CONTEXT_PATH (e.g. for contract development)"
  echo "    <tag>       → use that image tag locally, pulling only if missing or --pull-contracts is passed"
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
  echo "  $0 --contracts-tag ${CONTRACTS_TAG_LOCAL_BUILD} up -d # Build predeployed Anvil from local contracts"
  echo "  $0 --contracts-tag ${CONTRACTS_TAG_LOCAL_BUILD} up -d # Build predeployed Anvil from local contracts"
  echo "  $0 --contracts-tag v0.2.0-alpha.1 up -d   # Use specific registry tag"
  echo "  $0 --contracts-tag v0.2.0-alpha.1 --pull-contracts up -d # Force pull specific registry tag"
  echo "  $0 down                             # Stop blockchains"
  echo "  $0 ps                               # Check status"
  echo ""
  echo "Any additional arguments will be passed directly to docker compose."
  exit 0
}

FRESH=false
CONTRACTS_TAG_ARG=""
PULL_CONTRACTS=false

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
    --pull-contracts)
      PULL_CONTRACTS=true
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

wait_for_anvil_rpc() {
  local timeout_secs="${1:-60}"
  local elapsed=0

  echo "Waiting for ${ANVIL_CONTAINER} RPC..."
  while [[ "${elapsed}" -lt "${timeout_secs}" ]]; do
    if docker exec "${ANVIL_CONTAINER}" cast rpc eth_chainId --rpc-url http://127.0.0.1:8545 >/dev/null 2>&1; then
      echo "${ANVIL_CONTAINER} RPC is ready."
      return 0
    fi
    sleep 1
    elapsed=$((elapsed + 1))
  done

  echo "Error: ${ANVIL_CONTAINER} RPC was not ready after ${timeout_secs}s."
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
fi
# Map git tag to Docker image tag when they differ (Cargo.toml / --contracts-tag may use git tag only).
# e.g. registry publishes v0.4.1-alpha-10-4-2, not bare v0.4.1-alpha.
case "$CONTRACTS_IMAGE_TAG" in
  v0.2.0-alpha) CONTRACTS_IMAGE_TAG="v0.2.0-alpha.1" ;;
  v0.4.1-alpha) CONTRACTS_IMAGE_TAG="v0.4.1-alpha-10-4-2" ;;
esac
export PREDEPLOYED_ANVIL_IMAGE_BASE
export PREDEPLOYED_ANVIL_IMAGE_BASE
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

# When using a registry tag (not ${CONTRACTS_TAG_LOCAL_BUILD}): use a local image if present.
# Pull only if the image is missing locally or --pull-contracts was requested.
if [[ "${IS_UP_COMMAND}" == true && "${CONTRACTS_IMAGE_TAG}" != "${CONTRACTS_TAG_LOCAL_BUILD}" ]]; then
  PREDEPLOYED_ANVIL_IMAGE="${PREDEPLOYED_ANVIL_IMAGE_BASE}:${CONTRACTS_IMAGE_TAG}"
  PREDEPLOYED_ANVIL_IMAGE="${PREDEPLOYED_ANVIL_IMAGE_BASE}:${CONTRACTS_IMAGE_TAG}"
  DIGEST_BEFORE=""
  IMAGE_EXISTS=false
  if docker image inspect "$PREDEPLOYED_ANVIL_IMAGE" >/dev/null 2>&1; then
    IMAGE_EXISTS=true
    DIGEST_BEFORE=$(docker image inspect --format '{{index .RepoDigests 0}}' "$PREDEPLOYED_ANVIL_IMAGE" 2>/dev/null || true)
  fi

  if [[ "${IMAGE_EXISTS}" == true && "${PULL_CONTRACTS}" != true ]]; then
    echo "Using local predeployed Anvil image '$PREDEPLOYED_ANVIL_IMAGE' (pass --pull-contracts to refresh from registry)"
  else
    if [[ "${IMAGE_EXISTS}" == true ]]; then
      echo "Refreshing predeployed Anvil image '$PREDEPLOYED_ANVIL_IMAGE' from registry..."
    else
      echo "Local predeployed Anvil image '$PREDEPLOYED_ANVIL_IMAGE' not found; pulling from registry..."
    fi
    if ! docker pull --platform linux/amd64 "$PREDEPLOYED_ANVIL_IMAGE" ; then
      echo "Error: Failed to pull predeployed Anvil image '$PREDEPLOYED_ANVIL_IMAGE'."
      echo "  The image may not exist in the registry for this tag."
      echo "  Build it locally with this tag, pass --contracts-tag ${CONTRACTS_TAG_LOCAL_BUILD}, or publish it to the registry."
      exit 1
    fi
    DIGEST_AFTER=$(docker image inspect --format '{{index .RepoDigests 0}}' "$PREDEPLOYED_ANVIL_IMAGE" 2>/dev/null || true)
    if [[ -n "$DIGEST_BEFORE" && -n "$DIGEST_AFTER" && "$DIGEST_BEFORE" != "$DIGEST_AFTER" ]]; then
      echo "Predeployed Anvil image digest changed; forcing fresh start (down --volumes before up)"
      FRESH=true
    elif [[ -z "$DIGEST_BEFORE" && -n "$DIGEST_AFTER" ]]; then
      echo "Local image had no registry digest (likely built locally); forcing fresh start after pull"
      FRESH=true
    fi
  fi
fi

# When switching contracts tag: force fresh start so Anvil loads the matching chain state
# When switching contracts tag: force fresh start so Anvil loads the matching chain state
if [[ "${IS_UP_COMMAND}" == true ]]; then
  CURRENT_IMAGE=$(docker inspect anvil --format '{{.Config.Image}}' 2>/dev/null || true)
  EXPECTED_IMAGE="${PREDEPLOYED_ANVIL_IMAGE_BASE}:${CONTRACTS_IMAGE_TAG}"
  CURRENT_IMAGE=$(docker inspect anvil --format '{{.Config.Image}}' 2>/dev/null || true)
  EXPECTED_IMAGE="${PREDEPLOYED_ANVIL_IMAGE_BASE}:${CONTRACTS_IMAGE_TAG}"
  if [[ -n "$CURRENT_IMAGE" && "$CURRENT_IMAGE" != "$EXPECTED_IMAGE" ]]; then
    echo "Contracts tag changed ($CURRENT_IMAGE -> $EXPECTED_IMAGE); forcing fresh start"
    echo "Contracts tag changed ($CURRENT_IMAGE -> $EXPECTED_IMAGE); forcing fresh start"
    FRESH=true
  fi
fi

echo "IS_UP_COMMAND: ${IS_UP_COMMAND} | FRESH: ${FRESH} | CONTRACTS_IMAGE_TAG: ${CONTRACTS_IMAGE_TAG} | PULL_CONTRACTS: ${PULL_CONTRACTS}"

# If requested (or digest changed), clean local blockchains
if [[ "${FRESH}" == true ]]; then
  echo "Cleaning local blockchains stack (down -v)..."
  cmd="docker compose -p blockchains --env-file \"$ENV_PATH\" -f \"$COMPOSE_FILE\" down --volumes --timeout 1 || true"
  echo "Running: $cmd"
  eval "$cmd"
fi

BITCOIND_CONTAINER="bitcoind"
ANVIL_CONTAINER="anvil"
RUNNING_COUNT=$(docker compose -p blockchains --env-file "$ENV_PATH" -f "$COMPOSE_FILE" --profile local ps --status running -q "${BITCOIND_CONTAINER}" "${ANVIL_CONTAINER}" | wc -l | tr -d ' ')

echo "Detected $RUNNING_COUNT running containers in the local blockchains stack."

if [[ "${IS_UP_COMMAND}" == true && "${RUNNING_COUNT}" -ge 2 ]]; then
  echo "Local blockchains stack already running; skipping 'up'. Run 'down' to start again"
  wait_for_bitcoind_rpc
  wait_for_anvil_rpc
  exit 0
fi

# Finally, run the requested docker compose command
if ! docker compose -p blockchains --env-file "$ENV_PATH" -f "$COMPOSE_FILE" --profile local "${DOCKER_COMPOSE_ARGS[@]}"; then
  echo "Error: docker compose command failed"
  exit 1
fi

if [[ "${IS_UP_COMMAND}" == true ]]; then
  wait_for_bitcoind_rpc
  wait_for_anvil_rpc
fi

# If using 'up' command after a fresh teardown, create the Bitcoin wallet.
# If using 'up' command after a fresh teardown, create the Bitcoin wallet.
if [[ "${IS_UP_COMMAND}" == true && "${FRESH}" == true ]]; then
  # Create wallet
  echo "Creating wallet 'mainwallet' in ${BITCOIND_CONTAINER}..."
  if ! docker exec "${BITCOIND_CONTAINER}" bitcoin-cli -regtest -rpcuser="${BITCOIND_USER}" -rpcpassword="${BITCOIND_PASSWORD}" createwallet mainwallet; then
    echo "Error: Failed to create wallet 'mainwallet'"
    exit 1
  fi
fi

if [[ "${IS_UP_COMMAND}" == true ]]; then
  wait_for_bitcoind_rpc
  wait_for_bitcoind_wallet "mainwallet"
fi

echo
echo "Done!!!"
