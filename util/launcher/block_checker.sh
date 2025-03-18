#!/bin/bash
set -e

usage() {
  echo "Usage: $0 -t <tag> [-e <env>]"
  echo "  -t <tag>   (required) e.g. 'custom'"
  echo "  -e <env>   (optional, default: 'stage')"
  exit 1
}

# Default environment
config_env="stage"
tag=""

# Parse command-line options
while getopts ":t:e:" opt; do
  case "$opt" in
    t)
      tag="$OPTARG"
      ;;
    e)
      config_env="$OPTARG"
      ;;
    *)
      usage
      ;;
  esac
done

# Check that we have a required tag
if [ -z "$tag" ]; then
  usage
fi

# Determine config path
CONFIG_PATH="config/$config_env/$tag"

# Set derived values
LOG_FILE_BASE="$tag"

echo "Using configuration environment: $config_env"
echo "Using config directory: $CONFIG_PATH"
echo "Log file override base: $LOG_FILE_BASE"
echo

# Create a temporary log4rs_<tag>.yaml by copying the default log4rs.yaml
TEMP_LOG_CONFIG="log4rs_${LOG_FILE_BASE}.yaml"
cp log4rs.yaml "$TEMP_LOG_CONFIG"

# Update the path in the new log4rs config to logs/app_<tag>.log
sed -i.bak "s#path: \"logs/app.log\"#path: \"logs/app_${LOG_FILE_BASE}.log\"#" "$TEMP_LOG_CONFIG"
rm -f "$TEMP_LOG_CONFIG.bak"

# Build up the command to run
CMD="RUST_BACKTRACE=1 cargo run --bin check_gaps -- -l $TEMP_LOG_CONFIG -c $CONFIG_PATH"

echo "Starting Check Fork Tool with command: $CMD"
eval "$CMD"

# Clean up the temporary file
rm -f "$TEMP_LOG_CONFIG"
