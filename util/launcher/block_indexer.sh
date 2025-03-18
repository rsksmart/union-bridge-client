#!/bin/bash
set -e

# WS endpoint to get latest block info
WEBSOCKET_ENDPOINT="ws://rskj-01.testnet.ub.iovlabs.net:4445/websocket"

usage() {
  echo "Usage: $0 [-f <block_finality>] [-b <block_height>] [-c <true|false>] [-a <cache_size>] [-e <env>] -t <tag>"
  echo ""
  echo "  -f   Use block finality (number)."
  echo "  -b   Use a block height (number)."
  echo "       (Cannot provide both -f and -b.)"
  echo "  -c   true | false (default: true). Whether to copy a source config."
  echo "  -a   Cache size override (updates the new config file)."
  echo "  -e   Environment: 'dev' or 'stage' (default: stage)."
  echo "  -t   (Mandatory) A single tag, e.g. 'happy_path'."
  echo "       This sets: storage_path = data/<tag>,"
  echo "                  log_file     = <tag>,"
  echo "                  config_suffix= <tag>."
  exit 1
}

# Default environment
config_env="stage"

# Initialize variables
block_finality=""
block_height=""
source_original_config=true
cache_size=""
tag=""

# Parse options
while getopts "f:b:c:a:e:t:" opt; do
  case "$opt" in
    f) block_finality="$OPTARG" ;;
    b) block_height="$OPTARG" ;;
    c)
      if [[ "$OPTARG" != "true" && "$OPTARG" != "false" ]]; then
        echo "Error: -c must be 'true' or 'false' (got '$OPTARG')"
        exit 1
      fi
      source_original_config="$OPTARG"
      ;;
    a) cache_size="$OPTARG" ;;
    e) config_env="$OPTARG" ;;
    t) tag="$OPTARG" ;;
    *) usage ;;
  esac
done

# Ensure -t is provided
if [ -z "$tag" ]; then
  echo "Error: -t <tag> is mandatory."
  usage
fi

# Ensure not both -f and -b are provided
if [ -n "$block_finality" ] && [ -n "$block_height" ]; then
  echo "Error: Cannot provide both block finality (-f) and block height (-b)."
  usage
fi

# Determine the config folder based on environment
if [ "$config_env" == "dev" ]; then
  CONFIG_PATH="config/dev"
else
  CONFIG_PATH="config/stage"
fi

# Derived parameters from the tag
storage_path="data/$tag"
log_file="$tag"
config_suffix="$tag"

echo "Using config directory: $CONFIG_PATH"
[ -n "$block_finality" ] && echo "Block finality: $block_finality"
[ -n "$block_height" ] && echo "Block height: $block_height"
[ -n "$cache_size" ] && echo "Cache size: $cache_size"
echo "Storage path: $storage_path"
echo "Log file override base: $log_file"
echo "Config file override suffix: $config_suffix"

# Process log file override: create log4rs_<tag>.yaml
base="${log_file%.log}"
TEMP_LOG_CONFIG="log4rs_${base}.yaml"
cp log4rs.yaml "$TEMP_LOG_CONFIG"
sed -i.bak "s#path: \"logs/app.log\"#path: \"logs/app_${base}.log\"#" "$TEMP_LOG_CONFIG"
rm -f "$TEMP_LOG_CONFIG.bak"
LOG_CONFIG_ARG="-l $TEMP_LOG_CONFIG"

# Create a new configuration folder
NEW_CONFIG_DIR="$CONFIG_PATH/$config_suffix"
mkdir -p "$NEW_CONFIG_DIR"
NEW_CONFIG_FILE="$NEW_CONFIG_DIR/config.yaml"
if [ "$source_original_config" = "true" ]; then
  cp "$CONFIG_PATH/config.yaml" "$NEW_CONFIG_FILE"
  echo "Copied the original config to $NEW_CONFIG_FILE"
else
  echo "Did not copy any source config. Expected to find $NEW_CONFIG_FILE"
fi

###############################################################################
# Update initial_block_hash in the new config file if -f or -b is given
###############################################################################
if [ -n "$block_finality" ]; then
  # Retrieve the latest block number
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

  # Compute the target block
  TARGET_BLOCK_DEC=$((LATEST_BLOCK_DEC - block_finality))
  echo "Target block (decimal): $TARGET_BLOCK_DEC"
  TARGET_BLOCK_HEX=$(printf "0x%x" "$TARGET_BLOCK_DEC")
  echo "Target block (hex): $TARGET_BLOCK_HEX"

  # Get block hash
  GET_BLOCK_REQUEST=$(printf '{"jsonrpc":"2.0","id":2,"method":"eth_getBlockByNumber","params":["%s", false]}' "$TARGET_BLOCK_HEX")
  GET_BLOCK_RESPONSE=$(wscat -c "$WEBSOCKET_ENDPOINT" -x "$GET_BLOCK_REQUEST" 2>/dev/null)
  BLOCK_HASH=$(echo "$GET_BLOCK_RESPONSE" | jq -r '.result.hash')
  if [ -z "$BLOCK_HASH" ] || [ "$BLOCK_HASH" == "null" ]; then
    echo "Error: Could not get block hash for block $TARGET_BLOCK_HEX."
    exit 1
  fi
  echo "Retrieved block hash using block finality: $BLOCK_HASH"

  sed -i.bak -E 's/(initial_block_hash:[[:space:]]*")[^"]*(")/\1'"$BLOCK_HASH"'\2/' "$NEW_CONFIG_FILE"
  rm -f "$NEW_CONFIG_FILE.bak"

elif [ -n "$block_height" ]; then
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

  sed -i.bak -E 's/(initial_block_hash:[[:space:]]*")[^"]*(")/\1'"$BLOCK_HASH"'\2/' "$NEW_CONFIG_FILE"
  rm -f "$NEW_CONFIG_FILE.bak"
else
  echo "No block finality or block height specified. Using existing initial_block_hash in config."
fi

###############################################################################
# Update cache size if provided
###############################################################################
if [ -n "$cache_size" ]; then
  sed -i.bak "s/size: 1000/size: $cache_size/" "$NEW_CONFIG_FILE"
  rm -f "$NEW_CONFIG_FILE.bak"
  echo "Updated cache size in $NEW_CONFIG_FILE to $cache_size"
fi

###############################################################################
# Update storage path
###############################################################################
sed -i.bak "s|path: \"data\"|path: \"$storage_path\"|" "$NEW_CONFIG_FILE"
rm -f "$NEW_CONFIG_FILE.bak"
echo "Updated storage path in $NEW_CONFIG_FILE to $storage_path"

###############################################################################
# Run block-indexer with the new config folder
###############################################################################
CMD="RUST_BACKTRACE=1 cargo run --bin block-indexer -- $LOG_CONFIG_ARG -c $NEW_CONFIG_DIR"
echo "Starting block indexer with command: $CMD"
eval "$CMD"

# Clean up temporary log config
rm -f "$TEMP_LOG_CONFIG"
echo "Done."