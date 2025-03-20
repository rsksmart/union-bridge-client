#!/bin/bash
set -e

ROOT_DIRECTORY="/tmp/monitor-executions"

usage() {
  echo "Usage: $0 -t <tag> [-e <env>]"
  echo "  -t <tag>   (required) e.g. 'custom'"
  echo "  -e <env>   (optional, default: 'stage')"
  exit 1
}

# Default environment
env="stage"

tag=""

while getopts ":t:e:" opt; do
  case "$opt" in
    t)
      tag="$OPTARG"
      ;;
    e)
      env="$OPTARG"
      ;;
    *)
      usage
      ;;
  esac
done

if [ -z "$tag" ]; then
  usage
fi

target_folder="${ROOT_DIRECTORY}/$tag"
target_config_folder="$target_folder/config/$env"
target_log_folder="$target_folder/"
target_log_config_file="$target_folder/log4rs.yaml"

# Run block-indexer-validator
CMD="RUST_BACKTRACE=1 cargo run --bin block-indexer-validator -- -l $target_log_config_file -c $target_config_folder"
echo "Starting block-indexer-validator with command: $CMD"
eval "$CMD"

CMDTAIL="tail -12 ${target_log_folder}app.log"
eval "$CMDTAIL"