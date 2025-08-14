#!/usr/bin/env bash
set -e

if [ -z "${KEY_STORE_PASSWORD:-}" ]; then
  echo "[key-setup] No KEY_STORE_PASSWORD provided, exiting."
  exit 1
fi

RPC_URL="http://actors-mocking:8545"
KEYSTORE_PATH="/keystore/key.json"

if [ ! -f "${KEYSTORE_PATH}" ]; then
  echo "[key-setup] Creating new key with key-manager."
  mkdir -p /keystore
  if ! output=$(/app/key-manager new-key -p "${KEY_STORE_PASSWORD}" -d "/keystore" 2>&1); then
      echo "[key-setup] Error: Failed to generate key: ${output}"
      exit 1
  fi

  key_path=$(echo "${output}" | sed -n 's/.*Generated key @ \([^,]*\),.*/\1/p')
  [ -z "${key_path}" ] && echo "[key-setup] Error: Could not extract key path" && exit 1

  mv "${key_path}" "${KEYSTORE_PATH}"
else
  echo "[key-setup] Key already exists at ${KEYSTORE_PATH}, skipping key generation."
fi

echo "[key-setup] Using key at ${KEYSTORE_PATH}"

# forward to command entry in docker-compose.yml
exec "$@"