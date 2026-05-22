#!/usr/bin/env bash

# Manages 4 BitVMX clients (no Union Client, no blockchains).
# Prerequisites: Start blockchains first with: ./start-blockchains.sh up -d

set -euo pipefail

SCRIPT_DIR=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" &>/dev/null && pwd)
COMPOSE_FILE="${SCRIPT_DIR}/docker-compose.bitvmx.yml"
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
  echo "  ./start-blockchains.sh --fresh up -d"
  echo "  <project_root>/cli-setup-operators.sh --ops 4"
  echo ""
  echo "Connect to:"
  echo "  op_1 -> localhost:22222"
  echo "  op_2 -> localhost:33333"
  echo "  op_3 -> localhost:44444"
  echo "  op_4 -> localhost:55554"
  exit 0
}

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
  local op_num config_dir cfg_file

  for op_num in 1 2 3 4; do
    config_dir="${BASE_STORAGE_PATH}/.union_bridge/op_${op_num}/bitvmx"
    cfg_file="${config_dir}/op_${op_num}.yaml"
    if [[ ! -d "${config_dir}" ]]; then
      echo "Error: missing generated BitVMX config directory ${config_dir}" >&2
      echo "Run <project_root>/cli-setup-operators.sh --ops 4 before starting BitVMX." >&2
      exit 1
    fi
    if [[ ! -f "${cfg_file}" ]]; then
      echo "Error: missing generated BitVMX config ${cfg_file}" >&2
      echo "Run <project_root>/cli-setup-operators.sh --ops 4 before starting BitVMX." >&2
      exit 1
    fi
    if [[ ! -f "${config_dir}/broker_settings.yaml" ]] \
      || ! grep -q '^[[:space:]]*settings:[[:space:]]*config/broker_settings.yaml[[:space:]]*$' "${cfg_file}"; then
      echo "Error: generated BitVMX config ${cfg_file} is stale for the current BitVMX image." >&2
      echo "Run <project_root>/cli-setup-operators.sh --ops 4 to refresh the host-side BitVMX configs." >&2
      exit 1
    fi
  done
}

wait_for_container_health() {
  local container_name="$1"
  local timeout_secs="${2:-90}"
  local elapsed=0
  local status

  echo "Waiting for ${container_name} healthcheck..."
  while [[ "${elapsed}" -lt "${timeout_secs}" ]]; do
    status=$(docker inspect --format '{{if .State.Health}}{{.State.Health.Status}}{{else}}{{.State.Status}}{{end}}' "${container_name}" 2>/dev/null || true)
    case "${status}" in
      healthy)
        echo "${container_name} is healthy."
        return 0
        ;;
      unhealthy|exited|dead)
        echo "Error: ${container_name} entered state '${status}'."
        docker logs --tail 40 "${container_name}" || true
        return 1
        ;;
    esac
    sleep 1
    elapsed=$((elapsed + 1))
  done

  echo "Error: ${container_name} did not become healthy within ${timeout_secs}s."
  docker logs --tail 40 "${container_name}" || true
  return 1
}

wait_for_bitvmx_clients() {
  local timeout_secs=90
  local elapsed=0
  local op_num status
  local pending=(1 2 3 4)
  local next_pending=()

  echo "Waiting for BitVMX client healthchecks..."
  while [[ "${elapsed}" -lt "${timeout_secs}" && "${#pending[@]}" -gt 0 ]]; do
    next_pending=()
    for op_num in "${pending[@]}"; do
      status=$(docker inspect --format '{{if .State.Health}}{{.State.Health.Status}}{{else}}{{.State.Status}}{{end}}' "bitvmx-client-${op_num}" 2>/dev/null || true)
      case "${status}" in
        healthy)
          echo "bitvmx-client-${op_num} is healthy."
          ;;
        unhealthy|exited|dead)
          echo "Error: bitvmx-client-${op_num} entered state '${status}'."
          docker logs --tail 40 "bitvmx-client-${op_num}" || true
          return 1
          ;;
        *)
          next_pending+=("${op_num}")
          ;;
      esac
    done

    pending=("${next_pending[@]}")
    if [[ "${#pending[@]}" -eq 0 ]]; then
      return 0
    fi

    sleep 1
    elapsed=$((elapsed + 1))
  done

  echo "Error: BitVMX clients did not become healthy within ${timeout_secs}s."
  for op_num in "${pending[@]}"; do
    docker logs --tail 40 "bitvmx-client-${op_num}" || true
  done
  return 1
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
  docker compose -p bitvmx -f "$COMPOSE_FILE" down --volumes --timeout 1 2>/dev/null || true
fi

# Ensure network exists for up command
if [[ "${IS_UP_COMMAND}" == true ]]; then
  ensure_generated_bitvmx_configs
  ensure_network
fi

# Run docker compose with provided args
docker compose -p bitvmx -f "$COMPOSE_FILE" "${DOCKER_COMPOSE_ARGS[@]}"

# Print connection info after up
if [[ "${IS_UP_COMMAND}" == true ]]; then
  wait_for_bitvmx_clients
  echo
  echo "BitVMX clients ready. Connect to:"
  echo "  op_1 -> localhost:22222"
  echo "  op_2 -> localhost:33333"
  echo "  op_3 -> localhost:44444"
  echo "  op_4 -> localhost:55554"
fi
