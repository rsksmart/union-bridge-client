#!/usr/bin/env bash
set -u

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"
MODE="${1:-}"
BASE_STORAGE_PATH="${BASE_STORAGE_PATH:-$HOME}"
STOP_DATABASE_CONTAINERS_CONFIRMED=false
STOP_DATABASE_CONTAINERS_DENIED=false

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

print_capture_result() {
  local subject="$1"
  local rc="$2"

  if [[ "${rc}" -eq 0 ]]; then
    echo "[${display_ts}] Captured ${subject}"
  else
    echo "[${display_ts}] Failed to capture ${subject} exit_code=${rc}"
  fi
}

confirm_stop_database_containers() {
  local answer=""

  if [[ "${STOP_DATABASE_CONTAINERS_CONFIRMED}" == "true" ]]; then
    return 0
  fi
  if [[ "${STOP_DATABASE_CONTAINERS_DENIED}" == "true" ]]; then
    return 1
  fi

  if [[ ! -t 0 ]]; then
    echo "[${display_ts}] Cannot back up databases without confirmation to stop containers."
    STOP_DATABASE_CONTAINERS_DENIED=true
    return 1
  fi

  echo ""
  echo "BitVMX database backup requires stopping BitVMX containers."
  echo "Containers will be stopped with docker stop, volumes will be kept, and containers will remain stopped after backup."
  read -r -p "Continue? [y/N] " answer
  if [[ "${answer}" =~ ^[Yy]$ ]]; then
    STOP_DATABASE_CONTAINERS_CONFIRMED=true
    return 0
  fi

  STOP_DATABASE_CONTAINERS_DENIED=true
  return 1
}

confirm_stop_database_containers

strip_ansi() {
  perl -pe 's/\e\[[0-?]*[ -\/]*[@-~]//g'
}

mkdir -p "${backup_root}" "${log_dir}"
echo "[${display_ts}] Created log directory: ${log_dir}"

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

# Resolve by explicit container name first (matches container_name in compose files, e.g.
# docker/local-infra/docker-compose.bitvmx.yml -> bitvmx-client-1).
docker_container_id_from_name() {
  local name="$1"
  docker inspect -f '{{.Id}}' "${name}" 2>/dev/null || true
}

# Args: label preferred_container_name project service
# preferred_container_name may be empty to skip and use compose labels only.
capture_container_logs() {
  local label="$1"
  local preferred_name="${2:-}"
  local project="$3"
  local service="$4"
  local subject="Docker logs for ${label}"
  local container_id=""

  if [[ -n "${preferred_name}" ]]; then
    container_id="$(docker_container_id_from_name "${preferred_name}")"
  fi
  if [[ -z "${container_id}" ]]; then
    container_id="$(find_compose_container_id "${project}" "${service}")"
  fi
  if [[ -z "${container_id}" ]]; then
    local hint=""
    if [[ -n "${preferred_name}" ]]; then
      hint=" (tried name=${preferred_name}"
    fi
    if [[ -n "${hint}" ]]; then
      hint="${hint}, compose project=${project} service=${service})"
    else
      hint=" (compose project=${project} service=${service})"
    fi
    capture_missing "${label}" "No container found${hint}" "${subject}"
    return
  fi

  capture_command "${label}" "${subject}" docker logs "${container_id}"
}

