#!/usr/bin/env bash

# wrapper script for operations to perform operator and user operations
# usage: ./cli-operations.sh setup create-rootstock-wallets
#        ./cli-operations.sh operator fund --env local-docker
#        ./cli-operations.sh operator apply-stream -s 1 --env alphanet -o 1 -r prover
#        ./cli-operations.sh user fund --env local
#        ./cli-operations.sh user pegin -a 0x1234...cdef -v 100000
#        ./cli-operations.sh user pegout -v 100000
#        ./cli-operations.sh --help

set -euo pipefail

# change to script directory to ensure relative paths work
cd "$(dirname "$0")"

OPERATIONS_BIN="./target/release/operations"
TOP_LEVEL_COMMAND="${1:-}"

# Determine environment from UC_ENV or parse from args to load appropriate .env file
ENV_FROM_ARGS=""
for arg in "$@"; do
  if [[ "$arg" == "--env" ]] || [[ "$arg" == "-e" ]]; then
    ENV_FROM_ARGS="next"
  elif [[ "$ENV_FROM_ARGS" == "next" ]]; then
    ENV_FROM_ARGS="$arg"
    break
  elif [[ "$arg" == --env=* ]]; then
    ENV_FROM_ARGS="${arg#--env=}"
    break
  elif [[ "$arg" == -e=* ]]; then
    ENV_FROM_ARGS="${arg#-e=}"
    break
  fi
done

# Load environment from root .envrc (only if direnv hasn't already loaded it)
PROJECT_ROOT="$(pwd)"
ENVRC_FILE="${PROJECT_ROOT}/.envrc"
if [[ -z "${DIRENV_DIR:-}" && -f "$ENVRC_FILE" ]]; then
  source "$ENVRC_FILE"
fi

# Setup commands create local artifacts and should not inherit Docker operator
# defaults like KEY_STORE_PASSWORD from docker/operator/.env.* files.
SHOULD_LOAD_DOCKER_ENV=true
if [[ "${TOP_LEVEL_COMMAND}" == "setup" ]]; then
  SHOULD_LOAD_DOCKER_ENV=false
fi

# Use environment from args if provided, otherwise from UC_ENV
ENV_TO_LOAD="${ENV_FROM_ARGS:-${UC_ENV:-}}"

# Map environment names to .env file paths (for other vars like BITCOIND_URL, ROOTSTOCK_URL, etc.)
if [[ "${SHOULD_LOAD_DOCKER_ENV}" == true && -n "$ENV_TO_LOAD" ]]; then
  case "$ENV_TO_LOAD" in
    alphanet)
      ENV_FILE="docker/operator/.env.alphanet"
      ;;
    testnet)
      ENV_FILE="docker/operator/.env.testnet"
      ;;
    local|local-docker)
      ENV_FILE="docker/operator/.env.local"
      ;;
    *)
      # Unknown environment, skip loading .env file
      ENV_FILE=""
      ;;
  esac

  # Source the .env file if it exists (for other vars like BITCOIND_URL, ROOTSTOCK_URL, etc.)
  if [[ -n "$ENV_FILE" && -f "$ENV_FILE" ]]; then
    set -a
    source "$ENV_FILE"
    set +a
  fi
fi

# In GitHub Actions (e.g. e2e framework): use existing binary if present (cache hit). Locally: always build so we never run stale code.
if ! { [ -x "$OPERATIONS_BIN" ] && [ "${GITHUB_ACTIONS:-}" = "true" ]; }; then
  cargo build --release --manifest-path cli/operations/Cargo.toml --quiet
fi

# forward all arguments to operations (using release binary directly)
RUST_BACKTRACE=0 exec "$OPERATIONS_BIN" "$@"
