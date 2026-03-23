#!/usr/bin/env bash

# Manages 4 BitVMX clients (no Union Client, no blockchains).
# Prerequisites: Start blockchains first with: ./start_blockchains.sh up -d

set -euo pipefail

SCRIPT_DIR=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" &>/dev/null && pwd)
COMPOSE_FILE="${SCRIPT_DIR}/docker-compose.bitvmx.yml"
ENV_FILE="${SCRIPT_DIR}/.env.local"
NETWORK_NAME="bitvmx-shared-network"
export BASE_STORAGE_PATH="${BASE_STORAGE_PATH:-$HOME}"

print_help() {
  echo "Usage: $0 [OPTIONS] [DOCKER_COMPOSE_ARGS...]"
  echo ""
  echo "Options:"
  echo "  --help    Display this help message"
  echo "  --fresh   Clean (volumes) before running"
  echo ""
  echo "Examples:"
  echo "  $0 up -d           # Start 4 bitvmx clients"
  echo "  $0 --fresh up -d   # Clean and start"
  echo "  $0 down            # Stop"
  echo "  $0 down --volumes  # Stop and remove volumes"
  echo "  $0 logs -f         # Follow logs"
  echo "  $0 ps              # Status"
  echo ""
  echo "Prerequisites: Start blockchains first"
  echo "  ./start_blockchains.sh --fresh up -d"
  echo "  ../operator/setup_operators.sh --env local --ops 4"
  echo ""
  echo "Connect to:"
  echo "  op_1 -> localhost:22222"
  echo "  op_2 -> localhost:33333"
  echo "  op_3 -> localhost:44444"
  echo "  op_4 -> localhost:55554"
  exit 0
}

# Check env file exists
if [[ ! -f "$ENV_FILE" ]]; then
  echo "Error: .env.local not found at $ENV_FILE"
  exit 1
fi

FRESH=false
DOCKER_COMPOSE_ARGS=()

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
    *)
      DOCKER_COMPOSE_ARGS+=("$1")
      shift
      ;;
  esac
done

ensure_network() {
  if ! docker network inspect $NETWORK_NAME >/dev/null 2>&1; then
    echo "Creating docker network '$NETWORK_NAME'..."
    docker network create --driver bridge --subnet=172.20.0.0/16 $NETWORK_NAME
  fi
}

ensure_generated_bitvmx_configs() {
  local op_num config_dir

  for op_num in 1 2 3 4; do
    config_dir="${BASE_STORAGE_PATH}/.union_bridge/op_${op_num}/bitvmx/local/client/config"
    if [[ ! -d "${config_dir}" ]]; then
      echo "Error: missing generated BitVMX config directory ${config_dir}" >&2
      echo "Run ../operator/setup_operators.sh --env local --ops 4 before starting BitVMX." >&2
      exit 1
    fi
  done
}

# Check if we're using the 'up' command
IS_UP_COMMAND=false
for arg in "${DOCKER_COMPOSE_ARGS[@]}"; do
  if [[ "$arg" == "up" ]]; then
    IS_UP_COMMAND=true
    break
  fi
done

# If --fresh, clean first
if [[ "${FRESH}" == true ]]; then
  echo "Cleaning BitVMX stack (down --volumes)..."
  docker compose -p bitvmx --env-file "$ENV_FILE" -f "$COMPOSE_FILE" down --volumes 2>/dev/null || true
fi

# Ensure network exists for up command
if [[ "${IS_UP_COMMAND}" == true ]]; then
  ensure_generated_bitvmx_configs
  ensure_network
fi

# Run docker compose with provided args
docker compose -p bitvmx --env-file "$ENV_FILE" -f "$COMPOSE_FILE" "${DOCKER_COMPOSE_ARGS[@]}"

# Print connection info after up
if [[ "${IS_UP_COMMAND}" == true ]]; then
  echo
  echo "BitVMX clients ready. Connect to:"
  echo "  op_1 -> localhost:22222"
  echo "  op_2 -> localhost:33333"
  echo "  op_3 -> localhost:44444"
  echo "  op_4 -> localhost:55554"
fi