backup_bitvmx_container_database() {
  local label="$1"
  local preferred_name="${2:-}"
  local project="$3"
  local service="$4"
  local subject="BitVMX database for ${label}"
  local destination_file="${log_dir}/${label}-database.tar.gz"
  local stderr_file="${destination_file}.stderr"
  local container_id=""
  local container_image
  local was_running
  local rc

  if ! confirm_stop_database_containers; then
    capture_missing "${label}-database" "Skipped database backup because container stop was not confirmed." "${subject}"
    return
  fi

  if [[ -n "${preferred_name}" ]]; then
    container_id="$(docker_container_id_from_name "${preferred_name}")"
  fi
  if [[ -z "${container_id}" ]]; then
    container_id="$(find_compose_container_id "${project}" "${service}")"
  fi
  if [[ -z "${container_id}" ]]; then
    local hint=""
    if [[ -n "${preferred_name}" ]]; then
      hint=" (tried name=${preferred_name}"
    fi
    if [[ -n "${hint}" ]]; then
      hint="${hint}, compose project=${project} service=${service})"
    else
      hint=" (compose project=${project} service=${service})"
    fi
    capture_missing "${label}-database" "No container found${hint}" "${subject}"
    return
  fi

  was_running="$(docker inspect -f '{{.State.Running}}' "${container_id}" 2> "${stderr_file}")"
  rc=$?
  if [[ "${rc}" -ne 0 ]]; then
    print_capture_result "${subject}" "${rc}"
    return
  fi

  container_image="$(docker inspect -f '{{.Config.Image}}' "${container_id}" 2> "${stderr_file}")"
  rc=$?
  if [[ "${rc}" -ne 0 ]]; then
    print_capture_result "${subject}" "${rc}"
    return
  fi

  if [[ "${was_running}" == "true" ]]; then
    docker stop "${container_id}" > /dev/null 2> "${stderr_file}"
    rc=$?
    if [[ "${rc}" -ne 0 ]]; then
      print_capture_result "${subject}" "${rc}"
      return
    fi
  fi

  docker run --rm \
    --volumes-from "${container_id}" \
    --entrypoint tar \
    "${container_image}" \
    -C /tmp -czf - . > "${destination_file}" 2> "${stderr_file}"
  rc=$?

  if [[ "${rc}" -eq 0 && ! -s "${stderr_file}" ]]; then
    rm -f "${stderr_file}"
  fi

  print_capture_result "${subject}" "${rc}"
}

# Mirrors scripts/run-clients.sh: UB_LOG_DIR overrides; else logs/latest (timestamped run); else logs/.
resolve_local_union_logs_dir() {
  local logs_root="${PROJECT_ROOT}/logs"

  if [[ -n "${UB_LOG_DIR:-}" ]]; then
    if [[ "${UB_LOG_DIR}" == /* ]]; then
      printf '%s\n' "${UB_LOG_DIR}"
    else
      printf '%s\n' "${PROJECT_ROOT}/${UB_LOG_DIR}"
    fi
    return
  fi

  if [[ -L "${logs_root}/latest" ]]; then
    printf '%s\n' "${logs_root}/$(readlink "${logs_root}/latest")"
    return
  fi

  printf '%s\n' "${logs_root}"
}

backup_local_coordinator_logs() {
  local source_dir
  source_dir="$(resolve_local_union_logs_dir)"
  local copied_any=false
  local path
  local instance
  local target_path

  echo "[${display_ts}] Local union logs source: ${source_dir}"

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
    capture_missing "coordinator" "No local coordinator-*.log files in ${source_dir}" "local coordinator logs"
  fi
}

backup_local_mode() {
  backup_local_coordinator_logs

  for i in 1 2 3 4; do
    capture_container_logs "bitvmx-client-${i}" "bitvmx-client-${i}" "bitvmx" "bitvmx-client-${i}"
    backup_bitvmx_container_database "bitvmx-client-${i}" "bitvmx-client-${i}" "bitvmx" "bitvmx-client-${i}"
  done
}

backup_docker_mode() {
  local op_env_dir

  for i in 1 2 3 4; do
    op_env_dir="${BASE_STORAGE_PATH}/.union_bridge/op_${i}"
    if [[ ! -d "${op_env_dir}" ]]; then
      capture_missing "operator-${i}-env" "Missing operator directory: ${op_env_dir}"
    fi

    capture_container_logs "coordinator-${i}" "coordinator-${i}" "op_${i}" "coordinator"
    capture_container_logs "bitvmx-client-${i}" "bitvmx-client-${i}" "op_${i}" "bitvmx-client"
    backup_bitvmx_container_database "bitvmx-client-${i}" "bitvmx-client-${i}" "op_${i}" "bitvmx-client"
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
