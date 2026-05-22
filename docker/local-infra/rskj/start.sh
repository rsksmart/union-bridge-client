#!/usr/bin/env bash
#
# RSKj-specific portion of the `local-rskj` stack bring-up.
#
# Invoked by start-blockchains.sh AFTER bitcoind is up and `mainwallet` exists.
# Reads COMPOSE_FILE, ENV_PATH, ENVIRONMENT, FRESH from env (exported by the
# parent). RSKj-specific responsibilities:
#   - Parse --rskj-tag / --powpeg-tag
#   - Resolve CONTRACTS_CONTEXT_PATH for the deploy-contracts build
#   - Bring up rskj, wait for its RPC
#   - If FRESH, mine bootstrap BTC blocks (so the federation has mature funds)
#   - Bring up powpeg-node, wait healthy
#   - Fund the contract deployer from the COW account
#   - Bring up the one-shot deploy-contracts service, wait for clean exit
#   - Authorize the Native Bridge precompile (RSKIP502) for the deployed RbtcBridge

set -euo pipefail

SCRIPT_DIR=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" &>/dev/null && pwd)

# Defaults so the script is runnable standalone for debugging.
COMPOSE_FILE="${COMPOSE_FILE:-${SCRIPT_DIR}/docker-compose.yaml}"
ENV_PATH="${ENV_PATH:-${SCRIPT_DIR}/.env}"
FRESH="${FRESH:-false}"

RSKJ_TAG_ARG=""
POWPEG_TAG_ARG=""
DOCKER_COMPOSE_ARGS=()

while [[ $# -gt 0 ]]; do
  case $1 in
    --fresh)
      shift
      ;;
    --rskj-tag)
      if [[ $# -lt 2 || -z "${2:-}" ]]; then
        echo "Error: --rskj-tag requires a non-empty Docker tag" >&2
        exit 1
      fi
      RSKJ_TAG_ARG="$2"
      shift 2
      ;;
    --powpeg-tag)
      if [[ $# -lt 2 || -z "${2:-}" ]]; then
        echo "Error: --powpeg-tag requires a non-empty Docker tag" >&2
        exit 1
      fi
      POWPEG_TAG_ARG="$2"
      shift 2
      ;;
    --contracts-tag|--pull-contracts)
      echo "Error: $1 is only supported by anvil/start.sh" >&2
      exit 1
      ;;
    *)
      DOCKER_COMPOSE_ARGS+=("$1")
      shift
      ;;
  esac
done

# If invoked standalone, source the env file so BITCOIND_* / RSKJ_* are in scope.
if [[ -z "${BITCOIND_USER:-}" ]]; then
  if [[ ! -f "$ENV_PATH" ]]; then
    echo "Error: env file not found at $ENV_PATH" >&2
    exit 1
  fi
  # shellcheck disable=SC1090
  source "$ENV_PATH"
fi

validate_image_tag() {
  local name="$1"
  local tag="$2"

  if [[ -z "$tag" || ! "$tag" =~ ^[A-Za-z0-9._-]+$ ]]; then
    echo "Error: ${name} must be a Docker tag using only letters, numbers, '.', '_' or '-'." >&2
    exit 1
  fi
}

RSKJ_TAG="${RSKJ_TAG_ARG:-${RSKJ_TAG:-VETIVER-9.0.1}}"
POWPEG_TAG="${POWPEG_TAG_ARG:-${POWPEG_TAG:-VETIVER-9.0.0.0}}"
validate_image_tag RSKJ_TAG "$RSKJ_TAG"
validate_image_tag POWPEG_TAG "$POWPEG_TAG"
export RSKJ_TAG POWPEG_TAG

# Resolve the contracts source directory for the deploy-contracts build context.
CONTRACTS_CONTEXT_CANDIDATE=""
if CONTRACTS_CONTEXT_CANDIDATE=$(cd "$SCRIPT_DIR" && cd "$CONTRACTS_CONTEXT_PATH" 2>/dev/null && pwd); then
  :
elif [[ -d "$(cd "$SCRIPT_DIR/../../.." && pwd)/../union-bridge-contracts" ]]; then
  CONTRACTS_CONTEXT_CANDIDATE="$(cd "$SCRIPT_DIR/../../.." && pwd)/../union-bridge-contracts"
elif [[ -d "$HOME/Projects/rootstock/union/union-bridge-contracts" ]]; then
  CONTRACTS_CONTEXT_CANDIDATE="$HOME/Projects/rootstock/union/union-bridge-contracts"
else
  echo "Error: could not resolve CONTRACTS_CONTEXT_PATH '${CONTRACTS_CONTEXT_PATH}'." >&2
  echo "Set CONTRACTS_CONTEXT_PATH to a union-bridge-contracts checkout and rerun." >&2
  exit 1
fi
export CONTRACTS_CONTEXT_PATH="$CONTRACTS_CONTEXT_CANDIDATE"
export CONTRACTS_DOCKERFILE="$SCRIPT_DIR/Dockerfile_deploy"

echo "Using local-rskj images:"
echo "  RSKj:        rsksmart/rskj:${RSKJ_TAG}"
echo "  powpeg-node: rsksmart/powpeg-node:${POWPEG_TAG}"

# This script always runs in up-mode. Non-up commands are handled by the parent.

wait_for_rskj_rpc() {
  local timeout_secs="${1:-180}"
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

  echo "Error: RSKj RPC was not ready after ${timeout_secs}s." >&2
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

mine_bitcoin_blocks() {
  local blocks="$1"
  local address=""

  if [[ "$blocks" -le 0 ]]; then
    return 0
  fi

  address=$(docker exec bitcoind bitcoin-cli -regtest -rpcuser="${BITCOIND_USER}" -rpcpassword="${BITCOIND_PASSWORD}" -rpcwallet=mainwallet getnewaddress bootstrap bech32)
  echo "Mining ${blocks} Bitcoin regtest block(s) to ${address}..."
  docker exec bitcoind bitcoin-cli -regtest -rpcuser="${BITCOIND_USER}" -rpcpassword="${BITCOIND_PASSWORD}" -rpcwallet=mainwallet generatetoaddress "${blocks}" "${address}" >/dev/null
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

local_rskj_deployer_address() {
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

fund_local_rskj_deployer() {
  local deployer_address=""

  require_env_var DEPLOYER_PREFUND_VALUE || return 1

  if ! deployer_address=$(local_rskj_deployer_address); then
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

# powpeg's mounted federator key must be 0400 — set this unconditionally; harmless
# if already correct.
chmod 400 "${SCRIPT_DIR}/config/federator1.key" || true

echo "Starting rskj..."
docker compose -p blockchains --env-file "$ENV_PATH" -f "$COMPOSE_FILE" up -d rskj
wait_for_rskj_rpc 180

if [[ "${FRESH}" == "true" ]]; then
  mine_bitcoin_blocks "${LOCAL_RSKJ_BOOTSTRAP_BTC_BLOCKS:-25}"
fi

echo "Starting powpeg-node..."
docker compose -p blockchains --env-file "$ENV_PATH" -f "$COMPOSE_FILE" up -d powpeg-node
wait_for_service_healthy powpeg-node 300

fund_local_rskj_deployer || exit 1

echo "Deploying contracts..."
docker compose -p blockchains --env-file "$ENV_PATH" -f "$COMPOSE_FILE" up -d deploy-contracts
wait_for_one_shot_service deploy-contracts 900
authorize_native_bridge
