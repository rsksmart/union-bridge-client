#!/bin/bash

# start a new process group for this script
set -meuo pipefail

FEATURES=""
if [[ -n "${1:-}" ]]; then
    FEATURES="--features $1"
fi

# get the config path from the first argument, default to "local" if not provided
CONFIG_NAME=${2:-local}

# config path parameter
CONFIG_PARAM="-- --config-path ./config/$CONFIG_NAME"

# array to store service PIDs in order of startup
SERVICE_PIDS=()
SERVICE_NAMES=()

# Function to clean up all background processes on script termination
function cleanup() {
    # kill services in reverse order
    for ((i=${#SERVICE_PIDS[@]}-1; i>=0; i--)); do
        local pid=${SERVICE_PIDS[$i]}
        local name=${SERVICE_NAMES[$i]}
        if kill -0 $pid 2>/dev/null; then
            echo "Stopping $name (PID: $pid)..."
            kill $pid
            sleep 1 # wait for graceful shutdown
            # Force kill if still running
            if kill -0 $pid 2>/dev/null; then
                echo "Force stopping $name (PID: $pid)..."
                kill -9 $pid 2>/dev/null
            fi
        fi
    done

    # cleanup any remaining processes just in case
    pkill -P $$ 2>/dev/null
    sleep 1
    pkill -9 -P $$ 2>/dev/null

    echo "All services stopped."
}

# trap various signals to ensure cleanup function is executed when the script is interrupted or exits
trap cleanup INT TERM EXIT

# function to run a service in the background
 function run_service() {
     local service=$1
     echo "Starting $service: cargo run --bin $service $CONFIG_PARAM $FEATURES"

     # start the service in the foreground first to verify successful startup
     cargo run --bin "$service" $FEATURES &  # Background it
     local pid=$!
     SERVICE_PIDS+=($pid)
     SERVICE_NAMES+=("$service")
     echo "$service started with PID $pid"

     # verify the service is still running
     sleep 1
     if ! kill -0 $pid 2>/dev/null; then
         echo "Service $service failed to start properly"
         cleanup
         exit 1
     fi
 }

# Start other services in the background
echo "Starting other services..."
run_service "block-indexer"
run_service "log-indexer"
run_service "transaction-dispatcher"
sleep 2 # give some time for indexers to initialize
run_service "coordinator"
# wait for the services to finish

echo
echo "All services started successfully, you can check log files. Press Ctrl+C to shut down all services in the correct order."

# wait for all background processes to complete or until a signal is received
wait