#!/usr/bin/env bash

# Manages 4 BitVMX clients (no Union Client, no blockchains).
# Prerequisites: Start blockchains first with: ./start_blockchains.sh up -d

set -euo pipefail

SCRIPT_DIR=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" &>/dev/null && pwd)
COMPOSE_FILE="${SCRIPT_DIR}/docker-compose.bitvmx.yml"
ENV_FILE="${SCRIPT_DIR}/.env.local"
NETWORK_NAME="bitvmx-shared-network"
BASE_STORAGE_PATH="${BASE_STORAGE_PATH:-$HOME}"

bootstrap_broker_keystore() {
  local keystore_dir="${BASE_STORAGE_PATH}/.union_bridge/keystore"
  local broker_key_path="${keystore_dir}/broker.key"
  local broker_hash_path="${keystore_dir}/broker.pubkey_hash"

  mkdir -p "${keystore_dir}"

  if [[ ! -f "${broker_key_path}" ]]; then
    echo "Broker key not found at ${broker_key_path}. Generating..."
    openssl genpkey -algorithm RSA -out "${broker_key_path}" -pkeyopt rsa_keygen_bits:2048 2>/dev/null
    chmod 600 "${broker_key_path}" || true
  fi

  openssl pkey -in "${broker_key_path}" -pubout -outform DER 2>/dev/null \
    | openssl dgst -sha256 -binary \
    | od -A n -v -t x1 | tr -d ' \n' > "${broker_hash_path}"
}

get_bootstrapped_broker_pubkey_hash() {
  local broker_hash_path="${BASE_STORAGE_PATH}/.union_bridge/keystore/broker.pubkey_hash"
  if [[ ! -f "${broker_hash_path}" ]]; then
    echo "Error: broker pubkey hash not found at ${broker_hash_path}" >&2
    exit 1
  fi
  tr -d ' \n' < "${broker_hash_path}"
}

patch_local_operator_pubkey_hash() {
  local hash="$1"
  local config_dir="${SCRIPT_DIR}/../bitvmx-client/config/local/client/config"

  for file in "${config_dir}"/op_*.yaml; do
    [[ -f "${file}" ]] || continue
    awk -v h="${hash}" '/pubkey_hash:/ && ++n<=1 {sub(/pubkey_hash: .*/, "pubkey_hash: " h);} 1' \
      "${file}" > "${file}.tmp" && mv "${file}.tmp" "${file}"
  done
}

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

# Check if we're using a startup command
IS_STARTUP_COMMAND=false
for arg in "${DOCKER_COMPOSE_ARGS[@]}"; do
  if [[ "$arg" == "up" || "$arg" == "start" || "$arg" == "restart" || "$arg" == "create" ]]; then
    IS_STARTUP_COMMAND=true
    break
  fi
done

# If --fresh, clean first
if [[ "${FRESH}" == true ]]; then
  echo "Cleaning BitVMX stack (down --volumes)..."
  docker compose -p bitvmx --env-file "$ENV_FILE" -f "$COMPOSE_FILE" down --volumes 2>/dev/null || true
fi

# Ensure network exists for startup commands
if [[ "${IS_STARTUP_COMMAND}" == true ]]; then
  ensure_network
  bootstrap_broker_keystore
  BROKER_PUBKEY_HASH="$(get_bootstrapped_broker_pubkey_hash)"
  patch_local_operator_pubkey_hash "${BROKER_PUBKEY_HASH}"
fi

# Run docker compose with provided args
docker compose -p bitvmx --env-file "$ENV_FILE" -f "$COMPOSE_FILE" "${DOCKER_COMPOSE_ARGS[@]}"

# Print connection info after startup
if [[ "${IS_STARTUP_COMMAND}" == true ]]; then
  echo
  echo "BitVMX clients ready. Connect to:"
  echo "  op_1 -> localhost:22222"
  echo "  op_2 -> localhost:33333"
  echo "  op_3 -> localhost:44444"
  echo "  op_4 -> localhost:55554"
fi
