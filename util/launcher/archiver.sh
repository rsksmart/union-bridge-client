#!/bin/bash
set -e

usage() {
  echo "Usage: $0 [-l] [-d]"
  echo "  -l   Archive ./logs/app.log"
  echo "  -d   Archive ./dataMonitor folder"
  exit 1
}

archive_log=false
archive_data=false

# Process command-line options.
while getopts "ld" opt; do
  case "$opt" in
    l) archive_log=true ;;
    d) archive_data=true ;;
    *) usage ;;
  esac
done

# If no option is provided, show usage.
if [ "$archive_log" = false ] && [ "$archive_data" = false ]; then
  usage
fi

# Generate timestamp in format YYYYMMDD_HHMMSS.
timestamp=$(date +"%Y%m%d_%H%M%S")

if [ "$archive_log" = true ]; then
  log_file="./logs/app.log"
  if [ -f "$log_file" ]; then
    new_log_file="./logs/app_${timestamp}.log"
    echo "Archiving $log_file to $new_log_file"
    mv "$log_file" "$new_log_file"
  else
    echo "Log file $log_file does not exist."
  fi
fi

if [ "$archive_data" = true ]; then
  data_folder="./dataMonitor"
  if [ -d "$data_folder" ]; then
    new_data_folder="./dataMonitor_${timestamp}"
    echo "Archiving $data_folder to $new_data_folder"
    mv "$data_folder" "$new_data_folder"
  else
    echo "Data folder $data_folder does not exist."
  fi
fi