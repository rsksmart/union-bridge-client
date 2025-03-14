#!/bin/bash
set -e

usage() {
  echo "Usage: $0 -f <block_finality> [-a <cache_size>] [-s <storage_path>]"
  exit 1
}

# Parse command-line options.
while getopts "f:a:s:" opt; do
  case $opt in
    f) block_finality="$OPTARG" ;;
    a) cache_size="$OPTARG" ;;
    s) storage_path="$OPTARG" ;;
    *) usage ;;
  esac
done

# Ensure the required parameter is provided.
if [ -z "$block_finality" ]; then
  usage
fi

echo "Using block finality: $block_finality"
[ -n "$cache_size" ] && echo "Using cache size: $cache_size"
[ -n "$storage_path" ] && echo "Using storage path: $storage_path"

# Query the websocket endpoint for the latest block number.
LATEST_BLOCK_RESPONSE=$(wscat -c ws://rskj-01.testnet.ub.iovlabs.net:4445/websocket \
  -x '{"jsonrpc":"2.0","id":1,"method":"eth_blockNumber","params":[]}' 2>/dev/null)

# Extract the latest block number (hex string) from the JSON response.
LATEST_BLOCK_HEX=$(echo "$LATEST_BLOCK_RESPONSE" | jq -r '.result')
if [ -z "$LATEST_BLOCK_HEX" ] || [ "$LATEST_BLOCK_HEX" == "null" ]; then
  echo "Error: Could not get latest block number."
  exit 1
fi

echo "Latest block (hex): $LATEST_BLOCK_HEX"

# Convert the latest block hex to decimal.
LATEST_BLOCK_DEC=$((16#${LATEST_BLOCK_HEX:2}))
echo "Latest block (decimal): $LATEST_BLOCK_DEC"

# Subtract the block finality (provided in decimal) to get the target block.
TARGET_BLOCK_DEC=$((LATEST_BLOCK_DEC - block_finality))
echo "Target block (decimal): $TARGET_BLOCK_DEC"

# Convert the target block number back to hex.
TARGET_BLOCK_HEX=$(printf "0x%x" "$TARGET_BLOCK_DEC")
echo "Target block (hex): $TARGET_BLOCK_HEX"

# Build JSON payload for retrieving the block by number.
GET_BLOCK_REQUEST=$(printf '{"jsonrpc":"2.0","id":2,"method":"eth_getBlockByNumber","params":["%s", false]}' "$TARGET_BLOCK_HEX")
GET_BLOCK_RESPONSE=$(wscat -c ws://rskj-01.testnet.ub.iovlabs.net:4445/websocket -x "$GET_BLOCK_REQUEST" 2>/dev/null)

# Extract the block hash from the response.
BLOCK_HASH=$(echo "$GET_BLOCK_RESPONSE" | jq -r '.result.hash')
if [ -z "$BLOCK_HASH" ] || [ "$BLOCK_HASH" == "null" ]; then
  echo "Error: Could not get block hash for block $TARGET_BLOCK_HEX."
  exit 1
fi

echo "Retrieved block hash: $BLOCK_HASH"

# Build the command to launch the Rust block indexer,
# passing the calculated block hash and the optional cache size and storage path.
# Note the use of '--' to pass arguments to the binary.
CMD="RUST_BACKTRACE=1 RUST_LOG=info cargo run --bin block-indexer -- -b $BLOCK_HASH"
[ -n "$cache_size" ] && CMD="$CMD -a $cache_size"
[ -n "$storage_path" ] && CMD="$CMD -s $storage_path"

echo "Starting block indexer with command: $CMD"
eval "$CMD"