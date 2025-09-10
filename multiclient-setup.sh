#!/usr/bin/env bash
set -euo pipefail

if [ -z "${KEY_STORE_PASSWORD:-}" ]; then
  echo "[key-setup] No KEY_STORE_PASSWORD provided, exiting."
  exit 1
fi

if [ -z "${BASE_STORAGE_PATH:-}" ]; then
  echo "[key-setup] Error: BASE_STORAGE_PATH environment variable is required but not set."
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

create_or_use_keystore() {
  local keystore_path="$1"
  local file_name="$2"
  local full_keystore_path="${keystore_path}/${file_name}"

  echo "[key-setup] Creating or using keystore at ${keystore_path}"

  if [ ! -f "${full_keystore_path}" ]; then
    echo "[key-setup] Creating new key with key-manager."
    if ! output=$(cargo run --manifest-path key-manager/Cargo.toml -- new-key -p "${KEY_STORE_PASSWORD}" -d "${keystore_path}" 2>&1); then
        echo "[key-setup] Error: Failed to generate key: ${output}"
        exit 1
    fi

    key_path=$(echo "${output}" | sed -n 's/.*Generated key @ \([^,]*\),.*/\1/p')
    [ -z "${key_path}" ] && echo "[key-setup] Error: Could not extract key path" && exit 1

    mv "${key_path}" "${full_keystore_path}"
  else
    echo "[key-setup] Key already exists at ${full_keystore_path}, skipping key generation."
  fi

  echo "[key-setup] Using key at ${full_keystore_path}"
}

KEYSTORE_BASE_PATH="${BASE_STORAGE_PATH}/.union_bridge/keystore"
for i in {1..10}; do
  create_or_use_keystore "$KEYSTORE_BASE_PATH" "multi-client-$i"
done

# Generate fund_local_operators.sh with actual operator addresses
echo "[key-setup] Generating fund_local_operators.sh with actual operator addresses..."

# Copy template if fund_local_operators.sh doesn't exist
if [ ! -f "fund_local_operators.sh" ]; then
  echo "[key-setup] Copying fund_local_operators-template.sh to fund_local_operators.sh"
  cp "fund_local_operators-template.sh" "fund_local_operators.sh"
fi

# Derive addresses for first 4 operators and replace placeholders
for i in {1..4}; do
  keystore_file="${KEYSTORE_BASE_PATH}/multi-client-$i"
  
  if [ -f "$keystore_file" ]; then
    echo "[key-setup] Deriving address for operator $i from $keystore_file"
    
    # Run key-manager to get the address
    if output=$(cargo run --manifest-path key-manager/Cargo.toml -- derive-public-data -p "${KEY_STORE_PASSWORD}" -k "$keystore_file" 2>&1); then
      # Extract address from output using sed
      address=$(echo "$output" | sed -n "s/.*address '\\([^']*\\)'.*/\\1/p")
      
      if [ -n "$address" ]; then
        # Add 0x prefix if not present
        if [[ ! "$address" =~ ^0x ]]; then
          address="0x$address"
        fi
        
        echo "[key-setup] Operator $i address: $address"
        
        # Replace placeholder in fund_local_operators.sh
        sed -i.bak "s/<OPERATOR_${i}_ADDRESS>/$address/g" "fund_local_operators.sh"
      else
        echo "[key-setup] Warning: Could not extract address for operator $i"
      fi
    else
      echo "[key-setup] Error deriving address for operator $i: $output"
    fi
  else
    echo "[key-setup] Warning: Keystore file not found for operator $i: $keystore_file"
  fi
done

# Remove backup file created by sed
rm -f "fund_local_operators.sh.bak"

echo "[key-setup] fund_local_operators.sh has been updated with actual operator addresses"