#!/bin/bash
set -e

# WS endpoint to get latest block info
WEBSOCKET_ENDPOINT="ws://rskj-01.testnet.ub.iovlabs.net:4445/websocket"

usage() {
  echo "Usage: $0 [-f <block_finality>] [-b <block_height>] [-a <cache_size>] [-s <storage_path>] [-e <env>] [-l <log_file_base>]"
  echo "       If -f is provided, a new block hash is fetched (using block finality) and written to config."
  echo "       If -b is provided, that block height is used to fetch the corresponding block hash, updating config."
  echo "       It is not possible to provide both -f and -b."
  echo ""
  echo "       <env> can be 'dev' or 'stage' (default: stage)."
  echo "       <log_file_base> should be a name (e.g., custom or custom.log)."
  echo "         This will create a temporary YAML file named: log4rs_custom.yaml"
  echo "         and set the log file path inside to: logs/app_custom.log"
  exit 1
}

# Default configuration environment.
config_env="stage"

# Initialize variables.
block_finality=""
block_height=""
cache_size=""
storage_path=""
log_file=""

# Parse command-line options.
while getopts "f:b:a:s:e:l:" opt; do
  case "$opt" in
    f) block_finality="$OPTARG" ;;
    b) block_height="$OPTARG" ;;
    a) cache_size="$OPTARG" ;;
    s) storage_path="$OPTARG" ;;
    e) config_env="$OPTARG" ;;
    l) log_file="$OPTARG" ;;
    *) usage ;;
  esac
done

# Ensure not both -f and -b are provided.
if [ -n "$block_finality" ] && [ -n "$block_height" ]; then
  echo "Error: Cannot provide both block finality (-f) and block height (-b)."
  usage
fi

# Determine the config folder based on environment.
if [ "$config_env" == "dev" ]; then
  CONFIG_PATH="config/dev"
else
  CONFIG_PATH="config/stage"
fi

echo "Using config directory: $CONFIG_PATH"
[ -n "$block_finality" ] && echo "Block finality: $block_finality"
[ -n "$block_height" ] && echo "Block height: $block_height"
[ -n "$cache_size" ] && echo "Cache size: $cache_size"
[ -n "$storage_path" ] && echo "Storage path: $storage_path"

# Process the log file base name if provided.
if [ -n "$log_file" ]; then
  # Remove a trailing ".log" if present.
  base="${log_file%.log}"
  echo "Log file override base: $base"

  # Create a temporary YAML file in the root directory.
  TEMP_LOG_CONFIG="log4rs_${base}.yaml"
  cp log4rs.yaml "$TEMP_LOG_CONFIG"

  # Replace the default log file path ("logs/app.log") with "logs/app_<base>.log".
  sed -i.bak "s#path: \"logs/app.log\"#path: \"logs/app_${base}.log\"#" "$TEMP_LOG_CONFIG"
  rm "$TEMP_LOG_CONFIG.bak"

  LOG_CONFIG_ARG="-l $TEMP_LOG_CONFIG"
else
  LOG_CONFIG_ARG="-l log4rs.yaml"
fi

