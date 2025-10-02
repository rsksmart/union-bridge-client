#!/usr/bin/env bash
set -euo pipefail

# Parse arguments: require a named parameter for stream ID
STREAM_ID=""

print_usage() {
  echo "Usage: $0 --stream-id <id>"
  echo "  Options:"
  echo "    --stream-id <id>   Mandatory stream ID (integer)"
  echo "    -s <id>            Alias for --stream-id"
  echo "    -h, --help         Show this help and exit"
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --stream-id|-s)
      if [[ $# -lt 2 ]]; then
        echo "Error: Missing value for $1" >&2
        print_usage
        exit 1
      fi
      STREAM_ID="$2"
      shift 2
      ;;
    --stream-id=*|-s=*)
      STREAM_ID="${1#*=}"
      shift
      ;;
    -h|--help)
      print_usage
      exit 0
      ;;
    *)
      echo "Unknown option: $1" >&2
      print_usage
      exit 1
      ;;
  esac
done

if [[ -z "${STREAM_ID}" ]]; then
  echo "Error: --stream-id is required" >&2
  print_usage
  exit 1
fi

# Validate numeric stream id
if ! [[ "${STREAM_ID}" =~ ^[0-9]+$ ]]; then
  echo "Error: stream id must be an integer, got: ${STREAM_ID}" >&2
  exit 1
fi

post_apply() {
  local port="$1"
  local role="$2"
  local data
  data=$(cat <<EOF
{
  "ApplyToStream": {
    "stream_id": ${STREAM_ID},
    "role": "${role}",
    "funding_utxo": {
      "value": 10000000
    },
    "speed_up_utxo": {
      "value": 10000000
    }
  }
}
EOF
)
  curl -sS -X POST "http://localhost:${port}/apply-stream" \
    -H "Content-Type: application/json" \
    -d "${data}"
}

post_apply 40001 Prover

echo
sleep 5

post_apply 40002 Prover

echo
sleep 5

post_apply 40003 Verifier

echo
sleep 5

post_apply 40004 Verifier

echo