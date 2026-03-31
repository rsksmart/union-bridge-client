#!/usr/bin/env bash

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

cd "${SCRIPT_DIR}" || {
  echo "Error: Failed to change to script directory: ${SCRIPT_DIR}"
  exit 1
}

DOCKER_COMPOSE_ARGS=()
NUM_OPERATORS=""
AUTO_CONFIRM=false
FRESH=false
BASE_STORAGE_PATH="${BASE_STORAGE_PATH:-$HOME}"
DEFAULT_ENV_FILE="${SCRIPT_DIR}/docker-deploy.env"
ENV_FILE="${DEFAULT_ENV_FILE}"

print_help() {
  echo "Usage: $0 [OPTIONS] [DOCKER_COMPOSE_ARGS...]"
  echo ""
  echo "Before startup, prepare the operator env files on this host:"
  echo "  <project_root>/cli-setup-operators.sh --ops 4"
  echo ""
  echo "Options:"
  echo "  --ops <N>                Number of operators to start (1-10, default: 4)"
  echo "  --env-file <PATH>        Compose env file to use instead of docker/operator/docker-deploy.env"
  echo "  --tag <TAG>              Override UC_TAG for this docker compose invocation only"
  echo "  --help                   Display this help message"
  echo "  --fresh                  Tear down operators (and volumes) before running the command"
  echo "  --yes, -y                Automatic yes to fresh confirmation prompt"
  echo ""
  echo "Examples:"
  echo "  $0 up -d"
  echo "  $0 --ops 6 up -d"
  echo "  $0 --fresh up -d"
  echo "  $0 --env-file /path/to/docker-deploy.env up -d"
  echo "  $0 logs -f"
  echo "  $0 down"
  echo ""
  echo "The compose override is derived from NUM_OPERATORS:"
  echo "  1 -> docker-compose.one.yml"
  echo "  2-10 -> docker-compose.all.yml"
  echo ""
  echo "Any additional arguments will be passed directly to docker compose."
  exit 0
}

read_env_value() {
  local env_file="$1"
  local key="$2"

  awk -F= -v key="${key}" '$1 == key {print substr($0, index($0, "=") + 1); exit}' "${env_file}"
}

require_key_store_password() {
  local configured_password="${KEY_STORE_PASSWORD:-}"
  local env_file

  for env_file in "$@"; do
    if [[ -n "${configured_password}" ]]; then
      break
    fi
    if [[ -f "${env_file}" ]]; then
      configured_password="$(read_env_value "${env_file}" "KEY_STORE_PASSWORD")"
    fi
  done

  if [[ -z "${configured_password}" ]]; then
    echo "Error: KEY_STORE_PASSWORD is required for operator startup." >&2
    echo "Export KEY_STORE_PASSWORD or define it in the operator docker-service.env before running startup commands." >&2
    exit 1
  fi
}

while [[ $# -gt 0 ]]; do
  case $1 in
    --help)
      print_help
      ;;
    --ops)
      NUM_OPERATORS="$2"
      if ! [[ "$NUM_OPERATORS" =~ ^(10|[1-9])$ ]]; then
        echo "Error: --ops must be between 1 and 10"
        exit 1
      fi
      shift 2
      ;;
    --env-file)
      ENV_FILE="$2"
      shift 2
      ;;
    --tag)
      UC_TAG="$2"
      shift 2
      ;;
    --fresh)
      FRESH=true
      shift
      ;;
    --yes|-y)
      AUTO_CONFIRM=true
      shift
      ;;
    *)
      DOCKER_COMPOSE_ARGS+=("$1")
      shift
      ;;
  esac
done

if [[ ! -f "${ENV_FILE}" ]]; then
  echo "Error: missing env file ${ENV_FILE}" >&2
  exit 1
fi

for arg in "${DOCKER_COMPOSE_ARGS[@]}"; do
  if [[ "$arg" == "build" || "$arg" == "--build" || "$arg" == "-b" ]]; then
    echo "Error: Building from source is not supported with this script."
    echo "Only registry images should be used. Building images is not allowed."
    exit 1
  fi
done

IS_STARTUP_COMMAND=false
for arg in "${DOCKER_COMPOSE_ARGS[@]}"; do
  if [[ "$arg" == "up" || "$arg" == "restart" || "$arg" == "start" || "$arg" == "create" ]]; then
    IS_STARTUP_COMMAND=true
    break
  fi
done

operator_compose_env_file_path() {
  local op_num="$1"
  echo "${BASE_STORAGE_PATH}/.union_bridge/op_${op_num}/docker-compose.env"
}

operator_runtime_env_file_path() {
  local op_num="$1"
  echo "${BASE_STORAGE_PATH}/.union_bridge/op_${op_num}/docker-service.env"
}

