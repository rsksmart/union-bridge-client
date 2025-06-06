#!/usr/bin/env bash
set -e

RPC_URL="http://sc-event-mocking:2222"
COMPLETE_FLAG_FILE="/funded_key.flag"
KEYSTORE_PATH="/keystore/key.json"

# early exit if key already exists and is funded
if [ -f "$COMPLETE_FLAG_FILE" ]; then
    echo "[key-setup] Key already exists and is funded."
    exec /app/transaction-dispatcher --config-path /app/config
    # the script will end here if the key already exists
fi

echo "[key-setup] Creating and funding key with key-manager."
mkdir -p /keystore
output=$(/app/key-manager new-key -p "${KEY_STORE_PASSWORD}" -d "/keystore" 2>&1)
if [ $? -ne 0 ]; then
    echo "[key-setup] Error: Failed to generate key: $output"
    exit 1
fi

key_path=$(echo "$output" | sed -n 's/.*Generated key @ \([^,]*\),.*/\1/p')
[ -z "$key_path" ] && echo "[key-setup] Error: Could not extract key path" && exit 1

mv "$key_path" "$KEYSTORE_PATH"

address=$(echo "$output" | sed -n "s/.*address '\([^']*\)'.*/\1/p")
[ -z "$address" ] && echo "[key-setup] Error: Could not extract address" && exit 1

echo "[key-setup] Installing Foundry..."
apt-get update && apt-get install -y curl git gnupg ca-certificates
curl -L https://foundry.paradigm.xyz | bash
~/.foundry/bin/foundryup --install "${FOUNDRY_VERSION:-latest}"
mkdir -p /opt/foundry && cp -r ~/.foundry/* /opt/foundry
rm -rf ~/.foundry

echo "[key-setup] Funding address ${address} via cast..."
cast send \
  --rpc-url ${RPC_URL} \
  --from 0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266 \
  "${address}" \
  --value 1ether \
  --unlocked

echo "[key-setup] Key at $KEYSTORE_PATH with address ${address} created and funded successfully."

touch "$COMPLETE_FLAG_FILE"

exec /app/transaction-dispatcher --config-path /app/config