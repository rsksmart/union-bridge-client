#!/bin/bash
set -e

ROOT_DIRECTORY="/tmp/monitor-executions"
WEBSOCKET_ENDPOINT="ws://rskj-01.testnet.ub.iovlabs.net:4445/websocket"

command -v wscat >/dev/null 2>&1 || { echo "wscat is required but not installed."; exit 1; }
command -v jq >/dev/null 2>&1 || { echo "jq is required but not installed."; exit 1; }

usage() {
  echo "Usage: $0 [-f <block_finality>] [-b <block_height>] [-a <cache_size>] [-c <true|false>] [-e <env>] -t <tag>"
  echo ""
  echo "  -f   Use block finality (number)."
  echo "  -b   Use a block height (number)."
  echo "       Cannot provide both -f and -b. They override and update the config file."
  echo "  -a   Cache size. It overrides and updates the config file."
  echo "  -c   Whether to copy from a default config file (true) or expects an existing config file (false) - default: true"
  echo "  -e   Environment: 'dev' or 'stage' (default: stage)."
  echo "  -t   (Mandatory) A single tag, e.g. 'happy_path'."
  exit 1
}

# Default environment
env="stage"

block_finality=""
block_height=""
cache_size=""
from_original_config=true
tag=""

while getopts "f:b:a:c:e:t:" opt; do
  case "$opt" in
    f) block_finality="$OPTARG" ;;
    b) block_height="$OPTARG" ;;
    a) cache_size="$OPTARG" ;;
    c)
      if [[ "$OPTARG" != "true" && "$OPTARG" != "false" ]]; then
        echo "Error: -c must be 'true' or 'false' (got '$OPTARG')"
        exit 1
      fi
      from_original_config="$OPTARG"
      ;;
    e) env="$OPTARG" ;;
    t) tag="$OPTARG" ;;
    *) usage ;;
  esac
done

if [ -z "$tag" ]; then
  echo "Error: -t <tag> is mandatory."
  usage
fi

if [ -n "$block_finality" ] && [ -n "$block_height" ]; then
  echo "Error: Cannot provide both block finality (-f) and block height (-b)."
  usage
fi

if [ "$env" == "dev" ]; then
  source_config_path="config/dev"
else
  source_config_path="config/stage"
fi

source_storage_folder="/tmp/monitor-executions/default/storage"
source_config_file="$source_config_path/config.yaml"
source_log_folder="logs/"
source_log_config_file="log4rs.yaml"

target_folder="${ROOT_DIRECTORY}/$tag"
mkdir -p "$target_folder"

target_storage_folder="$target_folder/storage"
target_config_folder="$target_folder/config/$env"
target_config_file="$target_config_folder/config.yaml"
target_log_folder="$target_folder/"
target_log_config_file="$target_folder/log4rs.yaml"

# Handle log4rs.yaml
mkdir -p "$target_log_folder"
cp "$source_log_config_file" "$target_log_config_file"
sed -i.bak "s|${source_log_folder}|${target_log_folder}|g" "$target_log_config_file" && rm -f "$target_log_config_file.bak"

# Handle config.yaml
if [ "$from_original_config" = "true" ]; then
  mkdir -p "$target_config_folder"
  cp "$source_config_file" "$target_config_file"
  echo "Copied the original config from $source_config_file to $target_config_file"
else
  echo "Did not copy any source config. Expected to find $target_config_file"
fi

# Update initial_block_hash in the new config file if -f or -b is provided
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
  echo "Retrieved block hash using block finality $block_finality: $BLOCK_HASH"
  sed -i.bak -E 's/(initial_block_hash:[[:space:]]*")[^"]*(")/\1'"$BLOCK_HASH"'\2/' "$target_config_file" && rm -f "$target_config_file.bak"

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
  sed -i.bak -E 's/(initial_block_hash:[[:space:]]*")[^"]*(")/\1'"$BLOCK_HASH"'\2/' "$target_config_file" && rm -f "$target_config_file.bak"

else
  echo "No block finality or block height specified. Using existing initial_block_hash in config."
fi

# Update cache size if provided
if [ -n "$cache_size" ]; then
  sed -i.bak "s/size: 1000/size: $cache_size/" "$target_config_file" && rm -f "$target_config_file.bak"
  echo "Updated cache size in $target_config_file to $cache_size"
fi

# Update storage path
sed -i.bak "s|${source_storage_folder}|${target_storage_folder}|g" "$target_config_file" && rm -f "$target_config_file.bak"
echo "Updated storage folder in $target_config_file from $source_storage_folder to $target_storage_folder"

# Run block-indexer
CMD="RUST_BACKTRACE=1 cargo run --bin block-indexer -- -l $target_log_config_file -c $target_config_folder"
echo "Starting block-indexer with command: $CMD"
eval $CMD