#!/usr/bin/env bash

# wrapper script for run to run local operators
# usage: ./cli-run.sh --id 1 --fresh
#        ./cli-run.sh --features anvil
#        ./cli-run.sh --help
#        ./cli-run.sh --logs
#        ./cli-run.sh --start-mine    # start background mining (anvil + bitcoin)
#        ./cli-run.sh --stop-mine     # stop background mining
#        ./cli-run.sh --kill          # kill all existing running services and exit

set -euo pipefail

# change to script directory to ensure relative paths work
cd "$(dirname "$0")"

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

# forward all arguments to run
RUST_BACKTRACE=1 exec cargo run --manifest-path cli/run/Cargo.toml -- "$@"
