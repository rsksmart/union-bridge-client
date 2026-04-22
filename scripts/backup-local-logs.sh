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

capture_command() {
  local label="$1"
  local subject="$2"
  local destination_file="${log_dir}/${label}.log"
  shift 2

  "$@" > "${destination_file}" 2>&1
  local rc=$?
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

latest_coordinator_start_line() {
  local path="$1"

  awk '/Loading configuration profile:/ { line = NR } END { if (line) print line }' "${path}"
}

list_coordinator_instances() {
  local source_dir="$1"
  local path
  local base
  local instance
  local instances=""

  shopt -s nullglob
  for path in "${source_dir}"/coordinator-*.log; do
    base="$(basename "${path}")"
    if [[ "${base}" =~ ^coordinator-([0-9]+)(\.[0-9]+)?\.log$ ]]; then
      instance="${BASH_REMATCH[1]}"
      case " ${instances} " in
        *" ${instance} "*) ;;
        *) instances="${instances} ${instance}" ;;
      esac
    fi
  done
  shopt -u nullglob

  for instance in ${instances}; do
    printf '%s\n' "${instance}"
  done | sort -n
}

build_latest_coordinator_session_snapshot() {
  local raw_dir="$1"
  local snapshot_root="$2"
  local instance="$3"
  local snapshot_path="${snapshot_root}/union-client-${instance}.log"
  local merged_path="${snapshot_path}.tmp"
  local path
  local last_start_line
  local rotated_paths=()

  rm -f "${merged_path}"
  : > "${merged_path}"

  shopt -s nullglob
  rotated_paths=( "${raw_dir}/coordinator-${instance}."*.log )
  shopt -u nullglob

  if [[ ${#rotated_paths[@]} -gt 0 ]]; then
    while IFS= read -r path; do
      [[ -n "${path}" ]] || continue
      cat "${path}" >> "${merged_path}"
    done < <(
      for path in "${rotated_paths[@]}"; do
        if [[ "$(basename "${path}")" =~ ^coordinator-[0-9]+\.([0-9]+)\.log$ ]]; then
          printf '%08d\t%s\n' "${BASH_REMATCH[1]}" "${path}"
        fi
      done | sort -r -n | cut -f2-
    )
  fi

  if [[ -f "${raw_dir}/coordinator-${instance}.log" ]]; then
    cat "${raw_dir}/coordinator-${instance}.log" >> "${merged_path}"
  fi

  if [[ ! -s "${merged_path}" ]]; then
    rm -f "${merged_path}"
    return 1
  fi

  last_start_line="$(latest_coordinator_start_line "${merged_path}")"
  if [[ -n "${last_start_line}" ]]; then
    tail -n +"${last_start_line}" "${merged_path}" > "${snapshot_path}"
    echo "[${display_ts}] Extracted latest run snapshot for coordinator-${instance}.log (start line ${last_start_line})"
    rm -f "${merged_path}"
  else
    mv "${merged_path}" "${snapshot_path}"
    echo "[${display_ts}] Extracted latest run snapshot for coordinator-${instance}.log (no startup marker)"
  fi
}

backup_local_coordinator_logs() {
  local source_dir="${PROJECT_ROOT}/logs"
  local temp_dir=""
  local copied_any=false
  local built_snapshot=false
  local instance

  if [[ ! -d "${source_dir}" ]]; then
    capture_missing "coordinator" "Missing local log directory: ${source_dir}" "local coordinator logs"
    return
  fi

  temp_dir="$(mktemp -d "${log_dir}/.coordinator.XXXXXX")"
  trap "cleanup_dir '${temp_dir}'" RETURN

  for path in "${source_dir}"/coordinator-*.log; do
    local target_path

    if [[ ! -e "${path}" ]]; then
      continue
    fi
    copied_any=true
    target_path="${temp_dir}/$(basename "${path}")"
    cp "${path}" "${target_path}"
  done

  if [[ "${copied_any}" != "true" ]]; then
    capture_missing "coordinator" "No local coordinator logs found in ${source_dir}" "local coordinator logs"
    return
  fi

  while IFS= read -r instance; do
    [[ -n "${instance}" ]] || continue
    if build_latest_coordinator_session_snapshot "${temp_dir}" "${log_dir}" "${instance}"; then
      built_snapshot=true
    fi
  done < <(list_coordinator_instances "${temp_dir}")

  if [[ "${built_snapshot}" != "true" ]]; then
    echo "[${display_ts}] Copied local coordinator logs but extracted no cargo run snapshots"
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
