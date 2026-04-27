#!/usr/bin/env bash

# wrapper script for run to run local operators
# usage: ./cli-run.sh --id 1 --fresh
#        ./cli-run.sh --features anvil
#        ./cli-run.sh --bitvmx-mode repo
#        ./cli-run.sh --help
#        ./cli-run.sh --logs
#        ./cli-run.sh --kill          # kill all existing running services and exit

set -euo pipefail

# change to script directory to ensure relative paths work
cd "$(dirname "$0")"

rotate_logs_dir() {
  local timestamp
  local rotated_dir
  local log_files=()
  local log_file

  if [[ ! -d "logs" ]]; then
    return
  fi

  while IFS= read -r -d '' log_file; do
    log_files+=("${log_file}")
  done < <(find "logs" -mindepth 1 -maxdepth 1 -type f -print0)

  if [[ "${#log_files[@]}" -eq 0 ]]; then
    return
  fi

  timestamp="$(date +%Y%m%d%H%M%S)"
  rotated_dir="logs/${timestamp}"

  while [[ -e "${rotated_dir}" ]]; do
    timestamp="$(date +%Y%m%d%H%M%S)"
    rotated_dir="logs/${timestamp}"
    sleep 1
  done

  mkdir -p "${rotated_dir}"
  mv "${log_files[@]}" "${rotated_dir}/"
}

# handle --logs option
if [[ "${1:-}" == "--logs" ]]; then
  (
    pids=()

    # kill all children on Ctrl+C (INT) or TERM
    cleanup() {
      kill "${pids[@]}" 2>/dev/null || true
      exit 0
    }
    trap cleanup INT TERM

    for i in {1..4}; do
      tail -n0 -F "logs/coordinator-$i.log" | sed "s/^/[op-$i] /" &
      pids+=($!)
    done

    wait
  )
  exit 0
fi

rotate_logs_dir

# forward all arguments to run
RUST_BACKTRACE=1 exec cargo run --manifest-path cli/run/Cargo.toml -- "$@"
