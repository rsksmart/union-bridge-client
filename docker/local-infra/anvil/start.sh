#!/usr/bin/env bash
#
# Anvil-specific portion of the `local-anvil` stack bring-up.
#
# Invoked by start-blockchains.sh AFTER bitcoind is up and `mainwallet` exists.
# Reads COMPOSE_FILE, ENV_PATH, ENVIRONMENT, FRESH from env (exported by the
# parent). Anvil-specific responsibilities:
#   - Resolve CONTRACTS_CONTEXT_PATH for local-build docker compose context
#   - Parse --contracts-tag / --pull-contracts
#   - Resolve the predeployed Anvil image tag (from arg or Cargo.toml)
#   - For registry tags: pull / refresh the image; detect digest changes that
#     warrant an implicit fresh teardown
#   - When local-build: inject docker compose --build
#   - Bring up the anvil service + wait for its RPC
#
# If implicit fresh is triggered (image tag/digest changed), this script does
# `down --volumes` and brings bitcoind + anvil back up together via a single
# compose command, then recreates the bitcoind wallet inline. No shared
# helpers are sourced.

set -euo pipefail

SCRIPT_DIR=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" &>/dev/null && pwd)

# Defaults so the script is runnable standalone for debugging.
COMPOSE_FILE="${COMPOSE_FILE:-${SCRIPT_DIR}/docker-compose.yaml}"
ENV_PATH="${ENV_PATH:-${SCRIPT_DIR}/.env}"
FRESH="${FRESH:-false}"

CONTRACTS_TAG_LOCAL_BUILD="local-build"
BITCOIND_CONTAINER="bitcoind"
ANVIL_CONTAINER="anvil"

CONTRACTS_TAG_ARG=""
PULL_CONTRACTS=false
DOCKER_COMPOSE_ARGS=()

while [[ $# -gt 0 ]]; do
  case $1 in
    --fresh)
      # Already handled by the parent; absorb so we don't push it into compose args.
      shift
      ;;
    --contracts-tag)
      if [[ $# -lt 2 || -z "${2:-}" ]]; then
        echo "Error: --contracts-tag requires a non-empty value (e.g. v0.2.0-alpha.1 or ${CONTRACTS_TAG_LOCAL_BUILD})" >&2
        exit 1
      fi
      CONTRACTS_TAG_ARG="$2"
      shift 2
      ;;
    --pull-contracts)
      PULL_CONTRACTS=true
      shift
      ;;
    --rskj-tag|--powpeg-tag)
      echo "Error: $1 is only supported by rskj/start.sh" >&2
      exit 1
      ;;
    *)
      DOCKER_COMPOSE_ARGS+=("$1")
      shift
      ;;
  esac
done

# If invoked standalone, source the env file so BITCOIND_* are in scope.
if [[ -z "${BITCOIND_USER:-}" ]]; then
  if [[ ! -f "$ENV_PATH" ]]; then
    echo "Error: env file not found at $ENV_PATH" >&2
    exit 1
  fi
  # shellcheck disable=SC1090
  source "$ENV_PATH"
fi

# Resolve the contracts source directory for local-build docker compose context.
CONTRACTS_CONTEXT_CANDIDATE=""
if [[ -n "${CONTRACTS_CONTEXT_PATH:-}" ]] && CONTRACTS_CONTEXT_CANDIDATE=$(cd "$SCRIPT_DIR" && cd "$CONTRACTS_CONTEXT_PATH" 2>/dev/null && pwd); then
  :
elif [[ -d "$(cd "$SCRIPT_DIR/../../.." && pwd)/../union-bridge-contracts" ]]; then
  CONTRACTS_CONTEXT_CANDIDATE="$(cd "$SCRIPT_DIR/../../.." && pwd)/../union-bridge-contracts"
elif [[ -d "$HOME/Projects/rootstock/union/union-bridge-contracts" ]]; then
  CONTRACTS_CONTEXT_CANDIDATE="$HOME/Projects/rootstock/union/union-bridge-contracts"
else
  echo "Error: could not resolve CONTRACTS_CONTEXT_PATH '${CONTRACTS_CONTEXT_PATH:-}'." >&2
  echo "Set CONTRACTS_CONTEXT_PATH to a union-bridge-contracts checkout and rerun." >&2
  exit 1
fi
export CONTRACTS_CONTEXT_PATH="$CONTRACTS_CONTEXT_CANDIDATE"

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

  echo "Error: ${ANVIL_CONTAINER} RPC was not ready after ${timeout_secs}s." >&2
  return 1
}

# Resolve CONTRACTS_IMAGE_TAG: --contracts-tag > Cargo.toml (no env var override)
PROJECT_ROOT="$(cd -- "${SCRIPT_DIR}/../../.." && pwd)"
CARGO_TOML="${PROJECT_ROOT}/Cargo.toml"

if [[ -n "$CONTRACTS_TAG_ARG" ]]; then
  CONTRACTS_IMAGE_TAG="$CONTRACTS_TAG_ARG"
else
  if [[ ! -f "$CARGO_TOML" ]]; then
    echo "Error: Cargo.toml not found at $CARGO_TOML" >&2
    exit 1
  fi
  CONTRACTS_IMAGE_TAG=$(sed -n 's/.*union-contracts.*tag[[:space:]]*=[[:space:]]*"\([^"]*\)".*/\1/p' "$CARGO_TOML" | head -1)
  if [[ -z "$CONTRACTS_IMAGE_TAG" ]]; then
    echo "Error: Could not extract union-contracts tag from $CARGO_TOML." >&2
    echo "       Expected format: union-contracts = { ..., tag = \"<version>\", ... } on a single line." >&2
    exit 1
  fi
  # Map git tag to image tag when they differ.
  case "$CONTRACTS_IMAGE_TAG" in
    v0.2.0-alpha) CONTRACTS_IMAGE_TAG="v0.2.0-alpha.1" ;;
    v0.4.1-alpha) CONTRACTS_IMAGE_TAG="v0.4.1-alpha-10-4-2-2m" ;;
  esac
