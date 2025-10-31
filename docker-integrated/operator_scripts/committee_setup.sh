#!/usr/bin/env bash
set -euo pipefail

# Apply operators to a stream
# - Local: Applies all 4 operators (2 Provers, 2 Verifiers) to simulate a committee
# - Alphanet: Applies the single operator on this host with specified role

STREAM_ID=""
ENVIRONMENT=""
ROLE=""

print_usage() {
  echo "Usage: $0 --stream-id <id> --env <env> [--role <role>]"
  echo ""
  echo "Options:"
  echo "  --stream-id <id>   Mandatory stream ID (integer)"
  echo "  -s <id>            Alias for --stream-id"
  echo "  --env <env>        Environment (mandatory): 'local' or 'alphanet'"
  echo "  -e <env>           Alias for --env"
  echo "  --role <role>      Role for alphanet (required): 'Prover' or 'Verifier'"
  echo "  -r <role>          Alias for --role"
  echo "  -h, --help         Show this help and exit"
  echo ""
  echo "Examples:"
  echo "  $0 --stream-id 0 --env local                      # Apply all 4 operators locally"
  echo "  $0 --stream-id 0 --env alphanet --role Prover     # Apply operator on alphanet as Prover"
  echo "  $0 --stream-id 0 --env alphanet --role Verifier   # Apply operator on alphanet as Verifier"
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
    --env|-e)
      if [[ $# -lt 2 ]]; then
        echo "Error: Missing value for $1" >&2
        print_usage
        exit 1
      fi
      ENVIRONMENT="$2"
      shift 2
      ;;
    --env=*|-e=*)
      ENVIRONMENT="${1#*=}"
      shift
      ;;
    --role|-r)
      if [[ $# -lt 2 ]]; then
        echo "Error: Missing value for $1" >&2
        print_usage
        exit 1
      fi
      ROLE="$2"
      shift 2
      ;;
    --role=*|-r=*)
      ROLE="${1#*=}"
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

# Validate required arguments
if [[ -z "${STREAM_ID}" ]]; then
  echo "Error: --stream-id is required" >&2
  print_usage
  exit 1
fi

if [[ -z "${ENVIRONMENT}" ]]; then
  echo "Error: --env is required" >&2
  print_usage
  exit 1
fi

# Validate numeric stream id
if ! [[ "${STREAM_ID}" =~ ^[0-9]+$ ]]; then
  echo "Error: stream id must be an integer, got: ${STREAM_ID}" >&2
  exit 1
fi

# Validate environment
if [[ "$ENVIRONMENT" != "local" && "$ENVIRONMENT" != "alphanet" ]]; then
  echo "Error: --env must be 'local' or 'alphanet', got: ${ENVIRONMENT}" >&2
  exit 1
fi

# Validate role for alphanet
if [[ "$ENVIRONMENT" == "alphanet" ]]; then
  if [[ -z "$ROLE" ]]; then
    echo "Error: --role is required when using --env alphanet" >&2
    print_usage
    exit 1
  fi
  if [[ "$ROLE" != "Prover" && "$ROLE" != "Verifier" ]]; then
    echo "Error: --role must be 'Prover' or 'Verifier', got: ${ROLE}" >&2
    exit 1
  fi
elif [[ -n "$ROLE" ]]; then
  echo "Warning: --role is ignored in local environment" >&2
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
  echo "Applying operator on port ${port} as ${role}..."
  curl -sS -X POST "http://localhost:${port}/member/apply-stream" \
    -H "Content-Type: application/json" \
    -d "${data}"
  echo
}

if [[ "$ENVIRONMENT" == "local" ]]; then
  # Local: apply all 4 operators (2 Provers, 2 Verifiers)
  post_apply 40001 Prover
  sleep 5

  post_apply 40002 Prover
  sleep 5

  post_apply 40003 Verifier
  sleep 5

  post_apply 40004 Verifier

  echo "Done. Applied 4 operators to stream ${STREAM_ID} (2 Provers, 2 Verifiers)"
else
  # Alphanet: apply single operator on this host with specified role
  post_apply 40001 "$ROLE"

  echo "Done. Applied operator to stream ${STREAM_ID} as ${ROLE}"
fi