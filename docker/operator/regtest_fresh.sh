#!/usr/bin/env bash

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REGTEST_UNION_REPO_ROOT="${REGTEST_UNION_REPO_ROOT:-$(cd "${SCRIPT_DIR}/../.." && pwd)}"
PRESET_REGTEST_CONTRACTS_REPO="${REGTEST_CONTRACTS_REPO-}"
PRESET_REGTEST_CONTRACTS_TAG="${REGTEST_CONTRACTS_TAG-}"
PRESET_REGTEST_NODE_CONTRACTS_DIR="${REGTEST_NODE_CONTRACTS_DIR-}"
REGTEST_ENV_FILE="${REGTEST_ENV_FILE:-${HOME}/regtest-fresh/.env}"
if [[ -f "$REGTEST_ENV_FILE" ]]; then
  set -a
  # shellcheck disable=SC1090
  source "$REGTEST_ENV_FILE"
  set +a
fi

REGTEST_UNION_HOST="${REGTEST_UNION_HOST:-union-bridge-use2-1.regtest.rskcomputing.net}"
REGTEST_NODE_HOST="${REGTEST_NODE_HOST:-node-use2-1.regtest.rskcomputing.net}"
REGTEST_POWPEG_HOST="${REGTEST_POWPEG_HOST:-powpeg-use2-1.regtest.rskcomputing.net}"
REGTEST_NODE_RPC_URL="${REGTEST_NODE_RPC_URL:-http://10.1.0.66:4444}"
REGTEST_POWPEG_RPC_URL="${REGTEST_POWPEG_RPC_URL:-http://10.1.0.107:18332}"

resolve_contracts_repo() {
  sed -nE 's/.*package = "union-bridge-contracts", git = "([^"]+)".*/\1/p' "${REGTEST_UNION_REPO_ROOT}/Cargo.toml" | head -n1
}

resolve_contracts_tag() {
  sed -nE 's/.*package = "union-bridge-contracts".*tag = "([^"]+)".*/\1/p' "${REGTEST_UNION_REPO_ROOT}/Cargo.toml" | head -n1
}

DEFAULT_CONTRACTS_REPO="$(resolve_contracts_repo)"
DEFAULT_CONTRACTS_TAG="$(resolve_contracts_tag)"
[[ -n "$DEFAULT_CONTRACTS_REPO" ]] || { echo "Error: could not resolve contracts repo from Cargo.toml" >&2; exit 1; }
[[ -n "$DEFAULT_CONTRACTS_TAG" ]] || { echo "Error: could not resolve contracts tag from Cargo.toml" >&2; exit 1; }

REGTEST_CONTRACTS_REPO="${PRESET_REGTEST_CONTRACTS_REPO:-$DEFAULT_CONTRACTS_REPO}"
REGTEST_CONTRACTS_TAG="${PRESET_REGTEST_CONTRACTS_TAG:-$DEFAULT_CONTRACTS_TAG}"
REGTEST_NODE_CONTRACTS_DIR="${PRESET_REGTEST_NODE_CONTRACTS_DIR:-${HOME}/.union-bridge/regtest-fresh/contracts/bitvmx-union-bridge-contracts-${REGTEST_CONTRACTS_TAG}}"
REGTEST_MAINWALLET_TARGET_BTC="${REGTEST_MAINWALLET_TARGET_BTC:-2000}"
REGTEST_TESTWALLET_TARGET_BTC="${REGTEST_TESTWALLET_TARGET_BTC:-500}"
REGTEST_DEPLOYER_TARGET_BALANCE_WEI="${REGTEST_DEPLOYER_TARGET_BALANCE_WEI:-5000000000000000000}"
REGTEST_BRIDGE_AUTH_TARGET_BALANCE_WEI="${REGTEST_BRIDGE_AUTH_TARGET_BALANCE_WEI:-1000000000000000000}"

REGTEST_RSK_RPC_URL="${REGTEST_RSK_RPC_URL:-$REGTEST_NODE_RPC_URL}"
REGTEST_RSK_RPC_LOCAL_URL="${REGTEST_RSK_RPC_LOCAL_URL:-$REGTEST_NODE_RPC_URL}"
REGTEST_BITCOIN_RPC_USER="${REGTEST_BITCOIN_RPC_USER:-user}"
REGTEST_BITCOIN_RPC_PASSWORD="${REGTEST_BITCOIN_RPC_PASSWORD:-pass}"
REGTEST_BRIDGE_AUTH_PRIVATE_KEY="${REGTEST_BRIDGE_AUTH_PRIVATE_KEY:-}"
REGTEST_BRIDGE_GAS_PRICE_WEI="${REGTEST_BRIDGE_GAS_PRICE_WEI:-4325612}"
REGTEST_NATIVE_BRIDGE_ADDRESS="${REGTEST_NATIVE_BRIDGE_ADDRESS:-0x0000000000000000000000000000000001000006}"
REGTEST_RUN_STEP_A="${REGTEST_RUN_STEP_A:-false}"
REGTEST_RUN_STEP_E2_VERIFY="${REGTEST_RUN_STEP_E2_VERIFY:-false}"

ARTIFACT_BASE="${HOME}/.union-bridge/regtest-fresh"
RUN_TIMESTAMP="$(date -u +%Y%m%dT%H%M%SZ)"
RUN_DIR="${ARTIFACT_BASE}/runs/${RUN_TIMESTAMP}"

PRE_FLIGHT_LOG="${RUN_DIR}/preflight.log"
WALLETS_LOG="${RUN_DIR}/wallets.log"
DEPLOY_LOG="${RUN_DIR}/deploy.log"
CONFIG_LOG="${RUN_DIR}/config.log"
VERIFY_LOG="${RUN_DIR}/verify.log"
BRIDGE_LOG="${RUN_DIR}/bridge.log"
OPERATORS_LOG="${RUN_DIR}/operators.log"
SUMMARY_JSON="${RUN_DIR}/summary.json"

GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m'

log() { echo -e "${BLUE}[INFO]${NC} $1"; }
ok() { echo -e "${GREEN}[OK]${NC} $1"; }
warn() { echo -e "${YELLOW}[WARN]${NC} $1"; }
die() { echo "Error: $1" >&2; exit 1; }

require_cmd() {
  local cmd="$1"
  command -v "$cmd" >/dev/null 2>&1 || die "Missing required command: ${cmd}"
}

require_env() {
  local var_name="$1"
  if [[ -z "${!var_name:-}" ]]; then
    die "Required env var is missing: ${var_name}"
  fi
}

