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

# forward all arguments to run
RUST_BACKTRACE=1 exec cargo run --manifest-path cli/run/Cargo.toml -- "$@"
