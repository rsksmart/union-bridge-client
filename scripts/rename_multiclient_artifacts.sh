#!/usr/bin/env bash

set -euo pipefail

if [[ -z "${BASE_STORAGE_PATH:-}" ]]; then
  echo "Error: BASE_STORAGE_PATH is required."
  echo "Example: BASE_STORAGE_PATH=/Users/illuque ./scripts/rename_broker_identity_files.sh"
  exit 1
fi

union_bridge_root="${BASE_STORAGE_PATH}/.union_bridge"

if [[ ! -d "${union_bridge_root}" ]]; then
  echo "Error: .union_bridge directory not found: ${union_bridge_root}"
  exit 1
fi

renamed_any=false

while IFS= read -r path; do
  parent_dir="$(dirname "${path}")"
  target_path="${parent_dir}/op_$(basename "${path}")"

  if [[ -e "${target_path}" ]]; then
    echo "Skipping ${path}: target already exists at ${target_path}"
    continue
  fi

  mv "${path}" "${target_path}"
  echo "Renamed ${path} -> ${target_path}"
  renamed_any=true
done < <(
  find "${union_bridge_root}" -depth -type d -path '*/multi-client/[0-9]*'
)

while IFS= read -r path; do
  base_name="$(basename "${path}")"

  if [[ ! "${base_name}" =~ ^multi-client-([0-9]+)(.*)$ ]]; then
    continue
  fi

  operator_id="${BASH_REMATCH[1]}"
  suffix="${BASH_REMATCH[2]}"
  target_path="$(dirname "${path}")/op_${operator_id}${suffix}"

  if [[ -e "${target_path}" ]]; then
    echo "Skipping ${path}: target already exists at ${target_path}"
    continue
  fi

  mv "${path}" "${target_path}"
  echo "Renamed ${path} -> ${target_path}"
  renamed_any=true
done < <(
  find "${union_bridge_root}" -depth \( -type f -o -type d \) -name 'multi-client-*'
)

if [[ "${renamed_any}" != true ]]; then
  echo "No multi-client artifacts found under ${union_bridge_root}"
fi