extract_result_value() {
  local key="$1"
  local file="$2"
  local value
  value="$(grep -E "^RESULT ${key}=" "$file" | tail -n1 | cut -d'=' -f2- || true)"
  if [[ -z "$value" ]]; then
    die "Missing RESULT ${key} in ${file}"
  fi
  printf '%s\n' "$value"
}

to_lower() {
  local value="$1"
  echo "${value,,}"
}

is_true() {
  case "$(to_lower "$1")" in
    1|true|yes|y|on) return 0 ;;
    *) return 1 ;;
  esac
}

rsk_get_balance_wei() {
  local address="$1"
  local rpc_url="$2"
  cast balance "$address" --rpc-url "$rpc_url" | tr -d '\r\n[:space:]'
}

find_rsk_bootstrap_account() {
  local rpc_url="$1"
  local excluded_address="${2,,}"
  local accounts_json address balance

  accounts_json="$(curl -sS -H 'Content-Type: application/json' \
    --data '{"jsonrpc":"2.0","method":"personal_listAccounts","params":[],"id":1}' \
    "$rpc_url")"

  for address in $(echo "$accounts_json" | jq -r '.result[]?'); do
    if [[ "${address,,}" == "$excluded_address" ]]; then
      continue
    fi

    balance="$(rsk_get_balance_wei "$address" "$rpc_url")"
    if [[ "$balance" =~ ^[0-9]+$ ]] && [[ "$balance" != "0" ]]; then
      printf '%s\n' "$address"
      return 0
    fi
  done

  return 1
}

docker_container_status() {
  local container_name="$1"
  docker inspect -f '{{if .State.Health}}{{.State.Health.Status}}{{else}}{{.State.Status}}{{end}}' "$container_name" 2>/dev/null || true
}

assert_code_exists_on_rsk() {
  local label="$1"
  local address="$2"
  local code
  export PATH="${HOME}/.foundry/bin:${PATH}"
  code="$(cast code "$address" --rpc-url "$REGTEST_RSK_RPC_LOCAL_URL" | tr -d '\r\n')"
  if [[ -z "$code" || "$code" == "0x" ]]; then
    die "${label} has no deployed code at ${address}"
  fi
  echo "RESULT ${label}_code_ok=${address}"
}

