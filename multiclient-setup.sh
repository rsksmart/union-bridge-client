#!/usr/bin/env bash

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
CREATE_WALLETS=false
FUND_WALLETS=false
SETUP_COMMITTEE=false
NUM_CLIENTS=""
STREAM_ID=""

# Constants
ANVIL_ADDRESS="0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266"

# Function to show help
show_help() {
    cat << EOF
Usage: $0 [OPTIONS]

Set up multiclient wallets. Mode is determined by the options provided.

OPTIONS:
    -w, --create-wallets          Create wallets (keystores) - only needed first time or to recreate
    -f, --fund-wallets            Fund existing wallets
    -c, --committee-setup NUM     Setup committee by applying to stream for NUM clients (1-10)
    --stream_id ID                REQUIRED: Set the stream ID for committee setup (when using --committee-setup)
    -h, --help                    Show this help message

ENVIRONMENT VARIABLES:
    KEY_STORE_PASSWORD            Required. Password for keystore encryption
    BASE_STORAGE_PATH             Required. Base path for client data storage

EXAMPLES:
    # Create wallets only
    $0 --create-wallets

    # Fund existing wallets only
    $0 --fund-wallets

    # Create and fund wallets
    $0 --create-wallets --fund-wallets

    # Setup committee for existing clients (stream_id is required)
    $0 --committee-setup 4 --stream_id 1

    # Setup committee for existing clients with custom stream ID
    $0 --committee-setup 4 --stream_id 5

    # With environment variables:
    KEY_STORE_PASSWORD=mypass BASE_STORAGE_PATH=/Users/username $0 --create-wallets --fund-wallets

EOF
}

# Parse command line arguments
while [[ $# -gt 0 ]]; do
    case $1 in
        -w|--create-wallets)
            CREATE_WALLETS=true
            shift
            ;;
        -f|--fund-wallets)
            FUND_WALLETS=true
            shift
            ;;
        -c|--committee-setup)
            SETUP_COMMITTEE=true
            NUM_CLIENTS="$2"
            shift 2
            ;;
        --stream_id)
            STREAM_ID="$2"
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

# Show help if no mode selected
if [[ "$CREATE_WALLETS" == false && "$FUND_WALLETS" == false && "$SETUP_COMMITTEE" == false ]]; then
    show_help
    exit 0
fi

# Validate required environment variables
if [ -z "${KEY_STORE_PASSWORD:-}" ]; then
    echo "Error: KEY_STORE_PASSWORD environment variable is required but not set."
    exit 1
fi

if [ -z "${BASE_STORAGE_PATH:-}" ]; then
    echo "Error: BASE_STORAGE_PATH environment variable is required but not set."
    echo ""
    echo "Please set the BASE_STORAGE_PATH environment variable before running this script:"
    echo "  export BASE_STORAGE_PATH=/Users/username"
    echo "  $0"
    echo ""
    echo "Example:"
    echo "  export BASE_STORAGE_PATH=/Users/username"
    echo "  $0 --both"
    exit 1
fi

KEYSTORE_BASE_PATH="${BASE_STORAGE_PATH}/.union_bridge/keystore"

# Wallet creation functions
create_or_use_keystore() {
    local keystore_path="$1"
    local file_name="$2"
    local full_keystore_path="${keystore_path}/${file_name}"

    if [ ! -f "${full_keystore_path}" ]; then
        echo "[wallet-setup] Creating new key with key-manager at ${keystore_path}..."
        if ! output=$(cargo run --manifest-path key-manager/Cargo.toml -- new-key -p "${KEY_STORE_PASSWORD}" -d "${keystore_path}" 2>&1); then
            echo "[wallet-setup] Error: Failed to generate key: ${output}"
            exit 1
        fi

        key_path=$(echo "${output}" | sed -n 's/.*Generated key @ \([^,]*\),.*/\1/p')
        [ -z "${key_path}" ] && echo "[wallet-setup] Error: Could not extract key path" && exit 1

        mv "${key_path}" "${full_keystore_path}"
    else
        echo "[wallet-setup] Key already exists at ${full_keystore_path}, skipping key generation."
    fi
}

