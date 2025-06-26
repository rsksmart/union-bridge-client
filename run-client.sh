#!/bin/bash

# start a new process group for this script
set -meuo pipefail

FEATURES=""
if [[ -n "${1:-}" ]]; then
    FEATURES="--features $1"
fi

# get the config path from the second argument, default to "local" if not provided
CONFIG_NAME=${2:-local}
CONFIG_PARAM="-- --config-path ./config/$CONFIG_NAME"

SERVICE_PIDS=()
SERVICE_NAMES=()

# Define our services
SERVICES=("block-indexer" "log-indexer" "transaction-dispatcher" "coordinator")

function cleanup() {
    # kill in reverse order
    for ((i=${#SERVICE_PIDS[@]}-1; i>=0; i--)); do
        pid=${SERVICE_PIDS[$i]}
        name=${SERVICE_NAMES[$i]}
        if kill -0 "$pid" &>/dev/null; then
            echo "Stopping $name (PID $pid)..."
            kill "$pid"
            sleep 1
            if kill -0 "$pid" &>/dev/null; then
                echo "Force stopping $name (PID $pid)..."
                kill -9 "$pid" &>/dev/null || true
            fi
        fi
    done

    # cleanup any remaining child processes
    pkill -P $$ &>/dev/null || true
    sleep 1
    pkill -9 -P $$ &>/dev/null || true

    echo "All services stopped."
}

trap cleanup INT TERM EXIT

function is_service_running() {
    local svc=$1
    # Look for exact binary name in process list
    if pgrep -f "target/debug/$svc" &>/dev/null; then
        return 0 # true, already running
    fi
    return 1 # false, not running
}

# Stop a running service, with force if needed
function stop_service() {
    local svc=$1
    echo "WARNING: Service '$svc' is already running. Stopping it..."
    pkill -f "target/debug/$svc"
    sleep 1
    # Force kill if still running
    if is_service_running "$svc"; then
        echo "Force stopping '$svc'..."
        pkill -9 -f "target/debug/$svc"
        sleep 1
    fi
}

function run_service() {
    local svc=$1

    # check if this service is already running
    if is_service_running "$svc"; then
        stop_service "$svc"
    fi

    echo "Starting $svc: cargo run --bin $svc $FEATURES $CONFIG_PARAM"

    cargo run --bin $svc $FEATURES $CONFIG_PARAM &
    pid=$!
    SERVICE_PIDS+=("$pid")
    SERVICE_NAMES+=("$svc")
    echo "$svc started (PID $pid)"

    # quick check: did it exit immediately?
    sleep 1
    if ! kill -0 "$pid" &>/dev/null; then
        echo "ERROR: $svc failed to start"
        exit 1
    fi
}

# First, check if any of our services are already running before we start
echo "Checking for existing service instances..."
for svc in "${SERVICES[@]}"; do
    if is_service_running "$svc"; then
        stop_service "$svc"
    fi
done

# prepare bitvmx-client dependency
git -C ../rust-bitvmx-workspace/ checkout f5d15597ee27f9a659498ef5fb86f4ecee094d51
git -C ../rust-bitvmx-workspace/ submodule update --init --recursive

# Start services in the background
echo "Starting services..."
run_service "block-indexer"
run_service "log-indexer"
run_service "transaction-dispatcher"
sleep 2 # give some time for indexers to initialize
run_service "coordinator"
sleep 2 # wait for the coordinator to finish

echo
echo "All services launched. Monitoring for failures..."
echo "Press Ctrl+C to shut down cleanly."

# Monitor: if any service PID disappears, abort
while true; do
    for i in "${!SERVICE_PIDS[@]}"; do
        pid=${SERVICE_PIDS[$i]}
        name=${SERVICE_NAMES[$i]}
        if ! kill -0 "$pid" &>/dev/null; then
            echo "ERROR: $name (PID $pid) exited unexpectedly."
            exit 1
        fi
    done
    sleep 1
done