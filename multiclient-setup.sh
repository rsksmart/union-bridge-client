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

echo "[key-setup] Setup complete! All keystores have been created."