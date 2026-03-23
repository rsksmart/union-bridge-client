#!/usr/bin/env bash

set -euo pipefail

BASE_STORAGE_PATH="${BASE_STORAGE_PATH:-$HOME}"
BROKER_ROOT="${BASE_STORAGE_PATH}/.union_bridge/broker"
SERVICES=("block-indexer" "log-indexer" "user-api" "coordinator")
OPERATOR_IDS=()

print_help() {
  echo "Usage: $0 [--op <ID> | --ops <N>]"
  echo ""
  echo "Creates or reuses host-side Union broker identities under:"
  echo "  ${BASE_STORAGE_PATH}/.union_bridge/broker/<service>/op_N.pem"
  echo "  ${BASE_STORAGE_PATH}/.union_bridge/broker/<service>/op_N.pubkey_hash"
  echo ""
  echo "Options:"
  echo "  --op <ID>    Create identities for one operator (1-10)"
  echo "  --ops <N>    Create identities for operators 1..N (1-10, default: 4)"
  echo "  --help       Display this help message"
  exit 0
}

ensure_dependencies() {
  if ! command -v openssl >/dev/null 2>&1; then
    echo "Error: openssl is required to create broker identities."
    exit 1
  fi
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --help)
      print_help
      ;;
    --op)
      if [[ -n "${2:-}" && "${2}" =~ ^(10|[1-9])$ ]]; then
        OPERATOR_IDS=("$2")
      else
        echo "Error: --op must be between 1 and 10"
        exit 1
      fi
      shift 2
      ;;
    --ops)
      if [[ -n "${2:-}" && "${2}" =~ ^(10|[1-9])$ ]]; then
        mapfile -t OPERATOR_IDS < <(seq 1 "$2")
      else
        echo "Error: --ops must be between 1 and 10"
        exit 1
      fi
      shift 2
      ;;
    *)
      echo "Error: unknown argument '$1'"
      echo "Run '$0 --help' for usage information."
      exit 1
      ;;
  esac
done

if [[ ${#OPERATOR_IDS[@]} -eq 0 ]]; then
  mapfile -t OPERATOR_IDS < <(seq 1 4)
fi

ensure_dependencies

compute_pubkey_hash() {
  local pem_path="$1"

  openssl pkey -in "${pem_path}" -pubout -outform DER 2>/dev/null \
    | openssl dgst -sha256 -binary \
    | od -A n -v -t x1 \
    | tr -d ' \n'
}

for op_num in "${OPERATOR_IDS[@]}"; do
  echo "Preparing broker identities for op_${op_num}:"

  for service in "${SERVICES[@]}"; do
    service_dir="${BROKER_ROOT}/${service}"
    pem_path="${service_dir}/op_${op_num}.pem"
    pubkey_hash_path="${service_dir}/op_${op_num}.pubkey_hash"

    mkdir -p "${service_dir}"

    if [[ -f "${pem_path}" ]]; then
      echo "  - Reusing ${service} key at ${pem_path}"
    else
      echo "  - Creating ${service} key at ${pem_path}"
      openssl genpkey -algorithm RSA -out "${pem_path}" -pkeyopt rsa_keygen_bits:2048 2>/dev/null
      chmod 600 "${pem_path}" || true
    fi

    compute_pubkey_hash "${pem_path}" > "${pubkey_hash_path}"
    echo "    pubkey_hash: $(cat "${pubkey_hash_path}")"
  done
done
