#!/usr/bin/env bash

# wrapper script for operations to perform operator and user operations
# usage: ./cli-operations.sh setup create-rootstock-wallets
#        ./cli-operations.sh operator fund --env local-docker
#        ./cli-operations.sh operator apply-stream -s 1 --env alphanet -o 1 -r prover
#        ./cli-operations.sh user fund --env local
#        ./cli-operations.sh user pegin -a 0x1234...cdef -v 100000 -p 7
#        ./cli-operations.sh user pegout -v 100000
#        ./cli-operations.sh --help

set -euo pipefail

# change to script directory to ensure relative paths work
cd "$(dirname "$0")"

OPERATIONS_BIN="./target/release/operations"

# Determine environment from UC_ENV or parse from args to load appropriate .env file
ENV_FROM_ARGS=""
for arg in "$@"; do
  if [[ "$arg" == "--env" ]] || [[ "$arg" == "-e" ]]; then
    ENV_FROM_ARGS="next"
  elif [[ "$ENV_FROM_ARGS" == "next" ]]; then
    ENV_FROM_ARGS="$arg"
    break
  fi
done

# Load UC_ENV, UC_TAG, UC_OPERATOR_ID, UC_OPERATOR_ROLE from root .envrc
# This ensures these variables are available even when scripts are run from subdirectories
PROJECT_ROOT="$(pwd)"
ENVRC_FILE="${PROJECT_ROOT}/.envrc"
if [[ -f "$ENVRC_FILE" ]]; then
  # Source .envrc to get UC_ENV, UC_TAG, UC_OPERATOR_ID, UC_OPERATOR_ROLE (only export statements, safe to source)
  set -a
  source "$ENVRC_FILE" 2>/dev/null || true
  set +a
fi

# Save UC_OPERATOR_ID and UC_OPERATOR_ROLE from .envrc before .env file might overwrite them
# Precedence: command-line flags (-o, -r) > UC_OPERATOR_ID/UC_OPERATOR_ROLE from .envrc > .env file values
SAVED_UC_OPERATOR_ID_FROM_ENVRC=""
SAVED_UC_OPERATOR_ROLE_FROM_ENVRC=""
if [[ -n "${UC_OPERATOR_ID:-}" ]]; then
  SAVED_UC_OPERATOR_ID_FROM_ENVRC="${UC_OPERATOR_ID}"
fi
if [[ -n "${UC_OPERATOR_ROLE:-}" ]]; then
  SAVED_UC_OPERATOR_ROLE_FROM_ENVRC="${UC_OPERATOR_ROLE}"
fi

# Use environment from args if provided, otherwise from UC_ENV
ENV_TO_LOAD="${ENV_FROM_ARGS:-${UC_ENV:-}}"

# Map environment names to .env file paths (for other vars like BITCOIND_URL, ROOTSTOCK_URL, etc.)
if [[ -n "$ENV_TO_LOAD" ]]; then
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
  # UC_TAG, UC_OPERATOR_ID, UC_OPERATOR_ROLE are loaded from .envrc above
  if [[ -n "$ENV_FILE" && -f "$ENV_FILE" ]]; then
    set -a
    source "$ENV_FILE"
    set +a
    
    # Restore UC_OPERATOR_ID and UC_OPERATOR_ROLE from .envrc if they were set
    # (command-line flags will still override these via clap's env variable handling)
    if [[ -n "${SAVED_UC_OPERATOR_ID_FROM_ENVRC}" ]]; then
      export UC_OPERATOR_ID="${SAVED_UC_OPERATOR_ID_FROM_ENVRC}"
    fi
    if [[ -n "${SAVED_UC_OPERATOR_ROLE_FROM_ENVRC}" ]]; then
      export UC_OPERATOR_ROLE="${SAVED_UC_OPERATOR_ROLE_FROM_ENVRC}"
    fi
  fi
fi

# In GitHub Actions (e.g. e2e framework): use existing binary if present (cache hit). Locally: always build so we never run stale code.
if ! { [ -x "$OPERATIONS_BIN" ] && [ "${GITHUB_ACTIONS:-}" = "true" ]; }; then
  cargo build --release --manifest-path cli/operations/Cargo.toml --quiet
fi

# forward all arguments to operations (using release binary directly)
RUST_BACKTRACE=0 exec "$OPERATIONS_BIN" "$@"

