#!/bin/bash
set -e

usage() {
  echo "Usage: $0 [-l [<log_override>]] [-b [<blocks_override>]] [-g [<datalogs_override>]]"
  echo "  -l   Archive log file."
  echo "         Without parameter: archives ./logs/app.log"
  echo "         With parameter (e.g., custom): archives ./logs/app_custom.log"
  echo "  -b   Archive blocks folder."
  echo "         Without parameter: archives data/blocks"
  echo "         With parameter (e.g., custom): archives data_custom/blocks"
  echo "  -g   Archive data logs folder."
  echo "         Without parameter: archives data/logs"
  echo "         With parameter (e.g., custom): archives data_custom/logs"
  exit 1
}

# Initialize flags and override strings.
archive_log=false
archive_blocks=false
archive_data_logs=false
override_log=""
override_blocks=""
override_datalogs=""

# Manual option parsing.
while [[ "$#" -gt 0 ]]; do
  case "$1" in
    -l)
      archive_log=true
      if [[ -n "$2" && "$2" != -* ]]; then
         override_log="$2"
         shift
      fi
      ;;
    -b)
      archive_blocks=true
      if [[ -n "$2" && "$2" != -* ]]; then
         override_blocks="$2"
         shift
      fi
      ;;
    -g)
      archive_data_logs=true
      if [[ -n "$2" && "$2" != -* ]]; then
         override_datalogs="$2"
         shift
      fi
      ;;
    *)
      usage
      ;;
  esac
  shift
done

# If no archiving flag was provided, show usage.
if ! $archive_log && ! $archive_blocks && ! $archive_data_logs; then
  usage
fi

timestamp=$(date +"%Y%m%d_%H%M%S")
suffix="${timestamp}"

echo "Timestamp suffix: $suffix"

# Process log file archiving.
if $archive_log; then
  if [ -n "$override_log" ]; then
    log_file="./logs/app_${override_log}.log"
    new_log_file="./logs/app_${override_log}_${suffix}.log"
  else
    log_file="./logs/app.log"
    new_log_file="./logs/app_${suffix}.log"
  fi

  if [ -f "$log_file" ]; then
    echo "Archiving log file $log_file to $new_log_file"
    mv "$log_file" "$new_log_file"
  else
    echo "Log file $log_file does not exist."
  fi
fi

# Process blocks folder archiving.
if $archive_blocks; then
  if [ -n "$override_blocks" ]; then
    blocks_folder="${override_blocks}/blocks"
    new_blocks_folder="${override_blocks}/blocks_${suffix}"
  else
    blocks_folder="data/blocks"
    new_blocks_folder="data/blocks_${suffix}"
  fi

  if [ -d "$blocks_folder" ]; then
    echo "Archiving blocks folder $blocks_folder to $new_blocks_folder"
    mv "$blocks_folder" "$new_blocks_folder"
  else
    echo "Folder $blocks_folder does not exist."
  fi
fi

# Process data logs folder archiving.
if $archive_data_logs; then
  if [ -n "$override_datalogs" ]; then
    data_logs_folder="${override_datalogs}/logs"
    new_data_logs_folder="${override_datalogs}/logs_${suffix}"
  else
    data_logs_folder="data/logs"
    new_data_logs_folder="data/logs_${suffix}"
  fi

  if [ -d "$data_logs_folder" ]; then
    echo "Archiving data logs folder $data_logs_folder to $new_data_logs_folder"
    mv "$data_logs_folder" "$new_data_logs_folder"
  else
    echo "Folder $data_logs_folder does not exist."
  fi
fi