#!/bin/bash

set -euo pipefail

# Constants
ANVIL_ADDRESS="0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266"
KEYSTORE_BASE_PATH="${BASE_STORAGE_PATH}/.union_bridge/keystore"

# Validate required environment variables
if [ -z "${KEY_STORE_PASSWORD:-}" ]; then
  echo "[fund-operators] Error: KEY_STORE_PASSWORD environment variable is required but not set."
  exit 1
fi

if [ -z "${BASE_STORAGE_PATH:-}" ]; then
  echo "[fund-operators] Error: BASE_STORAGE_PATH environment variable is required but not set."
  echo ""
  echo "Please set the BASE_STORAGE_PATH environment variable before running this script:"
  echo "  export BASE_STORAGE_PATH=/Users/username"
  echo "  $0"
  echo ""
  echo "Example:"
  echo "  export BASE_STORAGE_PATH=/Users/username"
  echo "  $0"
  exit 1
fi

derive_address_from_keystore() {
  local keystore_file="$1"
  
  if [ ! -f "$keystore_file" ]; then
    echo "[fund-operators] Warning: Keystore file not found: $keystore_file" >&2
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
      echo "[fund-operators] Warning: Could not extract address from keystore: $keystore_file" >&2
      echo "[fund-operators] Key-manager output was: $output" >&2
      return 1
    fi
  else
    echo "[fund-operators] Error deriving address from keystore: $keystore_file" >&2
    echo "[fund-operators] Key-manager error: $output" >&2
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
    echo "[fund-operators] Error: Failed to fund operator $operator_id at address $address" >&2
    echo "[fund-operators] Cast error: $cast_output" >&2
    return 1
  fi
}

echo "[fund-operators] Starting to fund operators..."
echo "[fund-operators] Using keystores from: $KEYSTORE_BASE_PATH"

# Find all multi-client keystore files and fund them
funded_count=0
failed_count=0

if [ ! -d "$KEYSTORE_BASE_PATH" ]; then
  echo "[fund-operators] Error: Keystore directory not found: $KEYSTORE_BASE_PATH"
  echo "[fund-operators] Make sure you've run multiclient-setup.sh first"
  exit 1
fi

# Find all multi-client-* files in the keystore directory
keystore_files=($(find "$KEYSTORE_BASE_PATH" -name "multi-client-*" -type f | sort -V))

if [ ${#keystore_files[@]} -eq 0 ]; then
  echo "[fund-operators] No multi-client keystore files found in $KEYSTORE_BASE_PATH"
  echo "[fund-operators] Make sure you've run multiclient-setup.sh first"
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