resolve_path() {
  local path="$1"
  if [[ "$path" == /* ]]; then
    echo "$path"
  else
    echo "${HOME}/${path}"
  fi
}

ensure_contracts_checkout() {
  local contracts_dir="$1"
  local contracts_repo="$2"
  local contracts_tag="$3"
  local parent_dir
  parent_dir="$(dirname "$contracts_dir")"
  mkdir -p "$parent_dir"

  if [[ ! -d "$contracts_dir/.git" ]]; then
    rm -rf "$contracts_dir"
    git clone --branch "$contracts_tag" --depth 1 --recurse-submodules "$contracts_repo" "$contracts_dir"
    return 0
  fi

  git -C "$contracts_dir" fetch --tags origin
  git -C "$contracts_dir" checkout --force "$contracts_tag"
  git -C "$contracts_dir" clean -fd
  git -C "$contracts_dir" submodule update --init --recursive
}

mkdir -p "$RUN_DIR"

require_env "REGTEST_DEPLOY_MNEMONIC"
require_env "REGTEST_COW_PRIVATE_KEY"
require_env "REGTEST_USER_BITCOIN_WIF"
require_env "REGTEST_BRIDGE_AUTH_PRIVATE_KEY"

log "Artifacts directory: ${RUN_DIR}"
log "Contracts repo/tag: ${REGTEST_CONTRACTS_REPO} @ ${REGTEST_CONTRACTS_TAG}"

require_cmd jq
require_cmd curl
require_cmd git

if is_true "$REGTEST_RUN_STEP_A"; then
  log "Step A: preflight checks"
  {
    chain_id="$(curl -sS -H 'Content-Type: application/json' \
      --data '{"jsonrpc":"2.0","method":"eth_chainId","params":[],"id":1}' \
      "$REGTEST_RSK_RPC_LOCAL_URL" | jq -r '.result // empty')"
    [[ -n "$chain_id" ]] || die "Could not query eth_chainId on ${REGTEST_RSK_RPC_LOCAL_URL}"
    echo "RESULT rsk_chain_id=${chain_id}"

    bitcoin_payload='{"jsonrpc":"1.0","id":"ub","method":"getblockcount","params":[]}'
    bitcoin_response="$(curl -sS --user "${REGTEST_BITCOIN_RPC_USER}:${REGTEST_BITCOIN_RPC_PASSWORD}" \
      -H 'content-type:text/plain' \
      --data-binary "${bitcoin_payload}" \
      "${REGTEST_POWPEG_RPC_URL}")"
    bitcoin_blockcount="$(echo "$bitcoin_response" | jq -r '.result // empty')"
    [[ "$bitcoin_blockcount" =~ ^[0-9]+$ ]] || die "Could not query bitcoind blockcount on ${REGTEST_POWPEG_RPC_URL}"
    echo "RESULT bitcoin_blockcount=${bitcoin_blockcount}"
  } 2>&1 | tee "$PRE_FLIGHT_LOG"
else
  log "Step A: skipped (set REGTEST_RUN_STEP_A=true to enable)"
  echo "RESULT step_a_skipped=true" | tee "$PRE_FLIGHT_LOG"
fi

log "Step B: ensure/fund Bitcoin wallets via ${REGTEST_POWPEG_RPC_URL}"
{
  bash -s -- "$REGTEST_MAINWALLET_TARGET_BTC" "$REGTEST_TESTWALLET_TARGET_BTC" "$REGTEST_USER_BITCOIN_WIF" "$REGTEST_BITCOIN_RPC_USER" "$REGTEST_BITCOIN_RPC_PASSWORD" "$REGTEST_POWPEG_RPC_URL" <<'EOS'
set -euo pipefail

main_target="$1"
test_target="$2"
faucet_wif="$3"
rpc_user="$4"
rpc_password="$5"
rpc_url="$6"

rpc_call() {
  local wallet_path="$1"
  local method="$2"
  local params_json="$3"
  curl -sS --user "${rpc_user}:${rpc_password}" \
    -H 'content-type:text/plain' \
    --data-binary "{\"jsonrpc\":\"1.0\",\"id\":\"ub\",\"method\":\"${method}\",\"params\":${params_json}}" \
    "${rpc_url}${wallet_path}"
}

rpc_result() {
  local wallet_path="$1"
  local method="$2"
  local params_json="$3"
  local response
  response="$(rpc_call "$wallet_path" "$method" "$params_json")"
  local error_message
  error_message="$(echo "$response" | jq -r '.error.message // empty' 2>/dev/null || true)"
  if [[ -n "$error_message" ]]; then
    echo "Bitcoin RPC error (${method}): ${error_message}" >&2
    return 1
  fi
  echo "$response" | jq -cr '.result'
}

ensure_wallet_loaded() {
  local wallet_name="$1"
  local wallet_params
  wallet_params="$(jq -cn --arg w "$wallet_name" '[$w]')"

  rpc_call "" "createwallet" "$wallet_params" >/dev/null 2>&1 || true
  rpc_call "" "loadwallet" "$wallet_params" >/dev/null 2>&1 || true

  local loaded
  loaded="$(rpc_result "" "listwallets" "[]")"
  if ! echo "$loaded" | jq -e --arg wallet "$wallet_name" 'index($wallet) != null' >/dev/null; then
    echo "Failed to load wallet: ${wallet_name}" >&2
    exit 1
  fi
}

ensure_watch_wallet_loaded() {
  local wallet_name="$1"
  local create_params load_params
  create_params="$(jq -cn --arg w "$wallet_name" '[$w, true, true, "", false, true]')"
  load_params="$(jq -cn --arg w "$wallet_name" '[$w]')"

  rpc_call "" "createwallet" "$create_params" >/dev/null 2>&1 || true
  rpc_call "" "loadwallet" "$load_params" >/dev/null 2>&1 || true

  local loaded
  loaded="$(rpc_result "" "listwallets" "[]")"
  if ! echo "$loaded" | jq -e --arg wallet "$wallet_name" 'index($wallet) != null' >/dev/null; then
    echo "Failed to load watch wallet: ${wallet_name}" >&2
    exit 1
  fi
}

btc_deficit() {
  local target="$1"
  local current="$2"
  awk -v t="$target" -v c="$current" 'BEGIN {d=t-c; if (d<0) d=0; printf "%.8f", d}'
}

btc_positive() {
  local value="$1"
  awk -v v="$value" 'BEGIN { if (v > 0.00000000) exit 0; exit 1 }'
}

wallet_balance() {
  local wallet_name="$1"
  rpc_result "/wallet/${wallet_name}" "getbalances" "[]" | jq -r '.mine.trusted // 0'
}

send_to_wallet() {
  local wallet_name="$1"
  local amount="$2"
  local address_params send_params
  address_params="$(jq -cn --arg label "$wallet_name" '[$label, "bech32"]')"
  local wallet_addr
  wallet_addr="$(rpc_result "/wallet/${wallet_name}" "getnewaddress" "$address_params")"
  send_params="$(jq -cn --arg addr "$wallet_addr" --arg amount "$amount" '[$addr, ($amount | tonumber)]')"
  rpc_result "/wallet/funding" "sendtoaddress" "$send_params" >/dev/null
}

ensure_wallet_loaded "mainwallet"
ensure_wallet_loaded "test_wallet"
ensure_wallet_loaded "funding"
ensure_watch_wallet_loaded "test_wallet_watch"

descriptor="wpkh(${faucet_wif})"
descriptor_params="$(jq -cn --arg d "$descriptor" '[$d]')"
descriptor_checksum="$(rpc_result "" "getdescriptorinfo" "$descriptor_params" | jq -r '.checksum // empty')"
if [[ -z "$descriptor_checksum" ]]; then
  echo "Could not compute checksum for faucet descriptor" >&2
  exit 1
fi

full_descriptor="${descriptor}#${descriptor_checksum}"
import_params="$(jq -cn --arg desc "$full_descriptor" '[[{"desc":$desc,"timestamp":0,"active":true,"label":"faucet"}]]')"
import_response="$(rpc_call "/wallet/funding" "importdescriptors" "$import_params")"
if ! echo "$import_response" | jq -e '(.error == null) and (.result | all(.success or ((.error.message // "") | test("already|exists|active"; "i"))))' >/dev/null; then
  echo "Could not import faucet descriptor into funding wallet" >&2
  echo "$import_response" >&2
  exit 1
fi

funding_before="$(wallet_balance "funding")"
if ! btc_positive "$funding_before"; then
  bootstrap_params="$(jq -cn '["bootstrap", "bech32"]')"
  bootstrap_addr="$(rpc_result "/wallet/funding" "getnewaddress" "$bootstrap_params")"
  bootstrap_mine_params="$(jq -cn --arg addr "$bootstrap_addr" '[160, $addr]')"
  rpc_result "" "generatetoaddress" "$bootstrap_mine_params" >/dev/null
  funding_before="$(wallet_balance "funding")"
  echo "Bootstrapped funding wallet by mining 160 blocks"
fi

main_before="$(wallet_balance "mainwallet")"
test_before="$(wallet_balance "test_wallet")"
main_missing="$(btc_deficit "$main_target" "$main_before")"
test_missing="$(btc_deficit "$test_target" "$test_before")"

did_fund=false
if btc_positive "$main_missing"; then
  send_to_wallet "mainwallet" "$main_missing"
  did_fund=true
  echo "Funded mainwallet by ${main_missing} BTC"
else
  echo "mainwallet already meets target (${main_target} BTC)"
fi

if btc_positive "$test_missing"; then
  send_to_wallet "test_wallet" "$test_missing"
  did_fund=true
  echo "Funded test_wallet by ${test_missing} BTC"
else
  echo "test_wallet already meets target (${test_target} BTC)"
fi

if [[ "$did_fund" == true ]]; then
  miner_params="$(jq -cn '["miner", "bech32"]')"
  miner_addr="$(rpc_result "/wallet/funding" "getnewaddress" "$miner_params")"
  mine_params="$(jq -cn --arg addr "$miner_addr" '[3, $addr]')"
  rpc_result "" "generatetoaddress" "$mine_params" >/dev/null
  echo "Mined 3 blocks to confirm wallet top-ups"
fi

main_after="$(wallet_balance "mainwallet")"
test_after="$(wallet_balance "test_wallet")"
funding_after="$(wallet_balance "funding")"

echo "RESULT mainwallet_balance=${main_after}"
echo "RESULT test_wallet_balance=${test_after}"
echo "RESULT funding_wallet_balance=${funding_after}"
EOS
} 2>&1 | tee "$WALLETS_LOG"

MAINWALLET_BALANCE="$(extract_result_value "mainwallet_balance" "$WALLETS_LOG")"
TEST_WALLET_BALANCE="$(extract_result_value "test_wallet_balance" "$WALLETS_LOG")"

log "Step C+D: fund deployer and deploy contracts via ${REGTEST_RSK_RPC_LOCAL_URL}"
{
  bash -s -- "$REGTEST_DEPLOY_MNEMONIC" "$REGTEST_COW_PRIVATE_KEY" "$REGTEST_CONTRACTS_TAG" "$REGTEST_NODE_CONTRACTS_DIR" "$REGTEST_CONTRACTS_REPO" "$REGTEST_RSK_RPC_LOCAL_URL" "$REGTEST_DEPLOYER_TARGET_BALANCE_WEI" <<'EOS'
set -euo pipefail

deploy_mnemonic="$1"
cow_private_key="$2"
contracts_tag="$3"
contracts_dir_input="$4"
contracts_repo="$5"
rsk_rpc_local="$6"
deployer_target_wei="$7"
resolve_path() {
  local path="$1"
  if [[ "$path" == /* ]]; then
    echo "$path"
  else
    echo "${HOME}/${path}"
  fi
}

contracts_dir="$(resolve_path "$contracts_dir_input")"

ensure_foundry() {
  export PATH="${HOME}/.foundry/bin:${PATH}"
  if command -v cast >/dev/null 2>&1 && command -v forge >/dev/null 2>&1; then
    return 0
  fi
  curl -sSL https://foundry.paradigm.xyz | bash
  export PATH="${HOME}/.foundry/bin:${PATH}"
  ~/.foundry/bin/foundryup >/dev/null
}

ensure_node_tools() {
  if command -v npx >/dev/null 2>&1; then
    return 0
  fi
  if command -v sudo >/dev/null 2>&1 && command -v apt-get >/dev/null 2>&1 && sudo -n true 2>/dev/null; then
    export DEBIAN_FRONTEND=noninteractive
    sudo apt-get update -y >/dev/null
    sudo apt-get install -y nodejs npm >/dev/null
  fi
  command -v npx >/dev/null 2>&1 || {
    echo "Missing npx. Install Node.js/npm on union host or preinstall @openzeppelin/upgrades-core dependencies." >&2
    exit 1
  }
}

uint_lt() {
  local left="$1"
  local right="$2"
  if (( ${#left} < ${#right} )); then
    return 0
  fi
  if (( ${#left} > ${#right} )); then
    return 1
  fi
  [[ "$left" < "$right" ]]
}

get_balance_wei() {
  local address="$1"
  local rpc_url="$2"
  cast balance "$address" --rpc-url "$rpc_url" | tr -d '\r\n[:space:]'
}

find_bootstrap_account() {
  local rpc_url="$1"
  local excluded_address="${2,,}"
  local accounts_json address balance

  accounts_json="$(curl -sS -H 'Content-Type: application/json' \
    --data '{"jsonrpc":"2.0","method":"personal_listAccounts","params":[],"id":1}' \
    "$rpc_url")"

  for address in $(echo "$accounts_json" | jq -r '.result[]?'); do
    if [[ "${address,,}" == "$excluded_address" ]]; then
      continue
    fi

    balance="$(get_balance_wei "$address" "$rpc_url")"
    if [[ "$balance" =~ ^[0-9]+$ ]] && [[ "$balance" != "0" ]]; then
      printf '%s\n' "$address"
      return 0
    fi
  done

  return 1
}

upsert_env() {
  local key="$1"
  local value="$2"
  if grep -q "^${key}=" .env; then
    sed -i "s|^${key}=.*|${key}=${value}|" .env
  else
    echo "${key}=${value}" >> .env
  fi
}

extract_address() {
  local log_path="$1"
  local pattern="$2"
  grep -Ei "$pattern" "$log_path" | tail -n1 | grep -Eo '0x[0-9a-fA-F]{40}' | tail -n1 || true
}

require_address() {
  local label="$1"
  local addr="$2"
  if [[ -z "$addr" ]]; then
    echo "Missing address for ${label} in deploy log" >&2
    exit 1
  fi
}

fake_peg_manager_supports_mock_api() {
  local address="$1"
  local rpc_url="$2"
  [[ -n "$address" ]] || return 1
  cast call "$address" \
    "requestAdvanceFunds(string,uint64)" \
    "probe_regtest_fresh" \
    1 \
    --rpc-url "$rpc_url" >/dev/null 2>&1
}

deploy_fake_peg_manager_contract() {
  local rpc_url="$1"
  local union_repo_root="${REGTEST_UNION_REPO_ROOT:-${HOME}/union-bridge-client}"
  local bytecode
  bytecode="$(perl -ne 'if(/bytecode = "([0-9a-f]+)"/){print $1; exit}' "${union_repo_root}/common/src/mocks/fake_contracts.rs")"
  if [[ -z "$bytecode" ]]; then
    echo "Could not extract FakePegManager bytecode from union repo" >&2
    exit 1
  fi

  local deploy_json
  deploy_json="$(cast send \
    --private-key "$REGTEST_COW_PRIVATE_KEY" \
    --rpc-url "$rpc_url" \
    --gas-price 0 \
    --legacy \
    --json \
    --create "0x${bytecode}")" || {
      echo "Failed to deploy FakePegManager" >&2
      exit 1
    }

  local deployed_address
  deployed_address="$(echo "$deploy_json" | jq -r '.contractAddress // empty')"
  if [[ -z "$deployed_address" || "$deployed_address" == "null" ]]; then
    echo "Could not read FakePegManager deployment address from cast output" >&2
    echo "$deploy_json" >&2
    exit 1
  fi

  printf '%s\n' "$deployed_address"
}

ensure_contracts_checkout() {
  local contracts_dir="$1"
  local contracts_repo="$2"
  local contracts_tag="$3"
  local parent_dir
  parent_dir="$(dirname "$contracts_dir")"
  mkdir -p "$parent_dir"
  if [[ ! -d "$contracts_dir/.git" ]]; then
    rm -rf "$contracts_dir"
    git clone --branch "$contracts_tag" --depth 1 --recurse-submodules "$contracts_repo" "$contracts_dir"
    return 0
  fi
  git -C "$contracts_dir" fetch --tags origin
  git -C "$contracts_dir" checkout --force "$contracts_tag"
  git -C "$contracts_dir" clean -fd
  git -C "$contracts_dir" submodule update --init --recursive
}

patch_regtest_contracts_checkout() {
  local contracts_dir="$1"
  local contracts_tag="$2"
  local deploy_script_path="${contracts_dir}/script/deploy/01_DeployImplAndProxy.s.sol"

  [[ "$contracts_tag" == "v0.4.1-alpha" ]] || return 0
  [[ -f "$deploy_script_path" ]] || {
    echo "Missing deploy script: ${deploy_script_path}" >&2
    exit 1
  }

  python3 - "$deploy_script_path" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
source = path.read_text()
old = """        } else if (block.chainid == ChainIds.LOCAL || block.chainid == ChainIds.RSK_REGTEST) {
            // Foundry local chainid
            btcBtcNetwork = BtcNetwork.REGTEST;
            // Set Bridge Mock
            vm.startBroadcast(getDeployerKey());
            BridgeMock bridgeMock = new BridgeMock();
            vm.stopBroadcast();
            bridgeAddress = payable(address(bridgeMock));
        } else {
"""
new = """        } else if (block.chainid == ChainIds.LOCAL) {
            // Foundry local chainid
            btcBtcNetwork = BtcNetwork.REGTEST;
            // Set Bridge Mock
            vm.startBroadcast(getDeployerKey());
            BridgeMock bridgeMock = new BridgeMock();
            vm.stopBroadcast();
            bridgeAddress = payable(address(bridgeMock));
        } else if (block.chainid == ChainIds.RSK_REGTEST) {
            btcBtcNetwork = BtcNetwork.REGTEST;
        } else {
"""
if old not in source:
    raise SystemExit(f"expected deploy snippet not found in {path}")
path.write_text(source.replace(old, new, 1))
PY
}

ensure_foundry
ensure_node_tools
export PATH="${HOME}/.foundry/bin:${PATH}"

deployer_address="$(cast wallet address --mnemonic "$deploy_mnemonic" --mnemonic-index 0)"
cow_address="$(cast wallet address --private-key "$cow_private_key")"
deployer_balance="$(get_balance_wei "$deployer_address" "$rsk_rpc_local")"
if [[ ! "$deployer_balance" =~ ^[0-9]+$ ]]; then
  echo "Unexpected deployer balance: ${deployer_balance}" >&2
  exit 1
fi

cow_balance="$(get_balance_wei "$cow_address" "$rsk_rpc_local")"
if [[ ! "$cow_balance" =~ ^[0-9]+$ ]]; then
  echo "Unexpected cow balance: ${cow_balance}" >&2
  exit 1
fi

if [[ "$cow_balance" == "0" ]]; then
  bootstrap_account="$(find_bootstrap_account "$rsk_rpc_local" "$cow_address")" || {
    echo "Could not find a funded unlocked bootstrap account to seed ${cow_address}" >&2
    exit 1
  }

  cast send "$cow_address" \
    --value "$deployer_target_wei" \
    --from "$bootstrap_account" \
    --unlocked \
    --rpc-url "$rsk_rpc_local" \
    --legacy >/dev/null

  cow_balance="$(get_balance_wei "$cow_address" "$rsk_rpc_local")"
  deployer_balance="$(get_balance_wei "$deployer_address" "$rsk_rpc_local")"
fi

if uint_lt "$deployer_balance" "$deployer_target_wei"; then
  top_up_wei=$((deployer_target_wei - deployer_balance))
  cast send "$deployer_address" \
    --value "$top_up_wei" \
    --private-key "$cow_private_key" \
    --rpc-url "$rsk_rpc_local" \
    --legacy >/dev/null
  deployer_balance="$(cast balance "$deployer_address" --rpc-url "$rsk_rpc_local" | tr -d '\r\n[:space:]')"
fi

ensure_contracts_checkout "$contracts_dir" "$contracts_repo" "$contracts_tag"
patch_regtest_contracts_checkout "$contracts_dir" "$contracts_tag"
cd "$contracts_dir"
forge clean >/dev/null
forge build >/dev/null

if [[ ! -f ".env" ]]; then
  if [[ -f ".env.example" ]]; then
    cp .env.example .env
  elif [[ -f ".env.sample" ]]; then
    cp .env.sample .env
  else
    touch .env
  fi
fi

upsert_env "MNEMONIC" "\"${deploy_mnemonic}\""
upsert_env "DEPLOYER_INDEX" "0"
upsert_env "RSK_REGTEST_RPC" "\"${rsk_rpc_local}\""

if [[ -f "shell/script/deploy/deploy-regtest.sh" ]]; then
  deploy_script="shell/script/deploy/deploy-regtest.sh"
elif [[ -f "shell/script/deploy/deploy_regtest.sh" ]]; then
  deploy_script="shell/script/deploy/deploy_regtest.sh"
else
  echo "deploy-regtest script not found in contracts repo" >&2
  exit 1
fi

deploy_log="/tmp/deploy-regtest-$(date -u +%Y%m%dT%H%M%SZ).log"
bash "$deploy_script" 2>&1 | tee "$deploy_log"

fake_peg_manager="$(extract_address "$deploy_log" 'FakePegManager')"
pegin_manager="$(extract_address "$deploy_log" 'PeginManager(\.sol)?[[:space:]]+address')"
pegout_manager="$(extract_address "$deploy_log" 'PegoutManager(\.sol)?[[:space:]]+address')"
challenge_manager="$(extract_address "$deploy_log" 'ChallengeManager(\.sol)?[[:space:]]+address')"
rbtc_bridge="$(extract_address "$deploy_log" 'RbtcBridge(\.sol)?[[:space:]]+address')"
signature_manager="$(extract_address "$deploy_log" 'SignatureManager(\.sol)?[[:space:]]+address')"
committee_registry="$(extract_address "$deploy_log" 'CommitteeRegistry(\.sol)?[[:space:]]+address')"
member_registry="$(extract_address "$deploy_log" 'MemberRegistry(\.sol)?[[:space:]]+address')"
stream_manager="$(extract_address "$deploy_log" 'StreamManager(\.sol)?[[:space:]]+address')"

require_address "PeginManager" "$pegin_manager"
require_address "PegoutManager" "$pegout_manager"
require_address "ChallengeManager" "$challenge_manager"
require_address "RbtcBridge" "$rbtc_bridge"
require_address "SignatureManager" "$signature_manager"
require_address "CommitteeRegistry" "$committee_registry"
require_address "MemberRegistry" "$member_registry"
require_address "StreamManager" "$stream_manager"

if [[ -z "$fake_peg_manager" ]]; then
  echo "Deploy log does not include FakePegManager. Deploying a dedicated mock contract for regtest."
  fake_peg_manager="$(deploy_fake_peg_manager_contract "$rsk_rpc_local")"
fi

if [[ "${fake_peg_manager,,}" == "${pegout_manager,,}" ]] || \
   ! fake_peg_manager_supports_mock_api "$fake_peg_manager" "$rsk_rpc_local"; then
  echo "Configured FakePegManager is invalid or collides with live contracts. Deploying a dedicated mock contract."
  fake_peg_manager="$(deploy_fake_peg_manager_contract "$rsk_rpc_local")"
fi

for addr in "$fake_peg_manager" "$pegin_manager" "$pegout_manager" "$challenge_manager" "$rbtc_bridge" "$signature_manager" "$committee_registry" "$member_registry" "$stream_manager"; do
  code="$(cast code "$addr" --rpc-url "$rsk_rpc_local" | tr -d '\r\n')"
  if [[ -z "$code" || "$code" == "0x" ]]; then
    echo "No code deployed at ${addr}" >&2
    exit 1
  fi
done

echo "RESULT deployer_address=${deployer_address}"
echo "RESULT deployer_balance_wei=${deployer_balance}"
echo "RESULT deploy_log_path=${deploy_log}"
echo "RESULT fake_peg_manager=${fake_peg_manager}"
echo "RESULT pegin_manager=${pegin_manager}"
echo "RESULT pegout_manager=${pegout_manager}"
echo "RESULT challenge_manager=${challenge_manager}"
echo "RESULT rbtc_bridge=${rbtc_bridge}"
echo "RESULT signature_manager=${signature_manager}"
echo "RESULT committee_registry=${committee_registry}"
echo "RESULT member_registry=${member_registry}"
echo "RESULT stream_manager=${stream_manager}"
EOS
} 2>&1 | tee "$DEPLOY_LOG"

DEPLOYER_ADDRESS="$(extract_result_value "deployer_address" "$DEPLOY_LOG")"
DEPLOYER_BALANCE_WEI="$(extract_result_value "deployer_balance_wei" "$DEPLOY_LOG")"
DEPLOY_LOG_PATH_ON_NODE="$(extract_result_value "deploy_log_path" "$DEPLOY_LOG")"
FAKE_PEG_MANAGER_ADDRESS="$(extract_result_value "fake_peg_manager" "$DEPLOY_LOG")"
PEGIN_MANAGER_ADDRESS="$(extract_result_value "pegin_manager" "$DEPLOY_LOG")"
PEGOUT_MANAGER_ADDRESS="$(extract_result_value "pegout_manager" "$DEPLOY_LOG")"
CHALLENGE_MANAGER_ADDRESS="$(extract_result_value "challenge_manager" "$DEPLOY_LOG")"
RBTC_BRIDGE_ADDRESS="$(extract_result_value "rbtc_bridge" "$DEPLOY_LOG")"
SIGNATURE_MANAGER_ADDRESS="$(extract_result_value "signature_manager" "$DEPLOY_LOG")"
COMMITTEE_REGISTRY_ADDRESS="$(extract_result_value "committee_registry" "$DEPLOY_LOG")"
MEMBER_REGISTRY_ADDRESS="$(extract_result_value "member_registry" "$DEPLOY_LOG")"
STREAM_MANAGER_ADDRESS="$(extract_result_value "stream_manager" "$DEPLOY_LOG")"

log "Step E: backup/update regtest.toml on union host"
{
  config_path="${REGTEST_UNION_REPO_ROOT}/config/environment/regtest.toml"
  [[ -f "$config_path" ]] || die "Missing config file: ${config_path}"

  backup_path="${config_path}.bak.$(date -u +%Y%m%dT%H%M%SZ)"
  cp "$config_path" "$backup_path"

  update_contract_address() {
    local contract_name="$1"
    local contract_address="$2"
    local tmp_file
    tmp_file="$(mktemp)"

    awk -v name="$contract_name" -v addr="$contract_address" '
      BEGIN { target=0; updated=0 }
      /^\[\[contracts\]\]$/ { target=0 }
      $0 == "name = \"" name "\"" { target=1 }
      target == 1 && /^address = "/ {
        sub(/"[^"]+"/, "\"" addr "\"")
        target=0
        updated=1
      }
      { print }
      END { if (updated == 0) exit 42 }
    ' "$config_path" > "$tmp_file"
    rc=$?
    if [[ "$rc" -ne 0 ]]; then
      rm -f "$tmp_file"
      if [[ "$rc" -eq 42 ]]; then
        die "Could not find contract entry: ${contract_name}"
      fi
      die "Failed to update contract entry: ${contract_name}"
    fi

    mv "$tmp_file" "$config_path"
  }

  has_contract_entry() {
    local contract_name="$1"
    grep -q "^name = \"${contract_name}\"$" "$config_path"
  }

  if has_contract_entry "FakePegManager"; then
    update_contract_address "FakePegManager" "$FAKE_PEG_MANAGER_ADDRESS"
  fi
  update_contract_address "PeginManager" "$PEGIN_MANAGER_ADDRESS"
  update_contract_address "PegoutManager" "$PEGOUT_MANAGER_ADDRESS"
  update_contract_address "ChallengeManager" "$CHALLENGE_MANAGER_ADDRESS"
  update_contract_address "SignatureManager" "$SIGNATURE_MANAGER_ADDRESS"
  update_contract_address "CommitteeRegistry" "$COMMITTEE_REGISTRY_ADDRESS"
  update_contract_address "MemberRegistry" "$MEMBER_REGISTRY_ADDRESS"
  update_contract_address "StreamManager" "$STREAM_MANAGER_ADDRESS"

  echo "RESULT regtest_toml=${config_path}"
  echo "RESULT regtest_toml_backup=${backup_path}"
} 2>&1 | tee "$CONFIG_LOG"

REGTEST_TOML_PATH="$(extract_result_value "regtest_toml" "$CONFIG_LOG")"
REGTEST_TOML_BACKUP_PATH="$(extract_result_value "regtest_toml_backup" "$CONFIG_LOG")"

log "Step E.2: verify deployed code for updated addresses"
if is_true "$REGTEST_RUN_STEP_E2_VERIFY"; then
  {
    assert_code_exists_on_rsk "fake_peg_manager" "$FAKE_PEG_MANAGER_ADDRESS"
    assert_code_exists_on_rsk "pegin_manager" "$PEGIN_MANAGER_ADDRESS"
    assert_code_exists_on_rsk "pegout_manager" "$PEGOUT_MANAGER_ADDRESS"
    assert_code_exists_on_rsk "challenge_manager" "$CHALLENGE_MANAGER_ADDRESS"
    assert_code_exists_on_rsk "rbtc_bridge" "$RBTC_BRIDGE_ADDRESS"
    assert_code_exists_on_rsk "signature_manager" "$SIGNATURE_MANAGER_ADDRESS"
    assert_code_exists_on_rsk "committee_registry" "$COMMITTEE_REGISTRY_ADDRESS"
    assert_code_exists_on_rsk "member_registry" "$MEMBER_REGISTRY_ADDRESS"
    assert_code_exists_on_rsk "stream_manager" "$STREAM_MANAGER_ADDRESS"
  } 2>&1 | tee "$VERIFY_LOG"
else
  log "Step E.2: skipped (set REGTEST_RUN_STEP_E2_VERIFY=true to enable)"
  echo "RESULT step_e2_skipped=true" | tee "$VERIFY_LOG"
fi

log "Step E.3: verify managers are wired to deployed RbtcBridge"
{
  export PATH="${HOME}/.foundry/bin:${PATH}"
  pegin_rbtc_bridge="$(cast call "$PEGIN_MANAGER_ADDRESS" "rbtcBridge()(address)" --rpc-url "$REGTEST_RSK_RPC_LOCAL_URL" | tr -d '\r\n')"
  pegout_rbtc_bridge="$(cast call "$PEGOUT_MANAGER_ADDRESS" "rbtcBridge()(address)" --rpc-url "$REGTEST_RSK_RPC_LOCAL_URL" | tr -d '\r\n')"
  challenge_rbtc_bridge="$(cast call "$CHALLENGE_MANAGER_ADDRESS" "rbtcBridge()(address)" --rpc-url "$REGTEST_RSK_RPC_LOCAL_URL" | tr -d '\r\n')"
  echo "RESULT pegin_manager_rbtc_bridge=${pegin_rbtc_bridge}"
  echo "RESULT pegout_manager_rbtc_bridge=${pegout_rbtc_bridge}"
  echo "RESULT challenge_manager_rbtc_bridge=${challenge_rbtc_bridge}"
  echo "RESULT expected_rbtc_bridge=${RBTC_BRIDGE_ADDRESS}"
} 2>&1 | tee -a "$VERIFY_LOG"

EXPECTED_RBTC_BRIDGE="$(extract_result_value "expected_rbtc_bridge" "$VERIFY_LOG")"
PEGIN_RBTC_BRIDGE="$(extract_result_value "pegin_manager_rbtc_bridge" "$VERIFY_LOG")"
PEGOUT_RBTC_BRIDGE="$(extract_result_value "pegout_manager_rbtc_bridge" "$VERIFY_LOG")"
CHALLENGE_RBTC_BRIDGE="$(extract_result_value "challenge_manager_rbtc_bridge" "$VERIFY_LOG")"
for actual_bridge in "$PEGIN_RBTC_BRIDGE" "$PEGOUT_RBTC_BRIDGE" "$CHALLENGE_RBTC_BRIDGE"; do
  if [[ "$(to_lower "$actual_bridge")" != "$(to_lower "$EXPECTED_RBTC_BRIDGE")" ]]; then
    die "Manager RbtcBridge mismatch: expected ${EXPECTED_RBTC_BRIDGE}, got ${actual_bridge}"
  fi
done

log "Step F: authorize new RbtcBridge in Native Bridge"
{
  export PATH="${HOME}/.foundry/bin:${PATH}"
  bridge_auth_address="$(cast wallet address --private-key "$REGTEST_BRIDGE_AUTH_PRIVATE_KEY")"
  bridge_auth_balance="$(rsk_get_balance_wei "$bridge_auth_address" "$REGTEST_RSK_RPC_LOCAL_URL")"
  if [[ ! "$bridge_auth_balance" =~ ^[0-9]+$ ]]; then
    die "Unexpected bridge auth balance: ${bridge_auth_balance}"
  fi

  if [[ "$bridge_auth_balance" == "0" ]]; then
    bootstrap_account="$(find_rsk_bootstrap_account "$REGTEST_RSK_RPC_LOCAL_URL" "$bridge_auth_address")" || {
      die "Could not find a funded unlocked bootstrap account to seed ${bridge_auth_address}"
    }

    cast send "$bridge_auth_address" \
      --rpc-url "$REGTEST_RSK_RPC_LOCAL_URL" \
      --value "$REGTEST_BRIDGE_AUTH_TARGET_BALANCE_WEI" \
      --from "$bootstrap_account" \
      --unlocked \
      --legacy >/dev/null
  fi

  cast send "$REGTEST_NATIVE_BRIDGE_ADDRESS" \
    "setUnionBridgeContractAddressForTestnet(address)" "$RBTC_BRIDGE_ADDRESS" \
    --rpc-url "$REGTEST_RSK_RPC_LOCAL_URL" \
    --legacy \
    --value 0 \
    --gas-limit 500100 \
    --gas-price "$REGTEST_BRIDGE_GAS_PRICE_WEI" \
    --private-key "$REGTEST_BRIDGE_AUTH_PRIVATE_KEY" >/dev/null

  authorized="$(cast call "$REGTEST_NATIVE_BRIDGE_ADDRESS" "getUnionBridgeContractAddress()(address)" --rpc-url "$REGTEST_RSK_RPC_LOCAL_URL" | tr -d '\r\n')"
  echo "RESULT authorized_rbtc_bridge=${authorized}"
} 2>&1 | tee "$BRIDGE_LOG"

AUTHORIZED_RBTC_BRIDGE="$(extract_result_value "authorized_rbtc_bridge" "$BRIDGE_LOG")"
if [[ "$(to_lower "$AUTHORIZED_RBTC_BRIDGE")" != "$(to_lower "$RBTC_BRIDGE_ADDRESS")" ]]; then
  die "Native Bridge authorization mismatch: expected ${RBTC_BRIDGE_ADDRESS}, got ${AUTHORIZED_RBTC_BRIDGE}"
fi

log "Step G: restart regtest operators with --fresh"
{
  cd "$REGTEST_UNION_REPO_ROOT/docker/operator"
  USER_BITCOIN_WIF="$REGTEST_USER_BITCOIN_WIF" bash start_operators.sh --env regtest --fresh up -d

  timeout_secs=240
  deadline=$((SECONDS + timeout_secs))

  for op_num in 1 2 3 4; do
    while (( SECONDS < deadline )); do
      if [[ "$(docker_container_status "op_${op_num}-bitvmx-client-1")" == "healthy" ]]; then
        echo "RESULT bitvmx_ready_op_${op_num}=true"
        break
      fi
      sleep 2
    done
  done

  while (( SECONDS < deadline )); do
    coordinator_count=0
    coordinator_healthy_count=0

    for op_num in 1 2 3 4; do
      coordinator_container="op_${op_num}-coordinator-1"
      coordinator_status="$(docker_container_status "$coordinator_container")"

      if docker ps --format '{{.Names}}' | grep -Eq "^${coordinator_container}$"; then
        coordinator_count=$((coordinator_count + 1))
      fi
      if [[ "$coordinator_status" == "healthy" ]]; then
        coordinator_healthy_count=$((coordinator_healthy_count + 1))
      fi

      if [[ "$coordinator_status" == "healthy" ]]; then
        continue
      fi
      if [[ "$(docker_container_status "op_${op_num}-bitvmx-client-1")" != "healthy" ]]; then
        continue
      fi
      if [[ "$coordinator_status" == "starting" ]]; then
        continue
      fi

      echo "[WARN] ${coordinator_container} status='${coordinator_status:-missing}' after BitVMX broker became ready; restarting coordinator"
      docker restart "$coordinator_container" >/dev/null || true
    done

    if [[ "$coordinator_healthy_count" -eq 4 ]]; then
      echo "RESULT coordinator_count=${coordinator_count}"
      echo "RESULT coordinator_healthy_count=${coordinator_healthy_count}"
      break
    fi
    sleep 5
  done

  coordinator_count="$(docker ps --format '{{.Names}}' | grep -Ec '^op_[1-4]-coordinator-1$' || true)"
  coordinator_healthy_count=0
  for op_num in 1 2 3 4; do
    if [[ "$(docker_container_status "op_${op_num}-coordinator-1")" == "healthy" ]]; then
      coordinator_healthy_count=$((coordinator_healthy_count + 1))
    fi
  done

  if [[ "$coordinator_count" -ne 4 || "$coordinator_healthy_count" -ne 4 ]]; then
    die "Timed out waiting for 4 healthy coordinator containers (running=${coordinator_count}, healthy=${coordinator_healthy_count})"
  fi
} 2>&1 | tee "$OPERATORS_LOG"

COORDINATOR_COUNT="$(extract_result_value "coordinator_count" "$OPERATORS_LOG")"

log "Step H: write summary artifacts"
jq -n \
  --arg run_timestamp "$RUN_TIMESTAMP" \
  --arg run_dir "$RUN_DIR" \
  --arg contracts_tag "$REGTEST_CONTRACTS_TAG" \
  --arg union_host "$REGTEST_UNION_HOST" \
  --arg node_host "$REGTEST_NODE_HOST" \
  --arg powpeg_host "$REGTEST_POWPEG_HOST" \
  --arg rsk_rpc "$REGTEST_RSK_RPC_URL" \
  --arg main_target "$REGTEST_MAINWALLET_TARGET_BTC" \
  --arg test_target "$REGTEST_TESTWALLET_TARGET_BTC" \
  --arg main_balance "$MAINWALLET_BALANCE" \
  --arg test_balance "$TEST_WALLET_BALANCE" \
  --arg deployer_address "$DEPLOYER_ADDRESS" \
  --arg deployer_balance_wei "$DEPLOYER_BALANCE_WEI" \
  --arg fake_peg_manager "$FAKE_PEG_MANAGER_ADDRESS" \
  --arg pegin_manager "$PEGIN_MANAGER_ADDRESS" \
  --arg pegout_manager "$PEGOUT_MANAGER_ADDRESS" \
  --arg challenge_manager "$CHALLENGE_MANAGER_ADDRESS" \
  --arg rbtc_bridge "$RBTC_BRIDGE_ADDRESS" \
  --arg signature_manager "$SIGNATURE_MANAGER_ADDRESS" \
  --arg committee_registry "$COMMITTEE_REGISTRY_ADDRESS" \
  --arg member_registry "$MEMBER_REGISTRY_ADDRESS" \
  --arg stream_manager "$STREAM_MANAGER_ADDRESS" \
  --arg bridge_address "$REGTEST_NATIVE_BRIDGE_ADDRESS" \
  --arg authorized_rbtc_bridge "$AUTHORIZED_RBTC_BRIDGE" \
  --arg regtest_toml "$REGTEST_TOML_PATH" \
  --arg regtest_toml_backup "$REGTEST_TOML_BACKUP_PATH" \
  --arg deploy_log_path_on_node "$DEPLOY_LOG_PATH_ON_NODE" \
  --arg coordinator_count "$COORDINATOR_COUNT" \
  '{
    run_timestamp: $run_timestamp,
    run_dir: $run_dir,
    hosts: {
      union: $union_host,
      node: $node_host,
      powpeg: $powpeg_host
    },
    contracts_tag: $contracts_tag,
    rpc: {
      rootstock: $rsk_rpc
    },
    wallets: {
      mainwallet_target_btc: $main_target,
      test_wallet_target_btc: $test_target,
      mainwallet_balance_btc: $main_balance,
      test_wallet_balance_btc: $test_balance
    },
    deployer: {
      address: $deployer_address,
      balance_wei: $deployer_balance_wei
    },
    contracts: {
      fake_peg_manager: $fake_peg_manager,
      pegin_manager: $pegin_manager,
      pegout_manager: $pegout_manager,
      challenge_manager: $challenge_manager,
      rbtc_bridge: $rbtc_bridge,
      signature_manager: $signature_manager,
      committee_registry: $committee_registry,
      member_registry: $member_registry,
      stream_manager: $stream_manager
    },
    bridge: {
      native_bridge_address: $bridge_address,
      authorized_rbtc_bridge: $authorized_rbtc_bridge
    },
    config: {
      regtest_toml_path: $regtest_toml,
      regtest_toml_backup: $regtest_toml_backup
    },
    operators: {
      coordinator_count: ($coordinator_count | tonumber)
    },
    deploy_log_path_on_node: $deploy_log_path_on_node
  }' > "$SUMMARY_JSON"

ok "Regtest fresh orchestration completed"
ok "Summary: ${SUMMARY_JSON}"
