#!/usr/bin/env bash
set -e

# User and member keys require KEY_STORE_PASSWORD (only needed by user-api and coordinator)
if [ -n "${KEY_STORE_PASSWORD:-}" ]; then
  mkdir -p /keystore
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
else
  echo "[key-setup] KEY_STORE_PASSWORD not set, skipping user/member key generation."
fi
