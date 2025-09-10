#!/bin/bash

set -meuo pipefail

# Validate FEATURES if provided
FEATURES=${1:-}
if [[ -n "$FEATURES" && ! "$FEATURES" =~ ^[a-zA-Z0-9_-]+$ ]]; then
  echo "Error: FEATURES must be a single word containing only letters, numbers, underscores, or hyphens."
  echo "Usage: $0 <CLIENT_ID> [FEATURES]"
  exit 1
fi

CLIENT_PIDS=()
CLIENT_NAMES=()

function cleanup() {
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
            echo "All clients shut down gracefully in ${i}s!"
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

# Debug: Let's add some verbose trap debugging
trap cleanup INT TERM EXIT

# Run the clients normally - we'll handle the signal race differently
echo "Starting clients..." >&2
for ID in {1..4}; do
  if [[ -n "$FEATURES" ]]; then
    CLIENT_ID=$ID ./run-client.sh --features "$FEATURES" --config ./config/multi-client/$ID --logger log4rs.stdout.yaml &
  else
    CLIENT_ID=$ID ./run-client.sh --config ./config/multi-client/$ID --logger log4rs.stdout.yaml &
  fi
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
    exit 1
  fi
  
  # Use a very short sleep so signals can be processed quickly, before run-client.sh does
  sleep 0.05
done