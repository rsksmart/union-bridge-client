#!/usr/bin/env bash

set -euo pipefail

# Load multiclient environment configuration if available
if [ -f "multiclient.env" ]; then
    set -a  # automatically export all variables
    source multiclient.env
    set +a  # disable automatic export
fi

# Default values and constants
readonly DEFAULT_CONFIG="multi-client-template"
readonly DEFAULT_LOGGER="log4rs.stdout.yaml"

NUM_CLIENTS=""
CLIENT_ID=""
FEATURES=""
CONFIG_PARAM="--config-path ./config/$DEFAULT_CONFIG"
LOGGER_PARAM="--logger-path $DEFAULT_LOGGER"

# Function to validate BASE_STORAGE_PATH
validate_base_storage_path() {
    if [[ -z "${BASE_STORAGE_PATH:-}" ]]; then
        echo "Error: BASE_STORAGE_PATH environment variable is required."
        echo ""
        echo "Please set the BASE_STORAGE_PATH environment variable before running this script:"
        echo "  export BASE_STORAGE_PATH=/Users/username"
        echo ""
        echo "Use --help for more information and examples."
        exit 1
    fi
}

# Helper function to parse arguments that require a value
parse_arg_value() {
    local arg_name=$1
    local var_name=$2
    local prefix=$3
    local value=$4

    if [[ -n "$value" ]]; then
        eval "$var_name=\"$prefix$value\""
        return 0
    else
        echo "Error: $arg_name requires a value"
        echo "Use --help for usage information."
        exit 1
    fi
}

# Helper function to validate numeric range (1-10)
validate_range_1_10() {
    local value=$1
    local var_name=$2

    if [[ ! "$value" =~ ^([1-9]|10)$ ]]; then
        echo "Error: $var_name must be between 1 and 10."
        if [[ "$var_name" == "CLIENT_ID" ]]; then
            echo "Current CLIENT_ID: $value"
        fi
        echo "Use --help for usage information."
        exit 1
    fi
}

# Function to show help
show_help() {
    cat << EOF
Usage: $0 [OPTIONS]

Run Union Bridge clients. Mode is automatically determined by the options provided.

OPTIONS:
    -n, --num-clients NUM         Number of clients to run simultaneously (1-10)
    -i, --id CLIENT_ID            Run a single client with the specified ID (1-10)
    -f, --features FEATURES       Optional features flag for clients
    -c, --config CONFIG_NAME      Optional config directory name under ./config/. Defaults to 'multi-client-template'
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
            parse_arg_value "features" "FEATURES" "--features " "$2"
            shift 2
            ;;
        -c|--config)
            parse_arg_value "config" "CONFIG_PARAM" "--config-path " "$2"
            shift 2
            ;;
        -l|--logger)
            parse_arg_value "logger" "LOGGER_PARAM" "--logger-path " "$2"
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
fi

MODE=$([[ -n "$NUM_CLIENTS" ]] && echo "multi" || echo "single")

# Set default CLIENT_ID for single mode
if [[ "$MODE" == "single" && -z "${CLIENT_ID:-}" ]]; then
    export CLIENT_ID="1"
fi

