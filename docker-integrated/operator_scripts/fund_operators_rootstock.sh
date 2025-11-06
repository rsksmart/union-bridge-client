#!/usr/bin/env bash
set -euo pipefail

# Fund Rootstock (RSK) accounts for operator stacks.
# - If --env local: fetches all 4 operators locally (0.0.0.0), funds accounts using local Anvil via `cast send`.
# - If --env alphanet: fetches all 4 operators from their respective alphanet hosts, prints addresses for manual funding.

# Host configuration
ALPHANET_HOSTS=("union-bridge-use1-1.alphanet.rskcomputing.net" "union-bridge-use1-2.alphanet.rskcomputing.net" "union-bridge-use1-3.alphanet.rskcomputing.net" "union-bridge-use1-4.alphanet.rskcomputing.net")
LOCAL_HOSTS=("0.0.0.0" "0.0.0.0" "0.0.0.0" "0.0.0.0")

ALPHANET_PROJECT_NAME="union-operator"
ALPHANET_PROJECTS=("$ALPHANET_PROJECT_NAME" "$ALPHANET_PROJECT_NAME" "$ALPHANET_PROJECT_NAME" "$ALPHANET_PROJECT_NAME")
LOCAL_PROJECTS=(op_1 op_2 op_3 op_4)

ALPHANET_RPC_URL="ws://node-use1-1.alphanet.rskcomputing.net:4445"
LOCAL_ANVIL_ADDRESS="0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266"

ENVIRONMENT=""

usage() {
  cat <<USAGE
Usage: $(basename "$0") --env <env>

Options:
  -e, --env <env>   Environment name (mandatory). Use 'local' or 'alphanet'.
  -h, --help        Show this help and exit.

Examples:
  $(basename "$0") --env local                  # Fund all 4 operators locally via Anvil
  $(basename "$0") --env alphanet               # Show funding commands for all 4 alphanet operators
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
  HOSTS=("${LOCAL_HOSTS[@]}")
  PROJECTS=("${LOCAL_PROJECTS[@]}")
else
  # alphanet: 4 operators on different hosts (all use union-operator project)
  HOSTS=("${ALPHANET_HOSTS[@]}")
  PROJECTS=("${ALPHANET_PROJECTS[@]}")
fi

# DRY: common helper to iterate operator projects and emit "project addr" pairs
get_signers() {
  local project addr host i cmd remote_cmd
  for i in "${!PROJECTS[@]}"; do
    project="${PROJECTS[$i]}"
    host="${HOSTS[$i]}"

    if [[ "$ENVIRONMENT" == "local" ]]; then
      cmd="docker compose -p \"$project\" logs | { grep 'Got member signer with address' || true; } | sed 's/.*Got member signer with address //' | sort -u"
      echo "Running: $cmd" >&2
      eval "$cmd" | while read -r addr; do
        [ -z "$addr" ] && continue
        echo "$project $addr"
      done
    else
      remote_cmd="docker compose -p '$project' logs | { grep 'Got member signer with address' || true; } | sed 's/.*Got member signer with address //' | sort -u"
      echo "Running: ssh ubuntu@$host \"$remote_cmd\"" >&2
      ssh ubuntu@$host "$remote_cmd" | while read -r addr; do
        [ -z "$addr" ] && continue
        echo "$project $addr"
      done
    fi
  done
}

# Count how many signers were found
count_signers() {
  get_signers | wc -l | tr -d ' '
}

fund_local() {
  local project addr
  local count expected
  count=$(count_signers)
  expected=${#PROJECTS[@]}
  if (( count < expected )); then
    echo "Error: expected $expected RSK address(es) but found $count. Ensure all required operator stacks are running and emitted signer addresses." >&2
    exit 1
  fi
  while read -r project addr; do
    echo "Processing $project"
    echo "  Funding RSK address: $addr"
    cmd="cast send --rpc-url http://127.0.0.1:8545 --from $LOCAL_ANVIL_ADDRESS \"$addr\" --value 1ether --unlocked"
    echo "  Running: $cmd" >&2
    eval "$cmd" >/dev/null
  done < <(get_signers)
  echo "Done. Funded operator RSK addresses on local Anvil."
}

print_alphanet() {
  local project addr
  local count expected
  local signers_data
  local cow_private_key

  # Cache get_signers output to avoid running SSH commands multiple times
  signers_data=$(get_signers)
  count=$(echo "$signers_data" | wc -l | tr -d ' ')
  expected=${#PROJECTS[@]}

  if (( count < expected )); then
    echo "Error: expected $expected RSK address(es) but found $count. Ensure all operators are running and emitted signer addresses." >&2
    exit 1
  fi
  echo "Operator RSK addresses to fund on alphanet:"
  while read -r project addr; do
    echo "  operator -> $addr"
  done <<< "$signers_data"
  echo ""

  # Prompt for Cow Private Key
  read -sp "Enter Cow Private Key: " cow_private_key
  echo ""

  if [[ -z "$cow_private_key" ]]; then
    echo "Error: Private key is required" >&2
    exit 1
  fi

  echo ""
  echo "Fund using:"
  while read -r project addr; do
    echo "  cast send $addr --value 0.25ether --private-key $cow_private_key --rpc-url $ALPHANET_RPC_URL"
  done <<< "$signers_data"
}

if [[ "$ENVIRONMENT" == "local" ]]; then
  fund_local
elif [[ "$ENVIRONMENT" == "alphanet" ]]; then
  print_alphanet
else
  echo "Error: unsupported environment '$ENVIRONMENT'" >&2
  exit 1
fi
