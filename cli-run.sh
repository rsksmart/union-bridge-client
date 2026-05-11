#!/usr/bin/env bash

# wrapper script for run to run local operators
# usage: ./cli-run.sh --id 1 --fresh
#        ./cli-run.sh --features anvil
#        ./cli-run.sh --bitvmx-mode repo
#        ./cli-run.sh --help
#        ./cli-run.sh --logs         # follow logs from latest run
#        ./cli-run.sh --kill          # kill all existing running services and exit

set -euo pipefail

# change to script directory to ensure relative paths work
cd "$(dirname "$0")"

notify_terminal_bell() {
  printf '\a'
}

notify_os_notification() {
  local title="$1"
  local message="$2"

  if [[ "$(uname -s)" != "Darwin" ]] || ! command -v osascript &> /dev/null; then
    return 1
  fi

  osascript - "$title" "$message" <<'APPLESCRIPT' &> /dev/null
on run argv
    set notificationTitle to item 1 of argv
    set notificationMessage to item 2 of argv
    display notification notificationMessage with title notificationTitle
end run
APPLESCRIPT
}

notify_completion() {
  local title="$1"
  local message="$2"

  notify_os_notification "$title" "$message" || notify_terminal_bell
}

resolve_logs_dir() {
  if [[ -n "${UB_LOG_DIR:-}" ]]; then
    mkdir -p "${UB_LOG_DIR}"
    printf '%s\n' "${UB_LOG_DIR}"
    return
  fi

  local ts_date ts_time log_subdir
  ts_date="$(date +%y%m%d)"
  ts_time="$(date +%H%M%S)"
  log_subdir="logs/${ts_date}/${ts_time}"
  mkdir -p "${log_subdir}"
  ln -sfn "${ts_date}/${ts_time}" logs/latest
  printf '%s\n' "${log_subdir}"
}

latest_logs_dir() {
  if [[ -n "${UB_LOG_DIR:-}" ]]; then
    printf '%s\n' "${UB_LOG_DIR}"
    return
  fi
  if [[ -L "logs/latest" ]]; then
    printf '%s\n' "logs/$(readlink logs/latest)"
    return
  fi
  printf '%s\n' "logs"
}

# handle --logs option
if [[ "${1:-}" == "--logs" ]]; then
  logs_dir="$(latest_logs_dir)"
  echo "Following logs from ${logs_dir}"

  (
    pids=()

    # kill all children on Ctrl+C (INT) or TERM
    cleanup() {
      kill "${pids[@]}" 2>/dev/null || true
      exit 0
    }
    trap cleanup INT TERM

    for i in {1..4}; do
      tail -n0 -F "${logs_dir}/coordinator-$i.log" | sed "s/^/[op-$i] /" &
      pids+=($!)
    done

    wait
  )
  exit 0
fi

UB_LOG_DIR="$(resolve_logs_dir)"
export UB_LOG_DIR

echo "Logging to ${UB_LOG_DIR}"

selected_client_ids() {
  local ids=(1 2 3 4)
  local arg next

  while [[ $# -gt 0 ]]; do
    arg="$1"
    case "$arg" in
      --id)
        next="${2:-}"
        if [[ "$next" =~ ^[1-4]$ ]]; then
          ids=("$next")
        fi
        shift 2
        ;;
      --id=[1-4])
        ids=("${arg#--id=}")
        shift
        ;;
      -i)
        next="${2:-}"
        if [[ "$next" =~ ^[1-4]$ ]]; then
          ids=("$next")
        fi
        shift 2
        ;;
      *)
        shift
        ;;
    esac
  done

  printf '%s\n' "${ids[@]}"
}

port_is_listening() {
  local port="$1"
  (echo > "/dev/tcp/127.0.0.1/${port}") &> /dev/null
}

notify_when_clients_ready() {
  local ids=("$@")
  local deadline=$((SECONDS + 180))

  while (( SECONDS < deadline )); do
    local all_ready=true
    local id port

    for id in "${ids[@]}"; do
      for port in "$((10000 + id))" "$((20000 + id))" "$((30000 + id))" "$((40000 + id))"; do
        if ! port_is_listening "$port"; then
          all_ready=false
          break 2
        fi
      done
    done

    if [[ "$all_ready" == true ]]; then
      sleep 1
      notify_completion "Union Bridge clients" "Ready"
      return 0
    fi

    sleep 1
  done
}

should_notify_ready() {
  local arg
  for arg in "$@"; do
    case "$arg" in
      --help|-h|--kill)
        return 1
        ;;
    esac
  done
  return 0
}

cleanup_background_jobs() {
  if [[ -n "${ready_notify_pid:-}" ]]; then
    kill "$ready_notify_pid" 2> /dev/null || true
  fi
}

set +e
ready_notify_pid=""
if should_notify_ready "$@"; then
  mapfile -t ready_ids < <(selected_client_ids "$@")
  notify_when_clients_ready "${ready_ids[@]}" &
  ready_notify_pid=$!
fi

trap cleanup_background_jobs EXIT
RUST_BACKTRACE=1 cargo run --manifest-path cli/run/Cargo.toml -- "$@"
exit_code=$?
cleanup_background_jobs
set -e
exit "$exit_code"
