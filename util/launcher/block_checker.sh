#!/bin/bash
set -e

usage() {
  echo "Usage: $0 [-e <env>] [-s <storage_path>] [-l <log_file_base>]"
  echo "       where <env> can be 'dev' or 'stage' (default: stage)."
  echo "       <log_file_base> should be a name (e.g., custom or custom.log)."
  echo "         This will create a temporary YAML file named: log4rs_custom.yaml"
  echo "         and set the log file path inside to: logs/app_custom.log"
  exit 1
}

# Default configuration environment and storage path.
config_env="stage"
storage_path=""
log_file=""

# Parse options.
while getopts "e:s:l:" opt; do
  case "$opt" in
    e) config_env="$OPTARG" ;;
    s) storage_path="$OPTARG" ;;
    l) log_file="$OPTARG" ;;
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

# Process log file override if provided.
if [ -n "$log_file" ]; then
  # Remove trailing ".log" if present.
  base="${log_file%.log}"
  echo "Log file override base: $base"

  # Create a temporary YAML file in the root directory.
  TEMP_LOG_CONFIG="log4rs_${base}.yaml"
  cp log4rs.yaml "$TEMP_LOG_CONFIG"

  # Update the log file path inside the YAML from "logs/app.log" to "logs/app_<base>.log".
  sed -i.bak "s#path: \"logs/app.log\"#path: \"logs/app_${base}.log\"#" "$TEMP_LOG_CONFIG"
  rm "$TEMP_LOG_CONFIG.bak"

  LOG_CONFIG_ARG="-l $TEMP_LOG_CONFIG"
else
  LOG_CONFIG_ARG="-l log4rs.yaml"
fi

# Build the command to launch the Rust Check Fork Tool.
CMD="RUST_BACKTRACE=1 cargo run --bin check_gaps -- $LOG_CONFIG_ARG -c $CONFIG_PATH"
[ -n "$storage_path" ] && CMD="$CMD -s $storage_path"

echo "Starting Check Fork Tool with command: $CMD"
eval "$CMD"

# Clean up the temporary log configuration file if it was created.
if [ -n "$log_file" ]; then
  rm "$TEMP_LOG_CONFIG"
fi