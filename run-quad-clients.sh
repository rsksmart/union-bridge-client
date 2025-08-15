#!/bin/bash

# Script to run four clients with separate log outputs
# Usage: ./run-quad-clients.sh [features]
# Example: ./run-quad-clients.sh anvil
set -euo pipefail

# Handle features argument like run-client.sh
FEATURES=""
if [[ -n "${1:-}" ]]; then
    FEATURES="--features $1"
fi

echo "Setting up quad client configuration..."
if [[ -n "$FEATURES" ]]; then
    echo "Using features: $FEATURES"
fi

# Create log directories
mkdir -p logs/client1 logs/client2 logs/client3 logs/client4

make_log_config() {
  local logger_file=$1
  local crate_name=$2
  local client_name=$3

    cat > "$logger_file" << EOF
refresh_rate: 30 seconds

appenders:
  rolling_file:
    kind: rolling_file
    path: "logs/$client_name/$crate_name.log"
    encoder:
      pattern: "{d(%Y-%m-%d %H:%M:%S%.3f)} - {l} - {m}{n}"
    policy:
      trigger:
        kind: size
        limit: 10mb
      roller:
        kind: fixed_window
        base: 1
        count: 5
        pattern: "logs/$client_name/$crate_name.{}.log"

root:
  level: debug
  appenders:
    - rolling_file

loggers:
  tarpc:
    level: warn
  alloy_provider:
    level: warn
  alloy_rpc-client:
    level: warn
EOF
}

# Function to run a client with custom logger
run_client_with_logger() {
    local client_name=$1
    local config_name=$2
    local logger_file=$3

    echo "Starting $client_name using config '$config_name' with logger '$logger_file'"
    
    # Run services with custom logger
    echo "Starting services for $client_name..."

    local crate_name="block-indexer"
    local logger_file_crate="$logger_file-$crate_name.yaml"
    make_log_config "$logger_file_crate" "$crate_name" "$client_name"
    cargo run --bin "$crate_name" $FEATURES -- --logger-path "$logger_file_crate" --config-path "./config/$config_name" &
    sleep 2

    crate_name="log-indexer"
    logger_file_crate="$logger_file-$crate_name.yaml"
    make_log_config "$logger_file_crate" "$crate_name" "$client_name"
    cargo run --bin "$crate_name" $FEATURES -- --logger-path "$logger_file_crate" --config-path "./config/$config_name" &
    sleep 2

    crate_name="transaction-dispatcher"
    logger_file_crate="$logger_file-$crate_name.yaml"
    make_log_config "$logger_file_crate" "$crate_name" "$client_name"
    cargo run --bin "$crate_name" $FEATURES -- --logger-path "$logger_file_crate" --config-path "./config/$config_name" &
    sleep 2

    crate_name="user-api"
    logger_file_crate="$logger_file-$crate_name.yaml"
    make_log_config "$logger_file_crate" "$crate_name" "$client_name"
    cargo run --bin "$crate_name" $FEATURES -- --logger-path "$logger_file_crate" --config-path "./config/$config_name" &
    sleep 2

    crate_name="coordinator"
    logger_file_crate="$logger_file-$crate_name.yaml"
    make_log_config "$logger_file_crate" "$crate_name" "$client_name"
    cargo run --bin "$crate_name" $FEATURES -- --logger-path "$logger_file_crate" --config-path "./config/$config_name" &
    sleep 2

    echo "$client_name services started with logs in logs/$client_name/"
}

# Trap to cleanup
trap 'echo "Cleaning up..."; pkill -f "target/debug/" 2>/dev/null || true; rm -f /tmp/client*.log4rs.yaml; exit' INT TERM EXIT

# Define the four configurations
CONFIGS=("local" "local2" "local3" "local4")
CLIENT_NAMES=("client1" "client2" "client3" "client4")

# Validate all configs exist
for config in "${CONFIGS[@]}"; do
    if [[ ! -d "config/$config" ]]; then
        echo "Error: config/$config directory not found"
        exit 1
    fi
done

echo "Starting four clients:"
echo "  Client 1: config/local -> logs/client1/"
echo "  Client 2: config/local2 -> logs/client2/"
echo "  Client 3: config/local3 -> logs/client3/"
echo "  Client 4: config/local4 -> logs/client4/"
if [[ -n "$FEATURES" ]]; then
    echo "  Using features: $FEATURES"
fi
echo ""

# Run all four clients
for i in "${!CONFIGS[@]}"; do
    config=${CONFIGS[$i]}
    client=${CLIENT_NAMES[$i]}
    
    echo "=== Starting $client ==="
    run_client_with_logger "$client" "$config" "/tmp/$client.log4rs"
    
    if [[ $i -lt $((${#CONFIGS[@]} - 1)) ]]; then
        echo "Waiting 2 seconds before starting next client..."
        sleep 2
    fi
    echo ""
done

echo "All four clients are running!"
echo "Client 1 logs: logs/client1/ (config: local)"
echo "Client 2 logs: logs/client2/ (config: local2)"
echo "Client 3 logs: logs/client3/ (config: local3)"
echo "Client 4 logs: logs/client4/ (config: local4)"
echo ""
echo "Press Ctrl+C to stop all clients."

# Wait indefinitely
while true; do
    sleep 1
done 
