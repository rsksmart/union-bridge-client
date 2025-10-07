#!/bin/bash

# Script to create a pegin transaction using bitcoin-wallet
# Usage: ./create_pegin_tx.sh [stream_amount] [packet_number]
# Automatically gets current RSK address and Bitcoin pubkey from user-api

STREAM_AMOUNT=${1:-1000000}
PACKET_NUMBER=${2:-0}

echo "Getting current Bitcoin info from user-api..."
BITCOIN_INFO=$(curl -s http://localhost:40001/bitcoin-info)

if [ $? -ne 0 ]; then
    echo "Error: Failed to connect to user-api"
    exit 1
fi

RSK_ADDRESS=$(echo "$BITCOIN_INFO" | jq -r '.rsk_address')
BTC_PUBKEY_HEX=$(echo "$BITCOIN_INFO" | jq -r '.btc_xonly_pubkey')

if [ "$RSK_ADDRESS" = "null" ] || [ "$BTC_PUBKEY_HEX" = "null" ]; then
    echo "Error: Failed to get Bitcoin info from user-api"
    echo "$BITCOIN_INFO"
    exit 1
fi

# Convert to x-only public key format (remove first byte if compressed)
if [ ${#BTC_PUBKEY_HEX} -eq 66 ]; then
    # It's a compressed public key, convert to x-only (32 bytes)
    X_ONLY_KEY="${BTC_PUBKEY_HEX:2}"
else
    X_ONLY_KEY="$BTC_PUBKEY_HEX"
fi

echo "Getting pegin address from user-api..."
echo "Parameters:"
echo "  Stream amount: $STREAM_AMOUNT"
echo "  Packet number: $PACKET_NUMBER"
echo "  RSK address: $RSK_ADDRESS"
echo "  BTC reimbursement key: 0x$X_ONLY_KEY"

# Get the pegin address from user-api
RESPONSE=$(curl -s -X POST http://localhost:40001/pegin-address \
  -H "Content-Type: application/json" \
  -d "{
    \"rootstock_deposit_address\": \"$RSK_ADDRESS\",
    \"value\": $STREAM_AMOUNT,
    \"btc_reimbursement_pub_key\": \"0x$X_ONLY_KEY\"
  }")

# Check if curl succeeded
if [ $? -ne 0 ]; then
    echo "Error: Failed to connect to user-api"
    exit 1
fi

# Extract the address from response
PEGIN_ADDRESS=$(echo "$RESPONSE" | jq -r '.address // empty')

if [ -z "$PEGIN_ADDRESS" ]; then
    echo "Error: Failed to get pegin address from user-api"
    echo "Response: $RESPONSE"
    exit 1
fi

echo "Got pegin address: $PEGIN_ADDRESS"
echo ""
echo "IMPORTANT: The command below uses the public key from /bitcoin-info endpoint."
echo "If user-api has restarted, this might be outdated!"
echo ""
echo "Current Bitcoin info:"
echo "  RSK: $RSK_ADDRESS"
echo "  X-only: $BTC_PUBKEY_HEX"
echo ""
echo "Now run the following command in bitcoin-wallet CLI:"
echo ""
echo "create_pegin_tx $STREAM_AMOUNT $PACKET_NUMBER $PEGIN_ADDRESS $RSK_ADDRESS $X_ONLY_KEY"
echo ""
echo "Or run directly with:"
echo "cd ~/union-bridge-client/bitcoin-wallet && echo 'create_pegin_tx $STREAM_AMOUNT $PACKET_NUMBER $PEGIN_ADDRESS $RSK_ADDRESS $X_ONLY_KEY' | ./target/debug/ub-wallet"
