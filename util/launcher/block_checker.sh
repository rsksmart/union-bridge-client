#!/bin/bash
set -e

usage() {
  echo "Usage: $0 [-e <env>] [-s <storage_path>]"
  echo "       where <env> can be 'dev' or 'stage' (default: stage)"
  exit 1
}

# Set default configuration environment and storage_path.
config_env="stage"
storage_path=""

# Parse options.
while getopts "e:s:" opt; do
  case "$opt" in
    e) config_env="$OPTARG" ;;
    s) storage_path="$OPTARG" ;;
    *) usage ;;
  esac
done

# Determine configuration directory based on env.
if [ "$config_env" == "dev" ]; then
  CONFIG_PATH="config/dev"
else
  CONFIG_PATH="config/stage"
fi

echo "Using configuration environment: $config_env"
echo "Using config directory: $CONFIG_PATH"
[ -n "$storage_path" ] && echo "Using storage path: $storage_path"

# Build the command to launch the Rust Check Fork Tool.
CMD="RUST_BACKTRACE=1 cargo run --bin check_gaps -- -c $CONFIG_PATH"
[ -n "$storage_path" ] && CMD="$CMD -s $storage_path"

echo "Starting Check Fork Tool with command: $CMD"
eval "$CMD"