#!/usr/bin/env bash
set -euo pipefail

# Prepare Bitcoin funding for operator stacks.
# - If --env local: fetches all 4 operators, shows mine_utxo and mine_block steps (for Regtest/local).
# - If --env alphanet: fetches the single operator running on this host.

ENVIRONMENT=""

usage() {
  cat <<USAGE
Usage: $(basename "$0") --env <env>

Options:
  -e, --env <env>   Environment name (mandatory). Use 'local' or 'alphanet'.
  -h, --help        Show this help and exit.

Examples:
  $(basename "$0") --env local                  # Fund all 4 operators locally
  $(basename "$0") --env alphanet               # Fund the operator on this alphanet host
USAGE
}

# Parse args
while [[ $# -gt 0 ]]; do
  case "$1" in
    -e|--env)
      ENVIRONMENT="${2:-}"
      if [[ -z "$ENVIRONMENT" ]]; then echo "--env requires a value" >&2; exit 1; fi
      shift 2
      ;;
    -h|--help)
      usage; exit 0
      ;;
    *)
      echo "Unknown argument: $1" >&2
      usage; exit 1
      ;;
  esac
done

# Ensure --env was provided
if [[ -z "$ENVIRONMENT" ]]; then
  echo "Error: --env is mandatory" >&2
  usage
  exit 1
fi

# Determine which operators to query
if [[ "$ENVIRONMENT" == "local" ]]; then
  PORTS=(40001 40002 40003 40004)
  PROJECTS=(op_1 op_2 op_3 op_4)
  EXPECTED_COUNT=4
else
  # alphanet: single operator on this host (always uses port 40001 and docker-integrated project)
  PORTS=(40001)
  PROJECTS=(docker-integrated)
  EXPECTED_COUNT=1
fi

# Query user-api endpoints
for port in "${PORTS[@]}"; do
  echo "GET http://0.0.0.0:$port/member/bitvmx-address"
  curl -sS -X GET "http://0.0.0.0:$port/member/bitvmx-address"
  echo
done

sleep 10

# -------- Collect BitVMX funding addresses from logs --------
addresses=()

for project in "${PROJECTS[@]}"; do
  addr=$(
    docker compose -p "$project" logs 2>/dev/null \
    | grep "Received BitVMX Funding Address:" \
    | sed -nE 's/.*Received BitVMX Funding Address:[[:space:]]*([a-zA-Z0-9]+).*/\1/p' \
    | sort -u \
    | tail -n 1
  )
  if [ -n "${addr:-}" ]; then
    if [[ "$ENVIRONMENT" == "local" ]]; then
      echo "$project -> $addr" >&2
    else
      echo "operator -> $addr" >&2
    fi
    addresses+=("$addr")
  else
    echo "No BitVMX funding address found in $project logs" >&2
  fi
done

found=${#addresses[@]}
if (( found == 0 )); then
  echo "Error: could not fetch any BitVMX funding addresses from operator logs. Ensure the operator stack(s) are running." >&2
  exit 1
fi
if (( found < EXPECTED_COUNT )); then
  echo "Error: expected $EXPECTED_COUNT BitVMX funding address(es) but found $found. Ensure all required operator stacks are running and have emitted the funding address log line." >&2
  exit 1
fi

# Join addresses with commas
addr_array=$(IFS=,; echo "${addresses[*]}")

if [[ "$ENVIRONMENT" == "local" ]]; then
  cat <<EOF
Run the following commands in the bitcoin-wallet CLI (Regtest):
1 =>    clear_db   (if you see a misaligned utxos error)
1 =>    mine_utxo 900000000
2 =>    send_to_address $addr_array 20002000
3 =>    mine_block

Note: See the bitcoin-wallet README for how to start and use the CLI: ../bitcoin-wallet/README.md
EOF
elif [[ "$ENVIRONMENT" == "alphanet" ]]; then
  cat <<EOF
Run the following command in your bitcoin-wallet or wallet tooling for alphanet:
send_to_address $addr_array 20002000

Address: ${addresses[0]}
EOF
else
  cat <<EOF
Run the following command in your bitcoin-wallet or wallet tooling for '$ENVIRONMENT':
send_to_address $addr_array 20002000
EOF
fi
