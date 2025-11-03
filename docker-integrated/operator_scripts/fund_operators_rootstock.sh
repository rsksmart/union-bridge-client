#!/usr/bin/env bash
set -euo pipefail

# Fund Rootstock (RSK) accounts for all operator stacks.
# - If --env local: funds accounts using local Anvil via `cast send`.
# - Otherwise: only prints the addresses so the user can fund them on that network.

ENVIRONMENT=""

usage() {
  cat <<USAGE
Usage: $(basename "$0") --env <env>

Options:
  -e, --env <env>   Environment name (mandatory). If 'local', funds via Anvil; otherwise prints addresses.
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

# DRY: common helper to iterate operator projects and emit "project addr" pairs
get_signers() {
  local project addr
  for project in op_1 op_2 op_3 op_4; do
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
  if (( count < 4 )); then
    echo "Error: expected 4 RSK addresses but found $count. Ensure all operator stacks are running and emitted signer addresses." >&2
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

print_only() {
  local project addr
  local count
  count=$(count_signers)
  if (( count < 4 )); then
    echo "Error: expected 4 RSK addresses but found $count. Ensure all operator stacks are running and emitted signer addresses." >&2
    exit 1
  fi
  echo "Listing operator RSK addresses to fund on '$ENVIRONMENT':"
  while read -r project addr; do
    echo "  $project -> $addr"
  done < <(get_signers)
  while read -r project addr; do
    echo "cast send $addr --value 0.25ether --private-key <priv_key> --rpc-url <rpc_url>"
  done < <(get_signers)

}

if [[ "$ENVIRONMENT" == "local" ]]; then
  fund_local
else
  print_only
fi
