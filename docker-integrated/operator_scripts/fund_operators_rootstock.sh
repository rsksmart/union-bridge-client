#!/usr/bin/env bash
set -euo pipefail

# Fund Rootstock (RSK) accounts for operator stacks.
# - If --env local: fetches all 4 operators, funds accounts using local Anvil via `cast send`.
# - If --env alphanet: fetches the single operator on this host, prints address for manual funding.

ENVIRONMENT=""

usage() {
  cat <<USAGE
Usage: $(basename "$0") --env <env>

Options:
  -e, --env <env>   Environment name (mandatory). Use 'local' or 'alphanet'.
  -h, --help        Show this help and exit.

Examples:
  $(basename "$0") --env local                  # Fund all 4 operators locally via Anvil
  $(basename "$0") --env alphanet               # Show funding command for alphanet
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

# Determine which projects to query
if [[ "$ENVIRONMENT" == "local" ]]; then
  PROJECTS=(op_1 op_2 op_3 op_4)
  EXPECTED_COUNT=4
else
  # alphanet: single operator on this host (uses docker-integrated project)
  PROJECTS=("docker-integrated")
  EXPECTED_COUNT=1
fi

# DRY: common helper to iterate operator projects and emit "project addr" pairs
get_signers() {
  local project addr
  for project in "${PROJECTS[@]}"; do
    # Use a guard around grep to avoid failing the whole script under set -o pipefail when no match is found
    docker compose -p "$project" logs \
      | { grep "Got member signer with address" || true; } \
      | sed 's/.*Got member signer with address //' \
      | sort -u \
      | while read -r addr; do
          [ -z "$addr" ] && continue
          echo "$project $addr"
        done
  done
}

# Count how many signers were found
count_signers() {
  get_signers | wc -l | tr -d ' '
}

fund_local() {
  local project addr
  local count
  count=$(count_signers)
  if (( count < EXPECTED_COUNT )); then
    echo "Error: expected $EXPECTED_COUNT RSK address(es) but found $count. Ensure all required operator stacks are running and emitted signer addresses." >&2
    exit 1
  fi
  while read -r project addr; do
    echo "Processing $project"
    echo "  Funding RSK address: $addr"
    cast send --rpc-url http://127.0.0.1:8545 \
      --from 0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266 \
      "$addr" --value 1ether --unlocked >/dev/null
  done < <(get_signers)
  echo "Done. Funded operator RSK addresses on local Anvil."
}

print_alphanet() {
  local project addr
  local count
  count=$(count_signers)
  if (( count < EXPECTED_COUNT )); then
    echo "Error: expected $EXPECTED_COUNT RSK address(es) but found $count. Ensure operator is running and emitted signer address." >&2
    exit 1
  fi
  echo "Operator RSK address to fund on alphanet:"
  while read -r project addr; do
    echo "  operator -> $addr"
  done < <(get_signers)
  echo ""
  echo "Fund using:"
  while read -r project addr; do
    echo "  cast send $addr --value 0.25ether --private-key <priv_key> --rpc-url <alphanet_rpc_url>"
  done < <(get_signers)
}

if [[ "$ENVIRONMENT" == "local" ]]; then
  fund_local
elif [[ "$ENVIRONMENT" == "alphanet" ]]; then
  print_alphanet
else
  echo "Error: unsupported environment '$ENVIRONMENT'" >&2
  exit 1
fi
