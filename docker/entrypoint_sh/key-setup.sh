#!/usr/bin/env bash
set -e

if [ -z "${KEY_STORE_PASSWORD:-}" ]; then
  echo "[key-setup] No KEY_STORE_PASSWORD provided, exiting."
  exit 1
fi

if [ -z "${FUNDING_ADDRESS:-}" ]; then
  echo "[key-setup] No FUNDING_ADDRESS provided, exiting."
  exit 1
fi

RPC_URL="http://actors-mocking:2222"
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
fi

if ! pd_output=$(/app/key-manager derive-public-data -p "${KEY_STORE_PASSWORD}" -k "${KEYSTORE_PATH}" 2>&1) ; then
  echo "[key-setup] Error: Failed to derive public data from key"
  exit 1
fi

if ! address=$(echo "${pd_output}" | sed -n "s/.*address '\([^']*\)'.*/\1/p"); then
  echo "[key-setup] Error: Failed to extract address from key"
  exit 1
fi

if ! balance=$(RUST_LOG=error cast balance -e "${FUNDING_ADDRESS}" --rpc-url "${RPC_URL}"); then
  echo "[key-setup] Error: Failed to get balance for address ${address}"
  exit 1
fi

if [ "$balance" = "0.000000000000000000" ]; then
  echo "[key-setup] Funding address ${address} via cast..."
  cast send \
    --rpc-url ${RPC_URL} \
    --from "${FUNDING_ADDRESS}" \
    "${address}" \
    --value 1ether \
    --unlocked
fi

echo "[key-setup] Using key at ${KEYSTORE_PATH} with address ${address}"

# forward to command entry in docker-compose.yml
exec "$@"