require_operator_env_file() {
  local file_path="$1"

  if [[ ! -f "${file_path}" ]]; then
    echo "Error: missing prepared operator env file ${file_path}" >&2
    echo "Prepare the operator artifacts under \${BASE_STORAGE_PATH}/.union_bridge/op_N/ before starting containers." >&2
    return 1
  fi
}

resolved_num_operators() {
  local configured_count
  local default_count="4"

  configured_count="${NUM_OPERATORS:-$(read_env_value "${ENV_FILE}" "NUM_OPERATORS")}"
  configured_count="${configured_count:-${default_count}}"

  if ! [[ "${configured_count}" =~ ^(10|[1-9])$ ]]; then
    echo "Error: resolved NUM_OPERATORS must be between 1 and 10." >&2
    exit 1
  fi

  echo "${configured_count}"
}

compose_override_file() {
  if [[ "$(resolved_num_operators)" == "1" ]]; then
    echo "docker-compose.one.yml"
  else
    echo "docker-compose.all.yml"
  fi
}

uses_shared_bitvmx_network() {
  if [[ "$(compose_override_file)" == "docker-compose.all.yml" ]]; then
    return 0
  fi

  return 1
}

run_compose_stack() {
  local project_name="$1"
  local compose_env_file_path="$2"
  local runtime_env_file_path="$3"
  local compose_override
  compose_override="$(compose_override_file)"
  local -a compose_cmd=(
    docker compose
    -p "${project_name}"
    -f docker-compose.yml
    -f "${compose_override}"
    --env-file "${ENV_FILE}"
    --env-file "${compose_env_file_path}"
    --env-file "${runtime_env_file_path}"
  )

  compose_cmd+=("${DOCKER_COMPOSE_ARGS[@]}")

  if [[ "${IS_STARTUP_COMMAND}" == true ]]; then
    require_key_store_password "${ENV_FILE}" "${runtime_env_file_path}"
  fi

  echo
  echo "Running operator ${project_name} with env files ${compose_env_file_path} and ${runtime_env_file_path}"
  if [[ -n "${UC_TAG:-}" ]]; then
    printf "'UC_TAG=%q " "${UC_TAG}"
  else
    printf "'"
  fi
  printf "%q " "${compose_cmd[@]}"
  echo "'"

  if [[ -n "${UC_TAG:-}" ]]; then
    UC_TAG="${UC_TAG}" "${compose_cmd[@]}"
  else
    "${compose_cmd[@]}"
  fi
}

if [[ "${FRESH}" == true ]]; then
  echo "WARNING: --fresh will tear down operators and DELETE ALL VOLUMES (including data)."
  if [[ "${AUTO_CONFIRM}" != true ]]; then
    read -r -p "Are you sure you want to continue? (yes/no): " confirmation
    if [[ "$confirmation" != "yes" ]]; then
      echo "Aborted."
      exit 1
    fi
  fi

  echo "Cleaning operator stacks (down --volumes)..."
  for op_num in $(seq 1 "$(resolved_num_operators)"); do
    operator_compose_env_file="$(operator_compose_env_file_path "${op_num}")"
    operator_runtime_env_file="$(operator_runtime_env_file_path "${op_num}")"
    if ! require_operator_env_file "${operator_compose_env_file}" \
      || ! require_operator_env_file "${operator_runtime_env_file}"; then
      exit 1
    fi
    docker compose -p "op_${op_num}" -f docker-compose.yml -f "$(compose_override_file)" --env-file "${ENV_FILE}" --env-file "${operator_compose_env_file}" --env-file "${operator_runtime_env_file}" down --volumes
  done
fi

if [[ "${IS_STARTUP_COMMAND}" == true ]] && uses_shared_bitvmx_network; then
  NETWORK_NAME="bitvmx-shared-network"
  if ! docker network inspect "${NETWORK_NAME}" >/dev/null 2>&1; then
    echo "Creating docker network '${NETWORK_NAME}'..."
    docker network create --driver bridge --subnet=172.20.0.0/16 "${NETWORK_NAME}"
  fi
fi

for op_num in $(seq 1 "$(resolved_num_operators)"); do
  operator_compose_env_file="$(operator_compose_env_file_path "${op_num}")"
  operator_runtime_env_file="$(operator_runtime_env_file_path "${op_num}")"
  if ! require_operator_env_file "${operator_compose_env_file}" \
    || ! require_operator_env_file "${operator_runtime_env_file}"; then
    exit 1
  fi
  run_compose_stack "op_${op_num}" "${operator_compose_env_file}" "${operator_runtime_env_file}"
done
