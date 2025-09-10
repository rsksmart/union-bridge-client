#!/bin/bash

# start a new process group for this script
set -euo pipefail

# Usage function
function usage() {
    echo "Usage: $0 [OPTIONS]"
    echo ""
    echo "Options:"
    echo "  --features FEATURES             Optional comma-separated list of Cargo features. No default."
    echo "  --config CONFIG_NAME            Optional config directory name under ./config/. Defaults to 'local'."
    echo "  --logger LOGGER_FILE       Optional logger configuration file path. Defaults to 'log4rs.yaml'."
    echo "  --help, -h                      Show this help message"
    echo ""
    echo "Examples:"
    echo "  $0                                                                        # Run with default settings"
    echo "  $0 --features anvil                                                       # Run with 'anvil' feature"
    echo "  $0 --config config/qa-local                                               # Run with 'local' config"
    echo "  $0 --logger log4rs.yaml                                              # Run with custom logger config"
    echo "  $0 --features anvil --config config/qa-local --logger-path custom.yaml    # Run with all options"
    exit 1
}

# Initialize variables with defaults
FEATURES=""
CONFIG_PARAM="--config-path ./config/local"     # Default config: local
LOGGER_PARAM="--logger-path log4rs.stdout.yaml" # Default logger: stdout

# Parse named parameters
while [[ $# -gt 0 ]]; do
    case $1 in
        --features)
            if [[ -n "${2:-}" ]]; then
                FEATURES="--features $2"
                shift 2
            fi
            ;;
        --config)
            if [[ -n "${2:-}" ]]; then
                CONFIG_PARAM="--config-path $2"
                shift 2
            fi
            ;;
        --logger)
            if [[ -n "${2:-}" ]]; then
                LOGGER_PARAM="--logger-path $2"
                shift 2
            fi
            ;;
        --help|-h)
            usage
            ;;
        *)
            echo "Error: Unknown option $1"
            usage
            ;;
    esac
done

SERVICE_PIDS=()
SERVICE_NAMES=()
CLEANING_UP=false

# Define our services: order matters since some depend on others
SERVICES=("block-indexer" "log-indexer" "transaction-dispatcher" "user-api" "coordinator")

function cleanup() {
    if $CLEANING_UP; then
        return
    fi
    
    CLEANING_UP=true
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

    exit 130
}

trap cleanup INT TERM EXIT

function run_service() {
    local svc=$1

    cargo_run_cmd="cargo run --bin $svc $FEATURES -- $LOGGER_PARAM $CONFIG_PARAM"
    echo "Starting $svc: $cargo_run_cmd"
    $cargo_run_cmd &

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


# Start services in the background
echo "Starting services..."
for svc in "${SERVICES[@]}"; do
    # coordinator depends on the others, so we wait a bit before starting it
    if [[ "$svc" == "coordinator" ]]; then
        sleep 2
    fi
    run_service "$svc"
done

# Give services a moment to stabilize
sleep 2

echo
echo "All services launched. Monitoring for failures..."
echo "Press Ctrl+C to shut down cleanly."

# Monitor: if any service PID disappears, abort
while true; do
    for i in "${!SERVICE_PIDS[@]}"; do
        pid=${SERVICE_PIDS[$i]}
        name=${SERVICE_NAMES[$i]}
        if ! kill -0 "$pid" &>/dev/null; then
            echo "At least one service finished unexpectedly, stopping all..." >&2
            cleanup
            exit 1
        fi
    done
    sleep 1
done
