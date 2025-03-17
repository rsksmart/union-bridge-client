#!/bin/bash
set -e

usage() {
  echo "Usage: $0 [-l] [-b] [-g] [-t <tag>]"
  echo "  -l   Archive ./logs/app.log"
  echo "  -b   Archive dataMonitor/blocks folder"
  echo "  -g   Archive dataMonitor/logs folder"
  echo "  -t   Optional tag to insert before the timestamp (e.g., test_happy_path)"
  exit 1
}

archive_log=false
archive_blocks=false
archive_data_logs=false
tag=""

while getopts "lbgt:" opt; do
  case "$opt" in
    l) archive_log=true ;;
    b) archive_blocks=true ;;
    g) archive_data_logs=true ;;
    t) tag="$OPTARG" ;;
    *) usage ;;
  esac
done
shift $((OPTIND - 1))

if [ "$archive_log" = false ] && [ "$archive_blocks" = false ] && [ "$archive_data_logs" = false ]; then
  usage
fi

timestamp=$(date +"%Y%m%d_%H%M%S")

if [ -n "$tag" ]; then
  suffix="${tag}_${timestamp}"
else
  suffix="${timestamp}"
fi

if [ "$archive_log" = true ]; then
  log_file="./logs/app.log"
  if [ -f "$log_file" ]; then
    new_log_file="./logs/app_${suffix}.log"
    echo "Archiving $log_file to $new_log_file"
    mv "$log_file" "$new_log_file"
  else
    echo "Log file $log_file does not exist."
  fi
fi

if [ "$archive_blocks" = true ]; then
  blocks_folder="dataMonitor/blocks"
  if [ -d "$blocks_folder" ]; then
    new_blocks_folder="dataMonitor/blocks_${suffix}"
    echo "Archiving $blocks_folder to $new_blocks_folder"
    mv "$blocks_folder" "$new_blocks_folder"
  else
    echo "Folder $blocks_folder does not exist."
  fi
fi

if [ "$archive_data_logs" = true ]; then
  data_logs_folder="dataMonitor/logs"
  if [ -d "$data_logs_folder" ]; then
    new_data_logs_folder="dataMonitor/logs_${suffix}"
    echo "Archiving $data_logs_folder to $new_data_logs_folder"
    mv "$data_logs_folder" "$new_data_logs_folder"
  else
    echo "Folder $data_logs_folder does not exist."
  fi
fi