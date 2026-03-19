#!/usr/bin/env bash
set -e

# Create keystore directory
mkdir -p /keystore

BROKER_KEY_PATH="/keystore/broker.key"
BROKER_LOCK_PATH="/keystore/.broker-key.lock"

# Generate broker key if not exists (needed by all services).
# Multiple containers share /keystore and start concurrently, so we
# serialize with flock to prevent concurrent overwrites of broker.key.
flock -x "${BROKER_LOCK_PATH}" sh -c "
  if [ ! -f '${BROKER_KEY_PATH}' ]; then
    echo '[key-setup] Generating broker key.'
    openssl genpkey -algorithm RSA -out '${BROKER_KEY_PATH}' -pkeyopt rsa_keygen_bits:2048 2>/dev/null
    echo '[key-setup] Broker key created at ${BROKER_KEY_PATH}'
  else
    echo '[key-setup] Broker key already exists at ${BROKER_KEY_PATH}'
  fi
  # Broker pubkey hash = SHA256(public key DER), hex.
  # Used by bitvmx-client entrypoint to patch components.l2/components.bitvmx pubkey_hash.
  openssl pkey -in '${BROKER_KEY_PATH}' -pubout -outform DER 2>/dev/null | openssl dgst -sha256 -binary | od -A n -v -t x1 | tr -d ' \n' > /keystore/broker.pubkey_hash
"

# User and member keys require KEY_STORE_PASSWORD (only needed by user-api and coordinator)
if [ -n "${KEY_STORE_PASSWORD:-}" ]; then
  USER_KEYSTORE_PATH="/keystore/user-key.json"
  USER_LOCK_PATH="/keystore/.user-key.lock"
  MEMBER_KEYSTORE_PATH="/keystore/member-key.json"
  MEMBER_LOCK_PATH="/keystore/.member-key.lock"

  # Generate USER key if not exists (race-safe across containers via flock)
  flock -x "${USER_LOCK_PATH}" sh -c "
    set -e
    if [ ! -f '${USER_KEYSTORE_PATH}' ]; then
      echo '[key-setup] Creating new USER key with key-manager.'
      if ! output=\$(/app/key-manager new-key -p \"\${KEY_STORE_PASSWORD}\" -d '/keystore' 2>&1); then
        echo \"[key-setup] Error: Failed to generate user key: \$output\"
        exit 1
      fi

      key_path=\$(echo \"\$output\" | sed -n 's/.*Generated key @ \([^,]*\),.*/\1/p')
      [ -z \"\$key_path\" ] && echo '[key-setup] Error: Could not extract user key path' && exit 1

      mv \"\$key_path\" '${USER_KEYSTORE_PATH}'
      echo '[key-setup] User key created at ${USER_KEYSTORE_PATH}'
    else
      echo '[key-setup] User key already exists at ${USER_KEYSTORE_PATH}'
    fi
  "

  # Generate MEMBER key if not exists (race-safe across containers via flock)
  flock -x "${MEMBER_LOCK_PATH}" sh -c "
    set -e
    if [ ! -f '${MEMBER_KEYSTORE_PATH}' ]; then
      echo '[key-setup] Creating new MEMBER key with key-manager.'
      if ! output=\$(/app/key-manager new-key -p \"\${KEY_STORE_PASSWORD}\" -d '/keystore' 2>&1); then
        echo \"[key-setup] Error: Failed to generate member key: \$output\"
        exit 1
      fi

      key_path=\$(echo \"\$output\" | sed -n 's/.*Generated key @ \([^,]*\),.*/\1/p')
      [ -z \"\$key_path\" ] && echo '[key-setup] Error: Could not extract member key path' && exit 1

      mv \"\$key_path\" '${MEMBER_KEYSTORE_PATH}'
      echo '[key-setup] Member key created at ${MEMBER_KEYSTORE_PATH}'
    else
      echo '[key-setup] Member key already exists at ${MEMBER_KEYSTORE_PATH}'
    fi
  "

  echo "[key-setup] Keys ready:"
  echo "  - User: ${USER_KEYSTORE_PATH}"
  echo "  - Member: ${MEMBER_KEYSTORE_PATH}"
  echo "  - Broker: ${BROKER_KEY_PATH}"
else
  echo "[key-setup] Keys ready:"
  echo "  - Broker: ${BROKER_KEY_PATH}"
  echo "[key-setup] KEY_STORE_PASSWORD not set, skipping user/member key generation."
fi

# Forward to command
exec "$@"
