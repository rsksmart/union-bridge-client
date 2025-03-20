#!/bin/bash
set -e

ROOT_DIRECTORY="/tmp/monitor-executions"

usage() {
  echo "Usage: $0 -t <tag>"
  echo "  -t   Tag for archiving (required). Example: happy_path"
  exit 1
}

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

archive_folder "${ROOT_DIRECTORY}/${TAG}"