fi
export PREDEPLOYED_ANVIL_IMAGE_BASE
export CONTRACTS_IMAGE_TAG

# Detect whether the user invoked `up` (in practice this is always true when
# called from the orchestrator; kept for direct invocation parity).
IS_UP_COMMAND=false
for arg in "${DOCKER_COMPOSE_ARGS[@]}"; do
  if [[ "$arg" == "up" ]]; then
    IS_UP_COMMAND=true
    break
  fi
done

# For local-build + up, inject --build after 'up' (compose requires it as an up option).
if [[ "${IS_UP_COMMAND}" == true && "${CONTRACTS_IMAGE_TAG}" == "${CONTRACTS_TAG_LOCAL_BUILD}" ]]; then
  NEW_ARGS=()
  for arg in "${DOCKER_COMPOSE_ARGS[@]}"; do
    NEW_ARGS+=("$arg")
    [[ "$arg" == "up" ]] && NEW_ARGS+=("--build")
  done
  DOCKER_COMPOSE_ARGS=("${NEW_ARGS[@]}")
fi

# Implicit-fresh detection: registry-tag pull may surface a new digest, or
# the running anvil container may be on a different image than the one we want.
IMPLICIT_FRESH=false
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
      echo "Error: Failed to pull predeployed Anvil image '$PREDEPLOYED_ANVIL_IMAGE'." >&2
      echo "  The image may not exist in the registry for this tag." >&2
      echo "  Build it locally with this tag, pass --contracts-tag ${CONTRACTS_TAG_LOCAL_BUILD}, or publish it to the registry." >&2
      exit 1
    fi
    DIGEST_AFTER=$(docker image inspect --format '{{index .RepoDigests 0}}' "$PREDEPLOYED_ANVIL_IMAGE" 2>/dev/null || true)
    if [[ -n "$DIGEST_BEFORE" && -n "$DIGEST_AFTER" && "$DIGEST_BEFORE" != "$DIGEST_AFTER" ]]; then
      echo "Predeployed Anvil image digest changed; forcing fresh start (down --volumes before up)"
      IMPLICIT_FRESH=true
    elif [[ -z "$DIGEST_BEFORE" && -n "$DIGEST_AFTER" ]]; then
      echo "Local image had no registry digest (likely built locally); forcing fresh start after pull"
      IMPLICIT_FRESH=true
    fi
  fi
fi

# Force fresh when switching contracts tag, so Anvil loads the matching chain state.
if [[ "${IS_UP_COMMAND}" == true ]]; then
  CURRENT_IMAGE=$(docker inspect "${ANVIL_CONTAINER}" --format '{{.Config.Image}}' 2>/dev/null || true)
  EXPECTED_IMAGE="${PREDEPLOYED_ANVIL_IMAGE_BASE}:${CONTRACTS_IMAGE_TAG}"
  if [[ -n "$CURRENT_IMAGE" && "$CURRENT_IMAGE" != "$EXPECTED_IMAGE" ]]; then
    echo "Contracts tag changed ($CURRENT_IMAGE -> $EXPECTED_IMAGE); forcing fresh start"
    IMPLICIT_FRESH=true
  fi
fi

echo "IS_UP_COMMAND: ${IS_UP_COMMAND} | FRESH: ${FRESH} | IMPLICIT_FRESH: ${IMPLICIT_FRESH} | CONTRACTS_IMAGE_TAG: ${CONTRACTS_IMAGE_TAG} | PULL_CONTRACTS: ${PULL_CONTRACTS}"

# If implicit fresh: tear down (this kills bitcoind too) and bring bitcoind +
# anvil back together. Recreate the wallet inline — no shared helpers.
if [[ "${IS_UP_COMMAND}" == true && "${IMPLICIT_FRESH}" == true ]]; then
  echo "Tearing down local-anvil stack to apply forced fresh start..."
  docker compose -p blockchains --env-file "$ENV_PATH" -f "$COMPOSE_FILE" down --volumes --timeout 1 || true

  echo "Bringing ${BITCOIND_CONTAINER} + ${ANVIL_CONTAINER} back up together..."
  docker compose -p blockchains --env-file "$ENV_PATH" -f "$COMPOSE_FILE" --profile local up -d "${BITCOIND_CONTAINER}" "${ANVIL_CONTAINER}"

  # Inline bitcoind wallet bootstrap. Idempotent.
  for _ in {1..30}; do
    docker exec "${BITCOIND_CONTAINER}" bitcoin-cli -regtest -rpcuser="${BITCOIND_USER}" -rpcpassword="${BITCOIND_PASSWORD}" getblockcount >/dev/null 2>&1 && break
    sleep 1
  done
  docker exec "${BITCOIND_CONTAINER}" bitcoin-cli -regtest -rpcuser="${BITCOIND_USER}" -rpcpassword="${BITCOIND_PASSWORD}" createwallet mainwallet false false "" false true >/dev/null 2>&1 || true

  wait_for_anvil_rpc
  exit 0
fi

# Normal path: bitcoind already up + wallet created by parent. Just bring up
# anvil (compose is idempotent for bitcoind if it's in the args too).
if ! docker compose -p blockchains --env-file "$ENV_PATH" -f "$COMPOSE_FILE" --profile local "${DOCKER_COMPOSE_ARGS[@]}"; then
  echo "Error: docker compose command failed" >&2
  exit 1
fi

if [[ "${IS_UP_COMMAND}" == true ]]; then
  wait_for_anvil_rpc
fi