# Generic cleanup function for both single and multi-client modes
generic_cleanup() {
    local pids_array_name=$1
    local names_array_name=$2
    local process_type=$3

    # Get the arrays by name
    local pids=()
    local names=()
    eval "pids=(\"\${${pids_array_name}[@]}\")"
    eval "names=(\"\${${names_array_name}[@]}\")"

    if $CLEANING_UP; then
        return
    fi

    CLEANING_UP=true

    echo "Shutting down ${#pids[@]} $process_type..."

    # Send SIGTERM to all processes in reverse order to allow graceful shutdown
    for ((i=${#pids[@]}-1; i>=0; i--)); do
        pid=${pids[$i]}
        name=${names[$i]}
        if kill -0 "$pid" &>/dev/null; then
            echo "Stopping $name (PID $pid)..."
            kill -TERM "$pid" &>/dev/null || true
        fi
    done

    # Wait for processes to shut down gracefully
    for i in {1..10}; do
        all_stopped=true
        for pid in "${pids[@]}"; do
            if kill -0 "$pid" &>/dev/null; then
                all_stopped=false
                break
            fi
        done
        if [[ "$all_stopped" == true ]]; then
            echo "All $process_type shut down gracefully"
            return
        fi
        sleep 1
    done

    # If we get here, processes didn't shut down gracefully within 10 seconds

    # Force kill any remaining processes
    echo "Force stopping remaining $process_type..."
    for ((i=${#pids[@]}-1; i>=0; i--)); do
        pid=${pids[$i]}
        name=${names[$i]}
        if kill -0 "$pid" &>/dev/null; then
            echo "Force stopping $name (PID $pid)..."
            kill -9 "$pid" &>/dev/null || true
        fi
    done

    # Clean up any remaining child processes
    pkill -9 -P $$ &>/dev/null || true
    echo "Some $process_type were force-stopped, you may need to clean database manually."
}

# Function to set environment variables for multi-client setup
set_multi_client_env() {
    local ID=$1
    local BASE_STORAGE_PATH=$2
    
    if [[ -z "${ID:-}" ]]; then
        echo "Error: set_multi_client_env requires CLIENT_ID to be provided" >&2
        return 1
    fi

    # Validate CLIENT_ID range
    if [[ ! "$ID" =~ ^([1-9]|10)$ ]]; then
        echo "Error: CLIENT_ID must be between 1 and 10." >&2
        echo "Current CLIENT_ID: $ID" >&2
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
    SERVICES=("block-indexer" "log-indexer" "user-api" "coordinator")

    function cleanup() {
        generic_cleanup "SERVICE_PIDS" "SERVICE_NAMES" "services"
    }

    # Set up trap to handle signals and clean up
    trap cleanup INT TERM

    function run_service() {
        local svc=$1

        cargo_run_cmd="cargo run --bin $svc $FEATURES -- $LOGGER_PARAM $CONFIG_PARAM"
        echo "Starting $svc: $cargo_run_cmd"
        $cargo_run_cmd &

        pid=$!
        SERVICE_PIDS+=("$pid")
        SERVICE_NAMES+=("$svc")
        echo "$svc started (PID $pid)"

        # Quick check: did it exit immediately?
        sleep 1
        if ! kill -0 "$pid" &>/dev/null; then
            echo "ERROR: $svc failed to start"
            exit 1
        fi
    }

    # Start services in the background
    echo "Starting services..."
    for svc in "${SERVICES[@]}"; do
        # Coordinator depends on the others, so we wait a bit before starting it
        if [[ "$svc" == "coordinator" ]]; then
            sleep 2
        fi
        run_service "$svc"
    done

    # Give services a moment to stabilize
    sleep 2

    echo
    echo "All services launched. Press Ctrl+C to shut down cleanly."
    echo "Services are running in the background."

    # Wait for services to finish or signal
    wait
}

# Function to run multiple clients
run_multi_client_mode() {
    # Validate BASE_STORAGE_PATH
    validate_base_storage_path

    # Validate NUM_CLIENTS
    validate_range_1_10 "$NUM_CLIENTS" "NUM_CLIENTS"

    readonly NUM_CLIENTS

    CLIENT_PIDS=()
    CLIENT_NAMES=()
    CLEANING_UP=false

    function cleanup() {
        generic_cleanup "CLIENT_PIDS" "CLIENT_NAMES" "clients"
    }

    # Set up trap to handle signals and clean up
    trap cleanup INT TERM

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
    echo "Clients are running in the background."

    # Wait for clients to finish or signal
    wait
}

# Function to run single client
run_single_client_mode() {
    # Validate BASE_STORAGE_PATH
    validate_base_storage_path

    # Validate CLIENT_ID
    validate_range_1_10 "$CLIENT_ID" "CLIENT_ID"

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