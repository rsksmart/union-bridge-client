#!/usr/bin/env bash
set -euo pipefail

# Prepare Bitcoin funding for operator stacks.
# - If --env local: fetches all 4 operators locally (0.0.0.0), shows mine_utxo and mine_block steps (for Regtest/local).
# - If --env alphanet: fetches all 4 operators from their respective alphanet hosts.

# Host configuration
ALPHANET_ENDPOINTS=("union-bridge-use1-1.alphanet.rskcomputing.net:40001" "union-bridge-use1-2.alphanet.rskcomputing.net:40001" "union-bridge-use1-3.alphanet.rskcomputing.net:40001" "union-bridge-use1-4.alphanet.rskcomputing.net:40001")
LOCAL_ENDPOINTS=("0.0.0.0:40001" "0.0.0.0:40002" "0.0.0.0:40003" "0.0.0.0:40004")

ALPHANET_PROJECT_NAME="union-operator"
ALPHANET_PROJECTS=("$ALPHANET_PROJECT_NAME" "$ALPHANET_PROJECT_NAME" "$ALPHANET_PROJECT_NAME" "$ALPHANET_PROJECT_NAME")
LOCAL_PROJECTS=(op_1 op_2 op_3 op_4)

FUND_AMOUNT=20002000

ENVIRONMENT=""

usage() {
  cat <<USAGE
Usage: $(basename "$0") --env <env>

Options:
  -e, --env <env>   Environment name (mandatory). Use 'local' or 'alphanet'.
  -h, --help        Show this help and exit.

Examples:
  $(basename "$0") --env local                  # Fund all 4 operators locally
  $(basename "$0") --env alphanet               # Fund all 4 operators on alphanet hosts
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
  ENDPOINTS=("${LOCAL_ENDPOINTS[@]}")
  PROJECTS=("${LOCAL_PROJECTS[@]}")
else
  # alphanet: 4 operators on different hosts (all use port 40001 and union-operator project)
  ENDPOINTS=("${ALPHANET_ENDPOINTS[@]}")
  PROJECTS=("${ALPHANET_PROJECTS[@]}")
fi

# Query user-api endpoints
for endpoint in "${ENDPOINTS[@]}"; do
  echo "GET http://$endpoint/member/bitvmx-address"
  curl -sS -X GET "http://$endpoint/member/bitvmx-address"
  echo
done

sleep 10

# -------- Collect BitVMX funding addresses from logs --------
addresses=()

for i in "${!PROJECTS[@]}"; do
  project="${PROJECTS[$i]}"
  endpoint="${ENDPOINTS[$i]}"
  host="${endpoint%:*}"

  if [[ "$ENVIRONMENT" == "local" ]]; then
    cmd="docker compose -p \"$project\" logs 2>/dev/null | sed -nE 's/.*Received BitVMX Funding Address:[[:space:]]*([a-z0-9]+).*/\1/p' | tail -n 1"
    echo "Running: $cmd" >&2
    addr=$(eval "$cmd")
  else
    remote_cmd="docker compose -p '$project' logs 2>/dev/null | sed -nE 's/.*Received BitVMX Funding Address:[[:space:]]*([a-z0-9]+).*/\1/p' | tail -n 1"
    echo "Running: ssh ubuntu@$host \"$remote_cmd\"" >&2
    addr=$(ssh ubuntu@$host "$remote_cmd")
  fi
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
expected=${#PROJECTS[@]}
if (( found == 0 )); then
  echo "Error: could not fetch any BitVMX funding addresses from operator logs. Ensure the operator stack(s) are running." >&2
  exit 1
fi
if (( found < expected )); then
  echo "Error: expected $expected BitVMX funding address(es) but found $found. Ensure all required operator stacks are running and have emitted the funding address log line." >&2
  exit 1
fi

# Join addresses with commas
addr_array=$(IFS=,; echo "${addresses[*]}")

if [[ "$ENVIRONMENT" == "local" ]]; then
  cat <<EOF
Note: See the bitcoin-wallet README for how to start and use the CLI: ../bitcoin-wallet/README.md

Run the following commands in the bitcoin-wallet CLI (Regtest):
1 =>    clear_db   (if you see a misaligned utxos error)
2 =>    mine_utxo 900000000
3 =>    send_to_address $addr_array $FUND_AMOUNT
4 =>    mine_block
EOF
else
  cat <<EOF
Note: See the bitcoin-wallet README for how to start and use the CLI: ../bitcoin-wallet/README.md

Run the following command in your bitcoin-wallet or wallet tooling for $ENVIRONMENT:
send_to_address $addr_array $FUND_AMOUNT
EOF
fi
