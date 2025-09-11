#!/bin/bash

set -euo pipefail

# Source multiclient environment configuration
if [ -f "multiclient.env" ]; then
    set -a  # automatically export all variables
    source multiclient.env
    set +a  # disable automatic export
else
    echo "Error: multiclient.env file not found. Please ensure it exists in the current directory."
    exit 1
fi

# Default values
NUM_CLIENTS=""
CLIENT_ID=""
FEATURES=""

# Function to show help
show_help() {
    cat << EOF
Usage: $0 [OPTIONS]

Run Union Bridge clients. Mode is automatically determined by the options provided.

OPTIONS:
    -n, --num-clients NUM         Number of clients to run simultaneously (1-10)
    -i, --id CLIENT_ID            Run a single client with the specified ID (1-10)
    -f, --features FEATURES       Optional features flag for clients
    -h, --help                    Show this help message

MODES (automatically determined):
    Multiple clients: When --num-clients is specified
    Single client:    When --id is specified

ENVIRONMENT VARIABLES:
    BASE_STORAGE_PATH             Required. Base path for client data storage

EXAMPLES:
    # Run multiple clients
    $0 --num-clients 4                          # Run 4 clients
    $0 -n 6 --features my-feature               # Run 6 clients with features
    
    # Run single client
    $0 --id 1                                   # Run client 1
    $0 -i 2 --features my-feature               # Run client 2 with features
    
    # With environment variable:
    BASE_STORAGE_PATH=/Users/username $0 --num-clients 4
    BASE_STORAGE_PATH=/Users/username $0 --id 1 --features my-feature

EOF
}

# Parse command line arguments
while [[ $# -gt 0 ]]; do
    case $1 in
        -n|--num-clients)
            NUM_CLIENTS="$2"
            shift 2
            ;;
        -i|--id)
            CLIENT_ID="$2"
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

# Determine mode based on arguments
if [[ -n "$NUM_CLIENTS" && -n "$CLIENT_ID" ]]; then
    echo "Error: Cannot specify both --num-clients and --id at the same time."
    echo "Use --help for usage information."
    exit 1
elif [[ -n "$NUM_CLIENTS" ]]; then
    MODE="all"
elif [[ -n "$CLIENT_ID" ]]; then
    MODE="single"
else
    show_help
    exit 0
fi

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

# Function to set environment variables for multi-client setup
set_multi_client_env() {
  local ID=$1
  local BASE_STORAGE_PATH=$2
  
  if [[ -z "${ID:-}" || ! "$ID" =~ ^([1-9]|10)$ ]]; then
    echo "Error: set_multi_client_env requires CLIENT_ID to be one of [1-10]" >&2
    return 1
  fi

  if [[ -z "${BASE_STORAGE_PATH:-}" ]]; then
    echo "Error: set_multi_client_env requires BASE_STORAGE_PATH argument" >&2
    return 1
  fi
  
  # Use indirect variable expansion to get values from multiclient.env
  local var_name
  var_name="BLOCK_NOTIFIER_BROKER_PORT_$ID" && export UB__BLOCK_NOTIFIER__BROKER_PORT="${!var_name}"
  var_name="LOG_NOTIFIER_BROKER_PORT_$ID" && export UB__LOG_NOTIFIER__BROKER_PORT="${!var_name}"
  var_name="BLOCK_BROKER_PORT_$ID" && export UB__BLOCK_BROKER__PORT="${!var_name}"
  var_name="LOG_BROKER_PORT_$ID" && export UB__LOG_BROKER__PORT="${!var_name}"
  var_name="USER_BROKER_PORT_$ID" && export UB__USER_BROKER__PORT="${!var_name}"
  var_name="BROKER_CLIENT_ID_$ID" && export UB__BROKER_CLIENT_ID="${!var_name}"
  var_name="INDEXER_STORAGE_PATH_$ID" && export UB__INDEXER__STORAGE__PATH="${BASE_STORAGE_PATH}/.union_bridge/database/${!var_name}"
  var_name="KEY_STORE_PATH_$ID" && export UB__KEY_STORE__PATH="${BASE_STORAGE_PATH}/.union_bridge/keystore/${!var_name}"
  var_name="SERVER_URL_$ID" && export UB__SERVER__URL="${!var_name}"
  var_name="COORDINATOR_BROKER_CLIENT_ID_$ID" && export UB__COORDINATOR_BROKER_CLIENT_ID="${!var_name}"
  var_name="BROKER_SERVER_PORT_$ID" && export UB__BROKER_SERVER_PORT="${!var_name}"
  var_name="HTTP_SERVER_PORT_$ID" && export UB__HTTP_SERVER_PORT="${!var_name}"
  var_name="BITVMX_BROKER_PORT_$ID" && export UB__BITVMX_BROKER__PORT="${!var_name}"
  
  export CLIENT_ID=$ID
}

# Mode-specific logic
case "$MODE" in
    "all")
        # Validate NUM_CLIENTS parameter for all mode
        if [[ -z "$NUM_CLIENTS" ]]; then
            echo "Error: --num-clients is required for --all mode."
            echo "Use --help for usage information."
            exit 1
        fi

        if [[ ! "$NUM_CLIENTS" =~ ^([1-9]|10)$ ]]; then
            echo "Error: NUM_CLIENTS must be between 1 and 10."
            echo "Use --help for usage information."
            exit 1
        fi

        readonly NUM_CLIENTS

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

        # Set up trap for cleanup
        trap cleanup INT TERM EXIT

        # Run the clients
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

        # Monitor clients
        echo "Waiting for clients to finish (or Ctrl+C to stop)..." >&2

        # Use a loop that can be easily interrupted
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
          
          # Use a very short sleep so signals can be processed quickly
          sleep 0.05
        done
        ;;

    "single")
        # Validate CLIENT_ID for single mode
        if [[ ! "$CLIENT_ID" =~ ^([1-9]|10)$ ]]; then
            echo "Error: CLIENT_ID must be between 1 and 10."
            echo "Usage: $0 --id <CLIENT_ID> [--features FEATURES]"
            exit 1
        fi

        # Set environment variables for this client
        set_multi_client_env "$CLIENT_ID" "$BASE_STORAGE_PATH"

        # Run the client with the provided CLIENT_ID and optional FEATURES
        if [[ -n "$FEATURES" ]]; then
          ./run-client.sh --features "$FEATURES" --config ./config/multi-client-template --logger log4rs.stdout.yaml
        else
          ./run-client.sh --config ./config/multi-client-template --logger log4rs.stdout.yaml
        fi
        ;;

    *)
        echo "Error: Invalid mode '$MODE'"
        echo "Use --help for usage information."
        exit 1
        ;;
esac
