#!/usr/bin/env bash
#
# Orchestrates the local blockchain stack.
#
# Sequence:
#   1. Parse --env / --fresh; resolve compose file + env file
#   2. Source the env file
#   3. For non-up commands (down, ps, logs, ...) — forward directly to docker compose and exit
#   4. For up:
#        a. Optional fresh teardown if --fresh
#        b. Bring up bitcoind, wait for RPC, create mainwallet
#        c. Delegate to start-blockchains-{anvil,rskj}.sh for the Rootstock-node bring-up
#        d. Print final status
#
# The Rootstock-node script is invoked as a subprocess (not exec'd) so this
# script resumes control afterwards to finalize.

set -euo pipefail

SCRIPT_DIR=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" &>/dev/null && pwd)

BITCOIND_CONTAINER="bitcoind"

ENVIRONMENT="local-anvil"
FRESH=false
REMAINING_ARGS=()

# Parse only the cross-rootstock args (--env, --fresh); everything else flows
# through to the Rootstock-node script or directly to docker compose.
while [[ $# -gt 0 ]]; do
  case "$1" in
    --env)
      if [[ $# -lt 2 || -z "${2:-}" ]]; then
        echo "Error: --env requires a non-empty value (local-anvil or local-rskj)" >&2
        exit 1
      fi
      ENVIRONMENT="$2"
      shift 2
      ;;
    --fresh)
      FRESH=true
      REMAINING_ARGS+=("$1")
      shift
      ;;
    *)
      REMAINING_ARGS+=("$1")
      shift
      ;;
  esac
done

case "$ENVIRONMENT" in
  local-anvil)
    COMPOSE_FILE="${SCRIPT_DIR}/anvil/docker-compose.yaml"
    ENV_PATH="${SCRIPT_DIR}/anvil/.env"
    ROOTSTOCK_SCRIPT="${SCRIPT_DIR}/anvil/start.sh"
    # Used to detect / tear down the other chain when switching.
    OTHER_CHAIN_CONTAINER="rskj"
    OTHER_CHAIN_COMPOSE="${SCRIPT_DIR}/rskj/docker-compose.yaml"
    OTHER_CHAIN_ENV="${SCRIPT_DIR}/rskj/.env"
    ;;
  local-rskj)
    COMPOSE_FILE="${SCRIPT_DIR}/rskj/docker-compose.yaml"
    ENV_PATH="${SCRIPT_DIR}/rskj/.env"
    ROOTSTOCK_SCRIPT="${SCRIPT_DIR}/rskj/start.sh"
    OTHER_CHAIN_CONTAINER="anvil"
    OTHER_CHAIN_COMPOSE="${SCRIPT_DIR}/anvil/docker-compose.yaml"
    OTHER_CHAIN_ENV="${SCRIPT_DIR}/anvil/.env"
    ;;
  *)
    echo "Error: --env must be 'local-anvil' or 'local-rskj' (got: '${ENVIRONMENT}')" >&2
    exit 1
    ;;
esac

if [[ ! -f "$ENV_PATH" ]]; then
  echo "Error: env file not found at $ENV_PATH" >&2
  exit 1
fi
# shellcheck disable=SC1090
source "$ENV_PATH"

# Forbid docker compose `build` invocations from the user — anvil injects --build
# only when using the local-build contracts tag; rskj never needs it.
for arg in "${REMAINING_ARGS[@]}"; do
  if [[ "$arg" == "build" || "$arg" == "--build" || "$arg" == "-b" ]]; then
    echo "Error: --build flag is not supported. Use --contracts-tag local-build for anvil contract-source builds." >&2
    exit 1
  fi
done

# Is the user invoking `up`? Non-up commands skip orchestration entirely and
# forward directly to docker compose.
IS_UP_COMMAND=false
for arg in "${REMAINING_ARGS[@]}"; do
  if [[ "$arg" == "up" ]]; then
    IS_UP_COMMAND=true
    break
  fi
done

if [[ "$IS_UP_COMMAND" != true ]]; then
  COMPOSE_ARGS=()
  for arg in "${REMAINING_ARGS[@]}"; do
    [[ "$arg" == "--fresh" ]] && continue
    COMPOSE_ARGS+=("$arg")
  done
  exec docker compose -p blockchains --env-file "$ENV_PATH" -f "$COMPOSE_FILE" "${COMPOSE_ARGS[@]}"
fi

# ─── Up flow ────────────────────────────────────────────────────────────────

wait_for_bitcoind_rpc() {
  local timeout_secs="${1:-120}"
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

  echo "Error: ${BITCOIND_CONTAINER} RPC was not ready after ${timeout_secs}s." >&2
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

  echo "Error: wallet '${wallet_name}' was not ready after ${timeout_secs}s." >&2
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

# If the *other* chain is currently running, tear it down first. Both chains
# share ports (bitcoin: 18443, rootstock RPC: 8545), so they can't coexist.
# We preserve the other chain's volumes — only its containers are removed.
if docker ps --format '{{.Names}}' | grep -qx "${OTHER_CHAIN_CONTAINER}"; then
  echo "Detected '${OTHER_CHAIN_CONTAINER}' from the other chain; tearing it down so '${ENVIRONMENT}' can take over..."
  docker compose -p blockchains --env-file "$OTHER_CHAIN_ENV" -f "$OTHER_CHAIN_COMPOSE" down --remove-orphans --timeout 5 || true
fi

# Fresh teardown of the requested chain.
if [[ "$FRESH" == true ]]; then
  echo "Cleaning ${ENVIRONMENT} blockchains stack (down --volumes)..."
  docker compose -p blockchains --env-file "$ENV_PATH" -f "$COMPOSE_FILE" down --volumes --remove-orphans --timeout 1 || true
fi

# Bring up bitcoind first; the Rootstock-node script assumes it's ready.
echo "Starting ${BITCOIND_CONTAINER}..."
docker compose -p blockchains --env-file "$ENV_PATH" -f "$COMPOSE_FILE" up -d "${BITCOIND_CONTAINER}"
wait_for_bitcoind_rpc 120
create_bitcoin_wallet_if_needed mainwallet

# Export what the Rootstock-node script needs to know.
export COMPOSE_FILE ENV_PATH ENVIRONMENT FRESH

# Delegate the Rootstock-node bring-up.
"$ROOTSTOCK_SCRIPT" "${REMAINING_ARGS[@]}"

echo
echo "Done!!!"
