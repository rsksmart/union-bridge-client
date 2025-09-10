#!/bin/bash

set -euo pipefail

# Default values
NUM_CLIENTS=""
FEATURES=""

# Function to show help
show_help() {
    cat << EOF
Usage: $0 [OPTIONS]

Run multiple Union Bridge clients simultaneously.

OPTIONS:
    -n, --num-clients NUM     Number of clients to run (1-10, required)
    -f, --features FEATURES   Optional features flag for clients
    -h, --help                Show this help message

ENVIRONMENT VARIABLES:
    BASE_STORAGE_PATH         Required. Base path for client data storage

EXAMPLES:
    $0 --num-clients 4                    # Run 4 clients
    $0 -n 6 --features my-feature         # Run 6 clients with features
    
    # With environment variable:
    BASE_STORAGE_PATH=/Users/username $0 --num-clients 4

EOF
}

# Parse command line arguments
while [[ $# -gt 0 ]]; do
    case $1 in
        -n|--num-clients)
            NUM_CLIENTS="$2"
            shift 2
            ;;
        -f|--features)
            FEATURES="$2"
            shift 2
            ;;
        -h|--help)
            show_help
            exit 0
            ;;
        *)
            echo "Error: Unknown option '$1'"
            echo "Use --help for usage information."
            exit 1
            ;;
    esac
done

# Show help if no arguments provided
if [[ -z "$NUM_CLIENTS" ]]; then
    show_help
    exit 0
fi

# Validate NUM_CLIENTS parameter
if [[ ! "$NUM_CLIENTS" =~ ^([1-9]|10)$ ]]; then
    echo "Error: NUM_CLIENTS must be between 1 and 10."
    echo "Use --help for usage information."
    exit 1
fi

readonly NUM_CLIENTS

# Validate BASE_STORAGE_PATH environment variable is set
if [[ -z "${BASE_STORAGE_PATH:-}" ]]; then
    echo "Error: BASE_STORAGE_PATH environment variable is required but not set."
    echo ""
    echo "Please set the BASE_STORAGE_PATH environment variable before running this script:"
    echo "  export BASE_STORAGE_PATH=/Users/username"
    echo ""
    echo "Use --help for more information and examples."
    exit 1
fi

# Validate FEATURES if provided
if [[ -n "$FEATURES" && ! "$FEATURES" =~ ^[a-zA-Z0-9_-]+$ ]]; then
    echo "Error: FEATURES must be a single word containing only letters, numbers, underscores, or hyphens."
    echo "Use --help for more information."
    exit 1
fi

CLIENT_PIDS=()
CLIENT_NAMES=()
CLEANING_UP=false

function cleanup() {
    if $CLEANING_UP; then
        return
    fi

    CLEANING_UP=true

    echo "Shutting down ${#CLIENT_PIDS[@]} clients..."
    
    # Step 1: Send SIGTERM to all clients for graceful shutdown
    for ((i=0; i<${#CLIENT_PIDS[@]}; i++)); do
        pid=${CLIENT_PIDS[$i]}
        name=${CLIENT_NAMES[$i]}
        if kill -0 "$pid" &>/dev/null; then
            echo "Stopping $name (PID $pid)..."
            kill -TERM "$pid" &>/dev/null || true
        else
            echo "$name (PID $pid) already stopped"
        fi
    done
    
    # Step 2: Wait for graceful shutdown - check PIDs until they're gone or timeout
    echo "Waiting for graceful shutdown..."
    for i in {1..30}; do
        # Check if any PIDs are still running
        still_running=false
        for ((j=0; j<${#CLIENT_PIDS[@]}; j++)); do
            pid=${CLIENT_PIDS[$j]}
            if kill -0 "$pid" &>/dev/null; then
                still_running=true
                break
            fi
        done
        
        # If no PIDs are running, we're done!
        if [[ "$still_running" == false ]]; then
            # give some extra time
            sleep 5
            echo "All clients shut down gracefully"
            exit 130
        fi

        sleep 1
    done

    echo "Some clients didn't shut down gracefully"
    
    # Step 3: Force kill any remaining processes
    for ((i=0; i<${#CLIENT_PIDS[@]}; i++)); do
        pid=${CLIENT_PIDS[$i]}
        name=${CLIENT_NAMES[$i]}
        if kill -0 "$pid" &>/dev/null; then
            echo "Force stopping $name (PID $pid)..."
            kill -9 "$pid" &>/dev/null || true
        fi
    done
    
    echo "Some clients were force-stopped, you may need to clean database manually."
    exit 1
}

# Source common multi-client environment setup
source multi-client-env.sh

# Debug: Let's add some verbose trap debugging
trap cleanup INT TERM EXIT

# Run the clients normally - we'll handle the signal race differently
echo "Starting clients..." >&2
for ((ID=1; ID<=NUM_CLIENTS; ID++)); do
  # Set environment variables for this client in a subshell
  (
    set_multi_client_env "$ID" "$BASE_STORAGE_PATH"
    if [[ -n "$FEATURES" ]]; then
      ./run-client.sh --features "$FEATURES" --config ./config/multi-client-template --logger log4rs.stdout.yaml
    else
      ./run-client.sh --config ./config/multi-client-template --logger log4rs.stdout.yaml
    fi
  ) &
  CLIENT_PIDS+=($!)
  CLIENT_NAMES+=("client-$ID")
  echo "Started client-$ID with PID ${CLIENT_PIDS[-1]}" >&2
done

echo "All clients started"
echo "Press Ctrl+C to shut down cleanly."

# Monitor: Use a custom monitoring loop that actively checks for signals
echo "Waiting for clients to finish (or Ctrl+C to stop)..." >&2

# Instead of wait, use a loop that can be more easily interrupted
while true; do
  # Check if any client has finished
  all_running=true
  for pid in "${CLIENT_PIDS[@]}"; do
    if ! kill -0 "$pid" &>/dev/null; then
      echo "Client with PID $pid has finished" >&2
      all_running=false
      break
    fi
  done
  
  if [[ "$all_running" == false ]]; then
    echo "At least one client finished unexpectedly, stopping all..." >&2
    cleanup
  fi
  
  # Use a very short sleep so signals can be processed quickly, before run-client.sh does
  sleep 0.05
done