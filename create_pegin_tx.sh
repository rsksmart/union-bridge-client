#!/bin/bash

# Script to create a pegin transaction using bitcoin-wallet
# Usage: RSK_ADDRESS=0x... ./create_pegin_tx.sh [stream_amount] [packet_number]

if [ -z "$RSK_ADDRESS" ]; then
    echo "Error: RSK_ADDRESS environment variable not set"
    echo "Usage: RSK_ADDRESS=0x... $0 [stream_amount] [packet_number]"
    exit 1
fi

STREAM_AMOUNT=${1:-1000000}
PACKET_NUMBER=${2:-0}

echo "Getting pegin address from user-api..."
RESPONSE=$(curl -s -X POST http://localhost:40001/pegin-address \
  -H "Content-Type: application/json" \
  -d "{
    \"rootstock_deposit_address\": \"$RSK_ADDRESS\",
    \"value\": $STREAM_AMOUNT,
    \"btc_reimbursement_pub_key\": \"\"
  }")

# Check if curl succeeded
if [ $? -ne 0 ]; then
    echo "Error: Failed to connect to user-api"
    exit 1
fi

PEGIN_ADDRESS=$(echo "$RESPONSE" | jq -r '.address // empty')

if [ -z "$PEGIN_ADDRESS" ]; then
    echo "Error: Failed to get pegin address"
    echo "Response: $RESPONSE"
    exit 1
fi

echo "Parameters:"
echo "  Stream amount: $STREAM_AMOUNT"
echo "  Packet number: $PACKET_NUMBER"
echo "  RSK address: $RSK_ADDRESS"
echo "  Pegin address: $PEGIN_ADDRESS"
echo ""
echo "Now run the following command in bitcoin-wallet CLI:"
echo ""
echo "create_pegin_tx $STREAM_AMOUNT $PACKET_NUMBER $PEGIN_ADDRESS $RSK_ADDRESS"
