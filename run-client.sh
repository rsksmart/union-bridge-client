#!/usr/bin/env bash

set -euo pipefail

# Load multiclient environment configuration if available
if [ -f "multiclient.env" ]; then
    set -a  # automatically export all variables
    source multiclient.env
    set +a  # disable automatic export
fi

# Default values
NUM_CLIENTS=""
CLIENT_ID=""
FEATURES=""
CONFIG_PARAM="--config-path ./config/multi-client-template"     # Default config: multi-client-template
LOGGER_PARAM="--logger-path log4rs.stdout.yaml" # Default logger: stdout

# Function to show help
show_help() {
    cat << EOF
Usage: $0 [OPTIONS]

Run Union Bridge clients. Mode is automatically determined by the options provided.

OPTIONS:
    -n, --num-clients NUM         Number of clients to run simultaneously (1-10)
    -i, --id CLIENT_ID            Run a single client with the specified ID (1-10)
    -f, --features FEATURES       Optional features flag for clients
    -c, --config CONFIG_NAME      Optional config directory name under ./config/. Defaults to 'local'
    -l, --logger LOGGER_FILE      Optional logger configuration file path. Defaults to 'log4rs.stdout.yaml'
    -h, --help                    Show this help message

MODES (automatically determined):
    Multiple clients: When --num-clients is specified
    Single client:    When --id is specified or no mode flags are used (defaults to CLIENT_ID=1)

ENVIRONMENT VARIABLES:
    BASE_STORAGE_PATH             Required. Base path for client data storage
    CLIENT_ID                     Optional. Client ID for single client mode (can also be set via --id)

EXAMPLES:
    # Single client mode (defaults to CLIENT_ID=1)
    $0                                              # Run client 1
    $0 --features anvil                             # Run client 1 with anvil feature
    $0 --id 2 --features anvil                      # Run client 2 with anvil feature
    
    # Multi-client mode
    $0 --num-clients 4                              # Run 4 clients
    $0 --num-clients 6 --features anvil             # Run 6 clients with anvil feature
    
    # With environment variable:
    BASE_STORAGE_PATH=/Users/username $0
    BASE_STORAGE_PATH=/Users/username $0 --num-clients 4 --features anvil

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
            if [[ -n "${2:-}" ]]; then
                FEATURES="--features $2"
                shift 2
            fi
            ;;
        -c|--config)
            if [[ -n "${2:-}" ]]; then
                CONFIG_PARAM="--config-path $2"
                shift 2
            fi
            ;;
        -l|--logger)
            if [[ -n "${2:-}" ]]; then
                LOGGER_PARAM="--logger-path $2"
                shift 2
            fi
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
    MODE="multi"
else
    MODE="single"
    # Set default CLIENT_ID if not already set
    if [[ -z "${CLIENT_ID:-}" ]]; then
        export CLIENT_ID="1"
    fi
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

# Function to run services for a single client instance
run_single_client() {
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
}

# Function to run multiple clients
run_multi_client_mode() {
    # Validate BASE_STORAGE_PATH
    if [[ -z "${BASE_STORAGE_PATH:-}" ]]; then
        echo "Error: BASE_STORAGE_PATH environment variable is required for multi-client mode."
        echo ""
        echo "Please set the BASE_STORAGE_PATH environment variable before running this script:"
        echo "  export BASE_STORAGE_PATH=/Users/username"
        echo ""
        echo "Use --help for more information and examples."
        exit 1
    fi

    # Validate NUM_CLIENTS
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
        # Set environment variables for this client in a subshell and run directly
        (
            set_multi_client_env "$ID" "$BASE_STORAGE_PATH"
            run_single_client
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
}

# Function to run single client
run_single_client_mode() {
    # Validate BASE_STORAGE_PATH
    if [[ -z "${BASE_STORAGE_PATH:-}" ]]; then
        echo "Error: BASE_STORAGE_PATH environment variable is required for single client mode."
        echo ""
        echo "Please set the BASE_STORAGE_PATH environment variable before running this script:"
        echo "  export BASE_STORAGE_PATH=/Users/username"
        echo ""
        echo "Use --help for more information and examples."
        exit 1
    fi

    # Validate CLIENT_ID
    if [[ ! "$CLIENT_ID" =~ ^([1-9]|10)$ ]]; then
        echo "Error: CLIENT_ID must be between 1 and 10."
        echo "Current CLIENT_ID: $CLIENT_ID"
        exit 1
    fi

    # Set environment variables for this client
    set_multi_client_env "$CLIENT_ID" "$BASE_STORAGE_PATH"

    # Run the single client
    run_single_client
}

# Mode-specific logic
case "$MODE" in
    "multi")
        run_multi_client_mode
        ;;
    "single")
        run_single_client_mode
        ;;
    *)
        echo "Error: Invalid mode '$MODE'"
        echo "Use --help for usage information."
        exit 1
        ;;
esac