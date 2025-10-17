#!/usr/bin/env bash
set -euo pipefail

# Prepare Bitcoin funding for operator stacks.
# - If --env local: shows mine_utxo and mine_block steps (for Regtest/local).
# - Otherwise: only shows the send_to_address step.
# This script queries each user-api for the BitVMX funding address and prints
# the exact commands to run inside the bitcoin-wallet CLI.

ENVIRONMENT=""

usage() {
  cat <<USAGE
Usage: $(basename "$0") --env <env>

Options:
  -e, --env <env>   Environment name (mandatory). If 'local', includes mine_utxo/mine_block; otherwise only send_to_address.
  -h, --help        Show this help and exit.
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

for port in 40001 40002 40003 40004; do
  echo "GET http://0.0.0.0:$port/bitvmx-address"
  curl -sS -X GET "http://0.0.0.0:$port/bitvmx-address"
  echo
done

sleep 5

# -------- Collect BitVMX funding addresses from logs --------
addresses=()
found_projects=()
for project in op_1 op_2 op_3 op_4; do
  addr=$(
    docker compose -p "$project" logs 2>/dev/null \
    | sed -nE 's/.*Received BitVMX Funding Address:[[:space:]]*([a-z0-9]+).*/\1/p' \
    | tail -n 1
  )
  if [ -n "${addr:-}" ]; then
    echo "$project -> $addr" >&2
    addresses+=("$addr")
    found_projects+=("$project")
  else
    echo "No BitVMX funding address found in $project logs" >&2
  fi
done

expected=4
found=${#addresses[@]}
if (( found == 0 )); then
  echo "Error: could not fetch any BitVMX funding addresses from operator logs. Ensure the operator stacks are running and that bitcoind is up (start_blockchains.sh --env local)." >&2
  exit 1
fi
if (( found < expected )); then
  echo "Error: expected $expected BitVMX funding addresses but found $found. Ensure all operator stacks are running and have emitted the funding address log line." >&2
fi

# Join addresses with commas
addr_array=$(IFS=,; echo "${addresses[*]}")

if [[ "$ENVIRONMENT" == "local" ]]; then
  cat <<EOF
Run the following commands in the bitcoin-wallet CLI (Regtest):
1 =>    clear_db   (if you see a misaligned utxos error)
1 =>    mine_utxo 900000000
2 =>    send_to_address $addr_array 20001000
3 =>    mine_block

Note: See the bitcoin-wallet README for how to start and use the CLI: ../bitcoin-wallet/README.md
EOF
else
  cat <<EOF
Run the following command in your bitcoin-wallet or wallet tooling for '$ENVIRONMENT':
send_to_address $addr_array 20001000

Note: On non-local environments you should not attempt to mine_utxo or mine_block.
EOF
fi
