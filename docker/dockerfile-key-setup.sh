#!/usr/bin/env bash

mkdir -p /keystore

# Run the command and capture output
output=$(/app/key-manager new-key -p "${KEY_STORE_PASSWORD}" -d "/keystore" 2>&1)
# fail if the key generation fails
if [ $? -ne 0 ]; then
    echo "Error: Failed to generate key: $output"
    exit 1
fi


# Extract the generated key path using sed
key_path=$(echo "$output" | sed -n 's/.*Generated key @ \([^,]*\),.*/\1/p')

if [ -z "$key_path" ]; then
    echo "Error: Failed to extract key path from output"
    exit 1
fi

mv "$key_path" /keystore/key.json # should match the transaction-dispatcher configured path
echo "Done: Found key at $key_path"