# Update initial block hash in config.
if [ -n "$block_finality" ]; then
  # Fetch new block hash based on block finality.
  LATEST_BLOCK_RESPONSE=$(wscat -c "$WEBSOCKET_ENDPOINT" \
    -x '{"jsonrpc":"2.0","id":1,"method":"eth_blockNumber","params":[]}' 2>/dev/null)

  LATEST_BLOCK_HEX=$(echo "$LATEST_BLOCK_RESPONSE" | jq -r '.result')
  if [ -z "$LATEST_BLOCK_HEX" ] || [ "$LATEST_BLOCK_HEX" == "null" ]; then
    echo "Error: Could not get latest block number."
    exit 1
  fi

  echo "Latest block (hex): $LATEST_BLOCK_HEX"

  LATEST_BLOCK_DEC=$((16#${LATEST_BLOCK_HEX:2}))
  echo "Latest block (decimal): $LATEST_BLOCK_DEC"

  TARGET_BLOCK_DEC=$((LATEST_BLOCK_DEC - block_finality))
  echo "Target block (decimal): $TARGET_BLOCK_DEC"

  TARGET_BLOCK_HEX=$(printf "0x%x" "$TARGET_BLOCK_DEC")
  echo "Target block (hex): $TARGET_BLOCK_HEX"

  GET_BLOCK_REQUEST=$(printf '{"jsonrpc":"2.0","id":2,"method":"eth_getBlockByNumber","params":["%s", false]}' "$TARGET_BLOCK_HEX")
  GET_BLOCK_RESPONSE=$(wscat -c "$WEBSOCKET_ENDPOINT" -x "$GET_BLOCK_REQUEST" 2>/dev/null)

  BLOCK_HASH=$(echo "$GET_BLOCK_RESPONSE" | jq -r '.result.hash')
  if [ -z "$BLOCK_HASH" ] || [ "$BLOCK_HASH" == "null" ]; then
    echo "Error: Could not get block hash for block $TARGET_BLOCK_HEX."
    exit 1
  fi

  echo "Retrieved block hash using block finality: $BLOCK_HASH"

  CONFIG_FILE="$CONFIG_PATH/config.yaml"
  if [ -f "$CONFIG_FILE" ]; then
    echo "Updating initial_block_hash in $CONFIG_FILE using block finality"
    sed -i.bak -E 's/^( *initial_block_hash:).*$/\1 "'$BLOCK_HASH'"/' "$CONFIG_FILE"
  else
    echo "Error: Config file $CONFIG_PATH/config.yaml not found."
    exit 1
  fi

elif [ -n "$block_height" ]; then
  # Use the provided block height.
  TARGET_BLOCK_HEX=$(printf "0x%x" "$block_height")
  echo "Using block height $block_height, converted to hex: $TARGET_BLOCK_HEX"

  GET_BLOCK_REQUEST=$(printf '{"jsonrpc":"2.0","id":2,"method":"eth_getBlockByNumber","params":["%s", false]}' "$TARGET_BLOCK_HEX")
  GET_BLOCK_RESPONSE=$(wscat -c "$WEBSOCKET_ENDPOINT" -x "$GET_BLOCK_REQUEST" 2>/dev/null)

  BLOCK_HASH=$(echo "$GET_BLOCK_RESPONSE" | jq -r '.result.hash')
  if [ -z "$BLOCK_HASH" ] || [ "$BLOCK_HASH" == "null" ]; then
    echo "Error: Could not get block hash for block height $block_height (hex: $TARGET_BLOCK_HEX)."
    exit 1
  fi

  echo "Retrieved block hash for block height $block_height: $BLOCK_HASH"

  CONFIG_FILE="$CONFIG_PATH/config.yaml"
  if [ -f "$CONFIG_FILE" ]; then
    echo "Updating initial_block_hash in $CONFIG_FILE using block height"
    sed -i.bak -E 's/^( *initial_block_hash:).*$/\1 "'$BLOCK_HASH'"/' "$CONFIG_FILE"
  else
    echo "Error: Config file $CONFIG_PATH/config.yaml not found."
    exit 1
  fi
else
  echo "No block finality or block height specified. Will use existing config."
fi

# Build the command line for cargo run, passing the log configuration file and other options.
CMD="RUST_BACKTRACE=1 cargo run --bin block-indexer -- $LOG_CONFIG_ARG -c $CONFIG_PATH"
[ -n "$cache_size" ] && CMD="$CMD -a $cache_size"
[ -n "$storage_path" ] && CMD="$CMD -s $storage_path"

echo "Starting block indexer with command: $CMD"
eval "$CMD"

# Clean up the temporary log configuration file if it was created.
if [ -n "$log_file" ]; then
  rm "$TEMP_LOG_CONFIG"
fi