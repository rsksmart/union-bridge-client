#!/usr/bin/env bash

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

cd "${SCRIPT_DIR}" || {
  echo "Error: Failed to change to script directory: ${SCRIPT_DIR}"
  exit 1
}

DOCKER_COMPOSE_ARGS=()
NUM_OPERATORS=""
OPERATOR_ARG=""
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
  echo "  --op <ID>                Start only the prepared operator under .union_bridge/op_<ID> (1-10)"
  echo "  --ops <N>                Number of operators to start (1-10, default: 4)"
  echo "  --env-file <PATH>        Compose env file to use instead of docker/operator/docker-deploy.env"
  echo "  --tag <TAG>              Override UC_TAG for this docker compose invocation only"
  echo "  --help                   Display this help message"
  echo "  --fresh                  Tear down operators (and volumes) before running the command"
  echo "  --yes, -y                Automatic yes to fresh confirmation prompt"
  echo ""
  echo "Examples:"
  echo "  $0 up -d"
  echo "  $0 --op 3 up -d"
  echo "  $0 --ops 6 up -d"
  echo "  $0 --fresh up -d"
  echo "  $0 --env-file /path/to/docker-deploy.env up -d"
  echo "  $0 logs -f"
  echo "  $0 down"
  echo ""
  echo "The compose override is derived from NUM_OPERATORS:"
  echo "  --op <ID> -> docker-compose.one.yml"
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
    --op)
      OPERATOR_ARG="$2"
      if ! [[ "$OPERATOR_ARG" =~ ^(10|[1-9])$ ]]; then
        echo "Error: --op must be between 1 and 10"
        exit 1
      fi
      shift 2
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

if [[ -n "${OPERATOR_ARG}" && -n "${NUM_OPERATORS}" ]]; then
  echo "Error: --op and --ops cannot be used together." >&2
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

operator_keystore_file_path() {
  local op_num="$1"
  local key_name="$2"
  echo "${BASE_STORAGE_PATH}/.union_bridge/op_${op_num}/union-client/keystore/${key_name}"
}

require_operator_env_file() {
  local file_path="$1"

  if [[ ! -f "${file_path}" ]]; then
    echo "Error: missing prepared operator env file ${file_path}" >&2
    echo "Prepare the operator artifacts under \${BASE_STORAGE_PATH}/.union_bridge/op_N/ before starting containers." >&2
    return 1
  fi
}

require_operator_keystore_file() {
  local file_path="$1"

  if [[ ! -f "${file_path}" ]]; then
    echo "Error: missing operator keystore ${file_path}" >&2
    echo "Run <project_root>/cli-setup-operators.sh --ops <N> to create or reuse the required host-side keys before starting containers." >&2
    return 1
  fi
}

resolved_num_operators() {
  local configured_count
  local default_count="4"

  if [[ -n "${OPERATOR_ARG}" ]]; then
    echo "1"
    return
  fi

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

resolved_operator_ids() {
  if [[ -n "${OPERATOR_ARG}" ]]; then
    echo "${OPERATOR_ARG}"
  else
    seq 1 "$(resolved_num_operators)"
  fi
}

compose_project_name_for_operator() {
  local op_num="$1"
  local configured_project_name=""

  if [[ -n "${OPERATOR_ARG}" ]]; then
    configured_project_name="$(read_env_value "${ENV_FILE}" "COMPOSE_PROJECT_NAME")"
  fi

  echo "${configured_project_name:-op_${op_num}}"
}

uses_shared_bitvmx_network() {
  if [[ "$(compose_override_file)" == "docker-compose.all.yml" ]]; then
    return 0
  fi

  return 1
}

run_compose_stack() {
  local project_name="$1"
  local op_num="$2"
  local compose_env_file_path="$3"
  local runtime_env_file_path="$4"
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
    if ! require_operator_keystore_file "$(operator_keystore_file_path "${op_num}" "user")" \
      || ! require_operator_keystore_file "$(operator_keystore_file_path "${op_num}" "member")"; then
      exit 1
    fi
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

  local compose_exit=0
  if [[ -n "${UC_TAG:-}" ]]; then
    UC_TAG="${UC_TAG}" "${compose_cmd[@]}" || compose_exit=$?
  else
    "${compose_cmd[@]}" || compose_exit=$?
  fi

  # One retry: "address already in use" on published ports is a known flake when bringing
  # up several operator stacks in sequence (e.g. GitHub Actions after a fresh down).
  if [[ "${compose_exit}" -ne 0 && "${IS_STARTUP_COMMAND}" == true ]]; then
    echo "[WARN] docker compose failed for ${project_name} (exit ${compose_exit}); retrying once after teardown and brief wait..." >&2
    if [[ -n "${UC_TAG:-}" ]]; then
      UC_TAG="${UC_TAG}" docker compose -p "${project_name}" -f docker-compose.yml -f "${compose_override}" \
        --env-file "${ENV_FILE}" --env-file "${compose_env_file_path}" --env-file "${runtime_env_file_path}" \
        down --volumes || true
    else
      docker compose -p "${project_name}" -f docker-compose.yml -f "${compose_override}" \
        --env-file "${ENV_FILE}" --env-file "${compose_env_file_path}" --env-file "${runtime_env_file_path}" \
        down --volumes || true
    fi
    sleep 5
    compose_exit=0
    if [[ -n "${UC_TAG:-}" ]]; then
      UC_TAG="${UC_TAG}" "${compose_cmd[@]}" || compose_exit=$?
    else
      "${compose_cmd[@]}" || compose_exit=$?
    fi
  fi

  if [[ "${compose_exit}" -ne 0 ]]; then
    exit "${compose_exit}"
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
  for op_num in $(resolved_operator_ids); do
    operator_compose_env_file="$(operator_compose_env_file_path "${op_num}")"
    operator_runtime_env_file="$(operator_runtime_env_file_path "${op_num}")"
    project_name="$(compose_project_name_for_operator "${op_num}")"
    if ! require_operator_env_file "${operator_compose_env_file}" \
      || ! require_operator_env_file "${operator_runtime_env_file}"; then
      exit 1
    fi
    docker compose -p "${project_name}" -f docker-compose.yml -f "$(compose_override_file)" --env-file "${ENV_FILE}" --env-file "${operator_compose_env_file}" --env-file "${operator_runtime_env_file}" down --volumes
  done
  # CI / fast restarts: host port bindings can linger briefly after down (TIME_WAIT / proxy).
  echo "Waiting for host port bindings to release after teardown..."
  sleep 3
fi

if [[ "${IS_STARTUP_COMMAND}" == true ]] && uses_shared_bitvmx_network; then
  NETWORK_NAME="bitvmx-shared-network"
  if ! docker network inspect "${NETWORK_NAME}" >/dev/null 2>&1; then
    echo "Creating docker network '${NETWORK_NAME}'..."
    docker network create --driver bridge --subnet=172.20.0.0/16 "${NETWORK_NAME}"
  fi
fi

for op_num in $(resolved_operator_ids); do
  project_name="$(compose_project_name_for_operator "${op_num}")"
  operator_compose_env_file="$(operator_compose_env_file_path "${op_num}")"
  operator_runtime_env_file="$(operator_runtime_env_file_path "${op_num}")"
  if ! require_operator_env_file "${operator_compose_env_file}" \
    || ! require_operator_env_file "${operator_runtime_env_file}"; then
    exit 1
  fi
  run_compose_stack "${project_name}" "${op_num}" "${operator_compose_env_file}" "${operator_runtime_env_file}"
done
