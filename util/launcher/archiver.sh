#!/bin/bash
set -e

usage() {
  echo "Usage: $0 -t <tag> [-e <env>]"
  echo "  -t   Tag for archiving (required). Example: custom"
  echo "  -e   Environment (optional). Defaults to 'stage'"
  exit 1
}

# Default values
ENV="stage"
TAG=""

# Parse CLI options
while [[ $# -gt 0 ]]; do
  case "$1" in
    -t)
      if [[ -z "$2" || "$2" =~ ^- ]]; then
        usage
      fi
      TAG="$2"
      shift 2
      ;;
    -e)
      if [[ -z "$2" || "$2" =~ ^- ]]; then
        usage
      fi
      ENV="$2"
      shift 2
      ;;
    *)
      usage
      ;;
  esac
done

# Check required parameter
if [[ -z "$TAG" ]]; then
  usage
fi

timestamp=$(date +"%Y%m%d_%H%M%S")
echo "Archiving with timestamp suffix: $timestamp"
echo "Tag: $TAG"
echo "Env: $ENV"
echo

# Helper function for archiving folders
archive_folder() {
  local folder="$1"
  if [[ -d "$folder" ]]; then
    local folder_with_suffix="${folder}_${timestamp}"
    echo "Archiving folder $folder -> $folder_with_suffix"
    mv "$folder" "$folder_with_suffix"
  else
    echo "Folder $folder does not exist or is not a directory. Skipping."
  fi
}

# Helper function for archiving single files
archive_file() {
  local file="$1"
  if [[ -f "$file" ]]; then
    local extension="${file##*.}"           # e.g. "log"
    local base="${file%.*}"                 # e.g. "logs/app_custom"
    local new_file="${base}_${timestamp}.${extension}"
    echo "Archiving file $file -> $new_file"
    mv "$file" "$new_file"
  else
    echo "File $file does not exist. Skipping."
  fi
}

# 1) data/<tag> (folder)
archive_folder "data/${TAG}"

# 2) logs/app_<tag>.log
archive_file "logs/app_${TAG}.log"

# 3) data/<tag>/blocks (folder)
archive_folder "data/${TAG}/blocks"

# 4) data/<tag>/logs (folder)
archive_folder "data/${TAG}/logs"

# 5) log4rs_<tag>.yaml
for f in log4rs_"${TAG}".yaml; do
  # Skip if the glob doesn't match any actual files
  [[ -f "$f" ]] || continue
  archive_file "$f"
done

# 6) config/<env>/<tag> (folder)
archive_folder "config/${ENV}/${TAG}"

echo
echo "Archiving completed."