# Wallet funding functions
derive_address_from_keystore() {
    local keystore_file="$1"
    
    if [ ! -f "$keystore_file" ]; then
        echo "[fund-operators] Warning: Keystore file not found: $keystore_file"
        return 1
    fi
    
    # Run key-manager to get the address
    local output
    if output=$(cargo run --manifest-path key-manager/Cargo.toml -- derive-public-data -p "${KEY_STORE_PASSWORD}" -k "$keystore_file" 2>&1); then
        # Extract address from output using sed
        local address
        address=$(echo "$output" | sed -n "s/.*address '\\([^']*\\)'.*/\\1/p")
        
        if [ -n "$address" ]; then
            # Add 0x prefix if not present
            if [[ ! "$address" =~ ^0x ]]; then
                address="0x$address"
            fi
            echo "$address"
            return 0
        else
            echo "[fund-operators] Warning: Could not extract address from keystore: $keystore_file"
            echo "[fund-operators] Key-manager output was: $output"
            return 1
        fi
    else
        echo "[fund-operators] Error deriving address from keystore: $keystore_file"
        echo "[fund-operators] Key-manager error: $output"
        return 1
    fi
}

fund_operator() {
    local operator_id="$1"
    local address="$2"
    
    echo "[fund-operators] Funding operator $operator_id at address $address"
    
    local cast_output
    if cast_output=$(cast send --unlocked --from "$ANVIL_ADDRESS" "$address" --value 1000000000000000000 --rpc-url http://127.0.0.1:8545 2>&1); then
        echo "[fund-operators] Successfully funded operator $operator_id"
        echo "$cast_output"
        sleep 0.1
        return 0
    else
        echo "[fund-operators] Error: Failed to fund operator $operator_id at address $address"
        echo "[fund-operators] Cast error: $cast_output"
        return 1
    fi
}

# Committee setup functions
setup_committee() {
    local num_clients=$1
    
    if [[ -z "$num_clients" ]]; then
        echo "Error: setup_committee requires number of clients as parameter"
        return 1
    fi
    
    echo "[setup-committee] Starting committee setup for $num_clients clients..."
    
    local success_count=0
    local failed_count=0
    
    for ((i=1; i<=num_clients; i++)); do
        # Get the HTTP_SERVER_PORT for this client using indirect variable expansion
        local var_name="HTTP_SERVER_PORT_$i"
        local port="${!var_name}"
        
        if [ -z "$port" ]; then
            echo "[setup-committee] Warning: No HTTP_SERVER_PORT found for client $i, skipping"
            continue
        fi
        
        # Determine role: odd clients are Provers, even clients are Verifiers
        local role
        if (( i % 2 == 1 )); then
            role="Prover"
        else
            role="Verifier"
        fi
        
        echo "[setup-committee] Setting up client $i on port $port as $role..."
        
        local curl_output
        local json_payload='{"ApplyToStream":{"stream_id":'$STREAM_ID',"role":"'$role'","funding_utxo":{"value":10000000},"speed_up_utxo":{"value":10000000}}}'
        
        echo "[setup-committee] Sending request: $json_payload"
        
        if curl_output=$(curl -s --connect-timeout 5 --max-time 10 -X POST "http://localhost:$port/member/apply-stream" \
            -H "Content-Type: application/json" \
            -d "$json_payload" 2>&1); then
            echo "[setup-committee] Successfully setup client $i. Response: $curl_output"
            success_count=$((success_count + 1))
        else
            if [[ -z "$curl_output" ]]; then
                echo "[setup-committee] Error: Failed to setup client $i. Connection timeout - client may not be running on port $port"
            else
                echo "[setup-committee] Error: Failed to setup client $i. Curl error: $curl_output"
            fi
            failed_count=$((failed_count + 1))
        fi
        
        echo "[setup-committee] ---"
        sleep 0.5  # Small delay between requests
    done
    
    echo "[setup-committee] Committee setup complete! Successfully setup: $success_count clients"
    if [ "$failed_count" -gt 0 ]; then
        echo "[setup-committee] Failed to setup: $failed_count clients"
        return 1
    else
        echo "[setup-committee] All clients applied successfully!"
        return 0
    fi
}

# Execute wallet creation if requested
if [[ "$CREATE_WALLETS" == true ]]; then
    echo "[wallet-setup] Starting wallet creation..."
    
    for i in {1..10}; do
        create_or_use_keystore "$KEYSTORE_BASE_PATH" "multi-client-$i"
    done
    
    echo "[wallet-setup] Wallet creation complete! All keystores have been created."
fi

# Execute wallet funding if requested
if [[ "$FUND_WALLETS" == true ]]; then
    echo "[fund-operators] Starting to fund operators..."
    echo "[fund-operators] Using keystores from: $KEYSTORE_BASE_PATH"

    # Check if keystore directory exists
    if [ ! -d "$KEYSTORE_BASE_PATH" ]; then
        echo "[fund-operators] Error: Keystore directory not found: $KEYSTORE_BASE_PATH"
        if [[ "$CREATE_WALLETS" == false ]]; then
            echo "[fund-operators] Make sure you've created wallets first (use --create or --both)"
        fi
        exit 1
    fi

    # Find all multi-client keystore files and fund them
    funded_count=0
    failed_count=0

    # Find all multi-client-* files in the keystore directory
    keystore_files=($(find "$KEYSTORE_BASE_PATH" -name "multi-client-*" -type f | sort -V))

    if [ ${#keystore_files[@]} -eq 0 ]; then
        echo "[fund-operators] No multi-client keystore files found in $KEYSTORE_BASE_PATH"
        if [[ "$CREATE_WALLETS" == false ]]; then
            echo "[fund-operators] Make sure you've created wallets first (use --create or --both)"
        fi
        exit 1
    fi

    echo "[fund-operators] Found ${#keystore_files[@]} keystore files to process"

    for keystore_file in "${keystore_files[@]}"; do
        # Extract operator number from filename
        operator_id=$(basename "$keystore_file" | sed 's/multi-client-//')
        
        # Try to derive address
        if address=$(derive_address_from_keystore "$keystore_file" 2>/dev/null); then    
            # Try to fund operator
            if fund_operator "$operator_id" "$address" 2>/dev/null; then
                funded_count=$((funded_count + 1))
                echo "[fund-operators] Operator $operator_id funded successfully"
            else
                failed_count=$((failed_count + 1))
                echo "[fund-operators] Failed to fund operator $operator_id"
            fi
        else
            failed_count=$((failed_count + 1))
            echo "[fund-operators] Failed to derive address for operator $operator_id"
        fi
        
        echo "[fund-operators] ---"
    done

    echo "[fund-operators] Funding complete!"
    echo "[fund-operators] Successfully funded: $funded_count operators"
    if [ "$failed_count" -gt 0 ]; then
        echo "[fund-operators] Failed to fund: $failed_count operators"
        exit 1
    else
        echo "[fund-operators] All operators funded successfully!"
    fi
fi

# Execute committee setup if requested
if [[ "$SETUP_COMMITTEE" == true ]]; then
    # Validate NUM_CLIENTS parameter
    if [[ -z "$NUM_CLIENTS" ]]; then
        echo "Error: --committee-setup requires a number argument"
        echo "Use --help for usage information."
        exit 1
    fi

    if [[ ! "$NUM_CLIENTS" =~ ^([1-9]|10)$ ]]; then
        echo "Error: Number of clients must be between 1 and 10."
        echo "Use --help for usage information."
        exit 1
    fi

    # Validate STREAM_ID is provided and is a positive integer
    if [[ -z "$STREAM_ID" ]]; then
        echo "Error: --stream_id is required when using --committee-setup."
        echo "Use --help for usage information."
        exit 1
    fi
    if [[ ! "$STREAM_ID" =~ ^[0-9]+$ ]]; then
        echo "Error: Stream ID must be a positive integer."
        echo "Use --help for usage information."
        exit 1
    fi

    if ! setup_committee "$NUM_CLIENTS"; then
        exit 1
    fi
fi

# Final summary
echo ""
echo "=== Multiclient Setup Summary ==="
if [[ "$CREATE_WALLETS" == true ]]; then
    echo "Wallet creation completed"
fi
if [[ "$FUND_WALLETS" == true ]]; then
    echo "Wallet funding completed"
fi
if [[ "$SETUP_COMMITTEE" == true ]]; then
    echo "Committee application completed"
fi
