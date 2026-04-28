#!/usr/bin/env bash
set -u

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"
MODE="${1:-}"
BASE_STORAGE_PATH="${BASE_STORAGE_PATH:-$HOME}"

usage() {
  echo "Usage: $0 <local|docker>"
}

if [[ -z "${MODE}" ]]; then
  usage
  exit 1
fi

case "${MODE}" in
  local|docker)
    ;;
  *)
    echo "Invalid mode: ${MODE}"
    usage
    exit 1
    ;;
esac

display_ts="$(date '+%Y-%m-%d %H:%M:%S')"
dir_ts="$(date +%Y%m%d%H%M%S)"
backup_root="${HOME}/tmp/union_bridge_logs_backup"
log_dir="${backup_root}/${MODE}/${dir_ts}"

mkdir -p "${backup_root}" "${log_dir}"
echo "[${display_ts}] Created log directory: ${log_dir}"

print_capture_result() {
  local subject="$1"
  local rc="$2"

  if [[ "${rc}" -eq 0 ]]; then
    echo "[${display_ts}] Captured ${subject}"
  else
    echo "[${display_ts}] Failed to capture ${subject} exit_code=${rc}"
  fi
}

strip_ansi() {
  perl -pe 's/\e\[[0-?]*[ -\/]*[@-~]//g'
}

capture_command() {
  local label="$1"
  local subject="$2"
  local destination_file="${log_dir}/${label}.log"
  shift 2

  "$@" 2>&1 | strip_ansi > "${destination_file}"
  local pipeline_status=("${PIPESTATUS[@]}")
  local command_rc="${pipeline_status[0]}"
  local strip_rc="${pipeline_status[1]}"
  local rc="${command_rc}"
  if [[ "${rc}" -eq 0 && "${strip_rc}" -ne 0 ]]; then
    rc="${strip_rc}"
  fi
  print_capture_result "${subject}" "${rc}"
}

capture_missing() {
  local label="$1"
  local message="$2"
  local subject="${3:-${label}}"
  local destination_file="${log_dir}/${label}.log"

  printf '%s\n' "${message}" > "${destination_file}"
  echo "[${display_ts}] Failed to capture ${subject}: ${message}"
}

find_compose_container_id() {
  local project="$1"
  local service="$2"

  docker ps -a \
    --filter "label=com.docker.compose.project=${project}" \
    --filter "label=com.docker.compose.service=${service}" \
    --format '{{.ID}}' \
    | head -n 1
}

capture_container_logs() {
  local label="$1"
  local project="$2"
  local service="$3"
  local subject="Docker logs for ${label}"
  local container_id

  container_id="$(find_compose_container_id "${project}" "${service}")"
  if [[ -z "${container_id}" ]]; then
    capture_missing "${label}" "No container found for project=${project} service=${service}" "${subject}"
    return
  fi

  capture_command "${label}" "${subject}" docker logs "${container_id}"
}

cleanup_dir() {
  local path="$1"

  [[ -n "${path}" ]] || return 0
  [[ -d "${path}" ]] || return 0
  rm -rf "${path}"
}

backup_local_coordinator_logs() {
  local source_dir="${PROJECT_ROOT}/logs"
  local copied_any=false
  local path
  local instance
  local target_path

  if [[ ! -d "${source_dir}" ]]; then
    capture_missing "coordinator" "Missing local log directory: ${source_dir}" "local coordinator logs"
    return
  fi

  for path in "${source_dir}"/coordinator-*.log; do
    if [[ ! -e "${path}" ]]; then
      continue
    fi
    copied_any=true
    if [[ "$(basename "${path}")" =~ ^coordinator-([0-9]+)\.log$ ]]; then
      instance="${BASH_REMATCH[1]}"
      target_path="${log_dir}/union-client-${instance}.log"
      cp "${path}" "${target_path}"
      echo "[${display_ts}] Copied current coordinator-${instance}.log to $(basename "${target_path}")"
    fi
  done

  if [[ "${copied_any}" != "true" ]]; then
    capture_missing "coordinator" "No local coordinator logs found in ${source_dir}" "local coordinator logs"
  fi
}

backup_local_mode() {
  backup_local_coordinator_logs

  for i in 1 2 3 4; do
    capture_container_logs "bitvmx-client-${i}" "bitvmx" "bitvmx-client-${i}"
  done
}

backup_docker_mode() {
  local op_env_dir

  for i in 1 2 3 4; do
    op_env_dir="${BASE_STORAGE_PATH}/.union_bridge/op_${i}"
    if [[ ! -d "${op_env_dir}" ]]; then
      capture_missing "operator-${i}-env" "Missing operator directory: ${op_env_dir}"
    fi

    capture_container_logs "coordinator-${i}" "op_${i}" "coordinator"
    capture_container_logs "bitvmx-client-${i}" "op_${i}" "bitvmx-client"
  done
}

case "${MODE}" in
  local)
    backup_local_mode
    ;;
  docker)
    backup_docker_mode
    ;;
esac
