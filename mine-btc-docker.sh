#!/usr/bin/env bash
set -euo pipefail

# Config
CONTAINER="bitcoin-regtest"
RPCUSER="foo"
RPCPASS="rpcpassword"

# Get a new address inside the container
addr=$(docker exec "$CONTAINER" bitcoin-cli -regtest -rpcuser="$RPCUSER" -rpcpassword="$RPCPASS" getnewaddress)

echo "Mining 1 block to address: $addr"

# Mine 1 block to that address
docker exec "$CONTAINER" bitcoin-cli -regtest -rpcuser="$RPCUSER" -rpcpassword="$RPCPASS" generatetoaddress 1 "$addr"