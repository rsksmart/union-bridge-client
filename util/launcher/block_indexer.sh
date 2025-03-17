#!/bin/bash
set -e

# WS endpoint to get latest block info
WEBSOCKET_ENDPOINT="ws://rskj-01.testnet.ub.iovlabs.net:4445/websocket"

usage() {
  echo "Usage: $0 [-f <block_finality>] [-a <cache_size>] [-s <storage_path>] [-e <env>]"
  echo "       If -f is provided, a new block hash is fetched and written to config."
  echo "       Otherwise, the existing config value for initial_block_hash remains untouched."
  echo ""
  echo "       <env> can be 'dev' or 'stage' (default: stage)."
  exit 1
}

# Default configuration environment.
config_env="stage"

while getopts "f:a:s:e:" opt; do
  case "$opt" in
    f) block_finality="$OPTARG" ;;
    a) cache_size="$OPTARG" ;;
    s) storage_path="$OPTARG" ;;
    e) config_env="$OPTARG" ;;
    *) usage ;;
  esac
done

# Determine the config folder based on environment
if [ "$config_env" == "dev" ]; then
  CONFIG_PATH="config/dev"
else
  CONFIG_PATH="config/stage"
fi

echo "Using config directory: $CONFIG_PATH"
[ -n "$block_finality" ] && echo "Block finality: $block_finality"
[ -n "$cache_size" ] && echo "Cache size: $cache_size"
[ -n "$storage_path" ] && echo "Storage path: $storage_path"

# If block finality (-f) is provided, fetch a new block hash and update config.
if [ -n "$block_finality" ]; then

  # Query the endpoint for the latest block number.
  LATEST_BLOCK_RESPONSE=$(wscat -c "$WEBSOCKET_ENDPOINT" \
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
  GET_BLOCK_RESPONSE=$(wscat -c "$WEBSOCKET_ENDPOINT" -x "$GET_BLOCK_REQUEST" 2>/dev/null)

  # Extract the block hash from the response.
  BLOCK_HASH=$(echo "$GET_BLOCK_RESPONSE" | jq -r '.result.hash')
  if [ -z "$BLOCK_HASH" ] || [ "$BLOCK_HASH" == "null" ]; then
    echo "Error: Could not get block hash for block $TARGET_BLOCK_HEX."
    exit 1
  fi

  echo "Retrieved block hash: $BLOCK_HASH"

  CONFIG_FILE="$CONFIG_PATH/config.yaml"
  if [ -f "$CONFIG_FILE" ]; then
    echo "Updating initial_block_hash in $CONFIG_FILE"
    sed -i.bak -E 's/^( *initial_block_hash:).*$/\1 "'$BLOCK_HASH'"/' "$CONFIG_FILE"
  else
    echo "Error: Config file $CONFIG_FILE not found."
    exit 1
  fi

else
  # If -f is not specified, skip fetching block hash or updating config
  echo "No block finality specified. Will use existing config."
fi

CMD="RUST_BACKTRACE=1 cargo run --bin block-indexer -- -c $CONFIG_PATH"
[ -n "$cache_size" ] && CMD="$CMD -a $cache_size"
[ -n "$storage_path" ] && CMD="$CMD -s $storage_path"

echo "Starting block indexer with command: $CMD"
eval "$CMD"