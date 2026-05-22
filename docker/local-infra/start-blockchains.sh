#!/usr/bin/env bash

# This script manages the local blockchain stack.
# local-anvil: bitcoind + predeployed Anvil.
# local-rskj: bitcoind + RSKj + powpeg-node + contracts deploy.

DOCKER_COMPOSE_ARGS=()

 # TODO(iago) probably we should split this in 2 scripts

# Resolve script directory (for referencing compose files reliably)
SCRIPT_DIR=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" &>/dev/null && pwd)
ENVIRONMENT="local-anvil"
COMPOSE_FILE="${SCRIPT_DIR}/docker-compose.blockchains.anvil.yaml"
ENV_PATH="${SCRIPT_DIR}/.env.anvil"

CONTRACTS_TAG_LOCAL_BUILD="local-build"

# Display help message
print_help() {
  echo "Usage: $0 [OPTIONS] [DOCKER_COMPOSE_ARGS...]"
  echo ""
  echo "Options:"
  echo "  --help                     Display this help message"
  echo "  --env ENV                  Environment: local-anvil or local-rskj (default: local-anvil)"
  echo "  --fresh                    Tear down local blockchains (and volumes). Can be used standalone or with 'up'"
  echo "  --contracts-tag TAG         Override contracts image tag (e.g. v0.2.0-alpha.1 or ${CONTRACTS_TAG_LOCAL_BUILD})"
  echo "  --pull-contracts            Pull predeployed Anvil image from registry even if it exists locally"
  echo "  --rskj-tag TAG              Official rsksmart/rskj tag for local-rskj (default: .env.rskj)"
  echo "  --powpeg-tag TAG            Official rsksmart/powpeg-node tag for local-rskj (default: .env.rskj)"
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
  echo "  $0 --env local-rskj --fresh up -d # Start bitcoind + rskj + powpeg-node + contracts"
  echo "  $0 --env local-rskj --rskj-tag VETIVER-9.0.1 --powpeg-tag VETIVER-9.0.0.0 --fresh up -d"
  echo "  $0 --fresh up -d                    # Clean and start local blockchains"
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
RSKJ_TAG_ARG=""
POWPEG_TAG_ARG=""

# Parse args
while [[ $# -gt 0 ]]; do
  case $1 in
    --help)
      print_help
      ;;
    --env)
      if [[ $# -lt 2 || -z "${2:-}" ]]; then
        echo "Error: --env requires a non-empty value (local-anvil or local-rskj)"
        exit 1
      fi
      ENVIRONMENT="$2"
      shift 2
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
    --rskj-tag)
      if [[ $# -lt 2 || -z "${2:-}" ]]; then
        echo "Error: --rskj-tag requires a non-empty Docker tag"
        exit 1
      fi
      RSKJ_TAG_ARG="$2"
      shift 2
      ;;
    --powpeg-tag)
      if [[ $# -lt 2 || -z "${2:-}" ]]; then
        echo "Error: --powpeg-tag requires a non-empty Docker tag"
        exit 1
      fi
      POWPEG_TAG_ARG="$2"
      shift 2
      ;;
    *)
      DOCKER_COMPOSE_ARGS+=("$1")
      shift
      ;;
  esac
done

case "$ENVIRONMENT" in
  local-anvil)
    COMPOSE_FILE="${SCRIPT_DIR}/docker-compose.blockchains.anvil.yaml"
    ENV_PATH="${SCRIPT_DIR}/.env.anvil"
    ;;
  local-rskj)
    COMPOSE_FILE="${SCRIPT_DIR}/docker-compose.blockchains.rskj.yaml"
    ENV_PATH="${SCRIPT_DIR}/.env.rskj"
    ;;
  *)
    echo "Error: --env must be 'local-anvil' or 'local-rskj'" >&2
    exit 1
    ;;
esac

# Check env file exists
if [[ ! -f "$ENV_PATH" ]]; then
  echo "Error: env file not found at $ENV_PATH"
  exit 1
fi

source "${ENV_PATH}"

if [[ "$ENVIRONMENT" != "local-rskj" && ( -n "$RSKJ_TAG_ARG" || -n "$POWPEG_TAG_ARG" ) ]]; then
  echo "Error: --rskj-tag/--powpeg-tag are only supported with --env local-rskj." >&2
  exit 1
fi

validate_image_tag() {
  local name="$1"
  local tag="$2"

  if [[ -z "$tag" || ! "$tag" =~ ^[A-Za-z0-9._-]+$ ]]; then
    echo "Error: ${name} must be a Docker tag using only letters, numbers, '.', '_' or '-'." >&2
    exit 1
  fi
}

if [[ "$ENVIRONMENT" == "local-rskj" ]]; then
  RSKJ_TAG="${RSKJ_TAG_ARG:-${RSKJ_TAG:-VETIVER-9.0.1}}"
  POWPEG_TAG="${POWPEG_TAG_ARG:-${POWPEG_TAG:-VETIVER-9.0.0.0}}"
  validate_image_tag RSKJ_TAG "$RSKJ_TAG"
  validate_image_tag POWPEG_TAG "$POWPEG_TAG"
  export RSKJ_TAG POWPEG_TAG

  CONTRACTS_CONTEXT_CANDIDATE=""
  if CONTRACTS_CONTEXT_CANDIDATE=$(cd "$SCRIPT_DIR" && cd "$CONTRACTS_CONTEXT_PATH" 2>/dev/null && pwd); then
    :
  elif [[ -d "$(cd "$SCRIPT_DIR/../.." && pwd)/../union-bridge-contracts" ]]; then
    CONTRACTS_CONTEXT_CANDIDATE="$(cd "$SCRIPT_DIR/../.." && pwd)/../union-bridge-contracts"
  elif [[ -d "$HOME/Projects/rootstock/union/union-bridge-contracts" ]]; then
    CONTRACTS_CONTEXT_CANDIDATE="$HOME/Projects/rootstock/union/union-bridge-contracts"
  else
    echo "Error: could not resolve CONTRACTS_CONTEXT_PATH '${CONTRACTS_CONTEXT_PATH}'." >&2
    echo "Set CONTRACTS_CONTEXT_PATH to a union-bridge-contracts checkout and rerun." >&2
    exit 1
  fi
  export CONTRACTS_CONTEXT_PATH="$CONTRACTS_CONTEXT_CANDIDATE"
  export CONTRACTS_DOCKERFILE="$SCRIPT_DIR/Dockerfile_deploy_rskj"
fi

BITCOIND_CONTAINER="bitcoind"
ANVIL_CONTAINER="anvil"

# Check if we're using the 'up' command
IS_UP_COMMAND=false
for arg in "${DOCKER_COMPOSE_ARGS[@]}"; do
  if [[ "$arg" == "up" ]]; then
    IS_UP_COMMAND=true
    break
  fi
done

for arg in "${DOCKER_COMPOSE_ARGS[@]}"; do
  if [[ "$arg" == "build" || "$arg" == "--build" || "$arg" == "-b" ]]; then
    echo "Error: --build flag is not supported. Use --contracts-tag ${CONTRACTS_TAG_LOCAL_BUILD} to build from source."
    exit 1
  fi
done

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

wait_for_rskj_rpc() {
  local timeout_secs="${1:-120}"
  local elapsed=0

  echo "Waiting for RSKj RPC at ${RSKJ_HOST_HTTP_URL}..."
  while [[ "${elapsed}" -lt "${timeout_secs}" ]]; do
    if cast rpc eth_chainId --rpc-url "${RSKJ_HOST_HTTP_URL}" >/dev/null 2>&1; then
      echo "RSKj RPC is ready."
      return 0
    fi
    sleep 1
    elapsed=$((elapsed + 1))
  done

  echo "Error: RSKj RPC was not ready after ${timeout_secs}s."
  return 1
}

wait_for_service_healthy() {
  local service="$1"
  local timeout_secs="${2:-180}"
  local elapsed=0
  local container_id=""
  local status=""

  echo "Waiting for service '${service}' to become healthy..."
  while [[ "${elapsed}" -lt "${timeout_secs}" ]]; do
    container_id=$(docker compose -p blockchains --env-file "$ENV_PATH" -f "$COMPOSE_FILE" ps -q "$service" 2>/dev/null || true)
    status=""
    if [[ -n "$container_id" ]]; then
      status=$(docker inspect --format '{{if .State.Health}}{{.State.Health.Status}}{{else}}{{.State.Status}}{{end}}' "$container_id" 2>/dev/null || true)
    fi
    if [[ "$status" == "healthy" ]]; then
      echo "Service '${service}' is healthy."
      return 0
    fi
    sleep 2
    elapsed=$((elapsed + 2))
  done

  echo "Error: service '${service}' was not healthy after ${timeout_secs}s." >&2
  docker compose -p blockchains --env-file "$ENV_PATH" -f "$COMPOSE_FILE" ps "$service" >&2 || true
  docker compose -p blockchains --env-file "$ENV_PATH" -f "$COMPOSE_FILE" logs --tail=80 "$service" >&2 || true
  return 1
}

wait_for_one_shot_service() {
  local service="$1"
  local timeout_secs="${2:-600}"
  local elapsed=0
  local container_id=""
  local status=""
  local exit_code=""

  echo "Waiting for one-shot service '${service}' to complete..."
  while [[ "${elapsed}" -lt "${timeout_secs}" ]]; do
    container_id=$(docker compose -p blockchains --env-file "$ENV_PATH" -f "$COMPOSE_FILE" ps -aq "$service" 2>/dev/null || true)
    if [[ -n "$container_id" ]]; then
      status=$(docker inspect --format '{{.State.Status}}' "$container_id" 2>/dev/null || true)
      exit_code=$(docker inspect --format '{{.State.ExitCode}}' "$container_id" 2>/dev/null || true)
      if [[ "$status" == "exited" && "$exit_code" == "0" ]]; then
        echo "Service '${service}' completed successfully."
        return 0
      fi
      if [[ "$status" == "exited" && "$exit_code" != "0" ]]; then
        echo "Error: service '${service}' exited with code ${exit_code}." >&2
        docker compose -p blockchains --env-file "$ENV_PATH" -f "$COMPOSE_FILE" logs --tail=160 "$service" >&2 || true
        return 1
      fi
    fi
    sleep 2
    elapsed=$((elapsed + 2))
  done

  echo "Error: service '${service}' did not complete after ${timeout_secs}s." >&2
  docker compose -p blockchains --env-file "$ENV_PATH" -f "$COMPOSE_FILE" logs --tail=160 "$service" >&2 || true
  return 1
}

create_bitcoin_wallet_if_needed() {
  local wallet_name="$1"

  if docker exec "${BITCOIND_CONTAINER}" bitcoin-cli -regtest -rpcuser="${BITCOIND_USER}" -rpcpassword="${BITCOIND_PASSWORD}" -rpcwallet="${wallet_name}" getwalletinfo >/dev/null 2>&1; then
    echo "Wallet '${wallet_name}' already loaded."
    return 0
  fi

  echo "Creating wallet '${wallet_name}' in ${BITCOIND_CONTAINER}..."
  if docker exec "${BITCOIND_CONTAINER}" bitcoin-cli -regtest -rpcuser="${BITCOIND_USER}" -rpcpassword="${BITCOIND_PASSWORD}" createwallet "${wallet_name}" false false "" false true >/dev/null 2>&1; then
    wait_for_bitcoind_wallet "${wallet_name}"
    return 0
  fi

  echo "Wallet create failed; trying to load existing wallet '${wallet_name}'..."
  docker exec "${BITCOIND_CONTAINER}" bitcoin-cli -regtest -rpcuser="${BITCOIND_USER}" -rpcpassword="${BITCOIND_PASSWORD}" loadwallet "${wallet_name}" >/dev/null 2>&1 || true
  wait_for_bitcoind_wallet "${wallet_name}"
}

mine_bitcoin_blocks() {
  local blocks="$1"
  local address=""

  if [[ "$blocks" -le 0 ]]; then
    return 0
  fi

  address=$(docker exec "${BITCOIND_CONTAINER}" bitcoin-cli -regtest -rpcuser="${BITCOIND_USER}" -rpcpassword="${BITCOIND_PASSWORD}" -rpcwallet=mainwallet getnewaddress bootstrap bech32)
  echo "Mining ${blocks} Bitcoin regtest block(s) to ${address}..."
  docker exec "${BITCOIND_CONTAINER}" bitcoin-cli -regtest -rpcuser="${BITCOIND_USER}" -rpcpassword="${BITCOIND_PASSWORD}" -rpcwallet=mainwallet generatetoaddress "${blocks}" "${address}" >/dev/null
}

extract_deployed_address() {
  local contract_name="$1"

  docker compose -p blockchains --env-file "$ENV_PATH" -f "$COMPOSE_FILE" logs --no-color deploy-contracts 2>/dev/null \
    | awk -v contract_name="${contract_name}" '
      index($0, contract_name) && index($0, "address") {
        for (i = NF; i >= 1; i--) {
          if ($i ~ /^0x[0-9a-fA-F]{40}$/) {
            address = $i
            break
          }
        }
      }
      END {
        if (address != "") {
          print address
        }
      }
    '
}

normalize_hex_lower() {
  printf '%s' "$1" | tr '[:upper:]' '[:lower:]'
}

require_env_var() {
  local name="$1"

  if [[ -z "${!name:-}" ]]; then
    echo "Error: missing required env var ${name}" >&2
    return 1
  fi
}

local_regtest_deployer_address() {
  local deployer_private_key=""
  local deployer_address=""

  require_env_var MNEMONIC || return 1
  require_env_var DEPLOYER_INDEX || return 1

  deployer_private_key=$(cast wallet private-key --mnemonic "${MNEMONIC}" --mnemonic-index "${DEPLOYER_INDEX}")
  if [[ -z "${deployer_private_key}" ]]; then
    echo "Error: could not derive local-rskj deployer private key." >&2
    return 1
  fi

  deployer_address=$(cast wallet address --private-key "${deployer_private_key}")
  if [[ -z "${deployer_address}" ]]; then
    echo "Error: could not derive local-rskj deployer address." >&2
    return 1
  fi

  printf '%s\n' "${deployer_address}"
}

fund_rsk_address_if_empty() {
  local address="$1"
  local value="$2"
  local label="$3"
  local balance=""

  require_env_var RSKJ_HOST_HTTP_URL || return 1
  require_env_var RSKJ_COW_PRIVATE_KEY || return 1

  balance=$(cast balance "${address}" --rpc-url "${RSKJ_HOST_HTTP_URL}" 2>/dev/null || true)
  if [[ -z "${balance}" ]]; then
    balance="0"
  fi

  if [[ "${balance}" != "0" ]]; then
    echo "${label} already funded (balance wei: ${balance})."
    return 0
  fi

  echo "Funding ${label} ${address} with ${value}..."
  cast send "${address}" \
    --value "${value}" \
    --private-key "${RSKJ_COW_PRIVATE_KEY}" \
    --rpc-url "${RSKJ_HOST_HTTP_URL}" \
    --legacy >/dev/null

  sleep 1
  balance=$(cast balance "${address}" --rpc-url "${RSKJ_HOST_HTTP_URL}" 2>/dev/null || true)
  if [[ -z "${balance}" || "${balance}" == "0" ]]; then
    echo "Error: failed to fund ${label} ${address}." >&2
    return 1
  fi

  echo "${label} funded successfully (balance wei: ${balance})."
}

fund_local_regtest_deployer() {
  local deployer_address=""

  require_env_var DEPLOYER_PREFUND_VALUE || return 1

  if ! deployer_address=$(local_regtest_deployer_address); then
    return 1
  fi
  fund_rsk_address_if_empty "${deployer_address}" "${DEPLOYER_PREFUND_VALUE}" "Contract deployer"
}

authorize_native_bridge() {
  local bridge_precompile="0x0000000000000000000000000000000001000006"
  local rbtc_bridge=""
  local registered=""
  local authorizer_private_key=""
  local authorizer_address=""
  local registered_lower=""
  local rbtc_bridge_lower=""

  rbtc_bridge=$(extract_deployed_address "RbtcBridge.sol")
  if [[ -z "$rbtc_bridge" ]]; then
    echo "Error: failed to resolve deployed RbtcBridge address from deploy-contracts logs." >&2
    return 1
  fi

  rbtc_bridge_lower=$(normalize_hex_lower "$rbtc_bridge")

  authorizer_private_key=$(cast keccak "changeUnionBridgeContractAddressAuthorizer")
  authorizer_address=$(cast wallet address --private-key "$authorizer_private_key")
  fund_rsk_address_if_empty "${authorizer_address}" 100000000000000000 "Native Bridge authorizer" || return 1

  registered=$(cast call --rpc-url "${RSKJ_HOST_HTTP_URL}" "$bridge_precompile" "getUnionBridgeContractAddress()(address)" 2>/dev/null || true)
  registered_lower=$(normalize_hex_lower "$registered")
  if [[ "$registered_lower" == "$rbtc_bridge_lower" ]]; then
    echo "Native Bridge already authorized for RbtcBridge ${rbtc_bridge}."
    return 0
  fi

  echo "Authorizing Native Bridge union bridge contract: ${rbtc_bridge}"
  cast send --rpc-url "${RSKJ_HOST_HTTP_URL}" \
    --legacy \
    --private-key "$authorizer_private_key" \
    "$bridge_precompile" \
    "setUnionBridgeContractAddressForTestnet(address)" \
    "$rbtc_bridge" >/dev/null

  registered=$(cast call --rpc-url "${RSKJ_HOST_HTTP_URL}" "$bridge_precompile" "getUnionBridgeContractAddress()(address)")
  registered_lower=$(normalize_hex_lower "$registered")
  if [[ "$registered_lower" != "$rbtc_bridge_lower" ]]; then
    echo "Error: Native Bridge authorization mismatch. expected=${rbtc_bridge} actual=${registered}" >&2
    return 1
  fi

  echo "Native Bridge authorized for RbtcBridge ${registered}."
}

run_local_regtest() {
  if [[ -n "$CONTRACTS_TAG_ARG" || "$PULL_CONTRACTS" == true ]]; then
    echo "Error: --contracts-tag/--pull-contracts are only supported for --env local Anvil." >&2
    exit 1
  fi

  if [[ "${FRESH}" == true ]]; then
    echo "Cleaning local-rskj blockchains stack (down -v)..."
    docker compose -p blockchains --env-file "$ENV_PATH" -f "$COMPOSE_FILE" down --volumes --timeout 1 || true
  fi

  echo "Using local-rskj images:"
  echo "  RSKj:        rsksmart/rskj:${RSKJ_TAG}"
  echo "  powpeg-node: rsksmart/powpeg-node:${POWPEG_TAG}"

  if [[ "${IS_UP_COMMAND}" != true ]]; then
    docker compose -p blockchains --env-file "$ENV_PATH" -f "$COMPOSE_FILE" "${DOCKER_COMPOSE_ARGS[@]}"
    return $?
  fi

  chmod 400 "${SCRIPT_DIR}/powpeg/config/federator1.key" || true

  echo "Starting local-rskj base services (bitcoind + rskj)..."
  docker compose -p blockchains --env-file "$ENV_PATH" -f "$COMPOSE_FILE" up -d bitcoind rskj
  wait_for_bitcoind_rpc 120
  create_bitcoin_wallet_if_needed mainwallet
  wait_for_rskj_rpc 180

  if [[ "${FRESH}" == true ]]; then
    mine_bitcoin_blocks "${LOCAL_RSKJ_BOOTSTRAP_BTC_BLOCKS:-25}"
  fi

  echo "Starting powpeg-node..."
  docker compose -p blockchains --env-file "$ENV_PATH" -f "$COMPOSE_FILE" up -d powpeg-node
  wait_for_service_healthy powpeg-node 300

  fund_local_regtest_deployer || return 1

  echo "Deploying contracts..."
  docker compose -p blockchains --env-file "$ENV_PATH" -f "$COMPOSE_FILE" up -d deploy-contracts
  wait_for_one_shot_service deploy-contracts 900
  authorize_native_bridge

  echo
  echo "Done!!!"
}

if [[ "$ENVIRONMENT" == "local-rskj" ]]; then
  run_local_regtest
  exit $?
fi

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
    v0.4.1-alpha) CONTRACTS_IMAGE_TAG="v0.4.1-alpha-10-4-2" ;;
  esac
fi
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
if [[ "${IS_UP_COMMAND}" == true ]]; then
  CURRENT_IMAGE=$(docker inspect anvil --format '{{.Config.Image}}' 2>/dev/null || true)
  EXPECTED_IMAGE="${PREDEPLOYED_ANVIL_IMAGE_BASE}:${CONTRACTS_IMAGE_TAG}"
  if [[ -n "$CURRENT_IMAGE" && "$CURRENT_IMAGE" != "$EXPECTED_IMAGE" ]]; then
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
