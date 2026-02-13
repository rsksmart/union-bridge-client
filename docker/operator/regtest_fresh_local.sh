#!/usr/bin/env bash

set -euo pipefail

REGTEST_UNION_HOST="${REGTEST_UNION_HOST:-union-bridge-use2-1.regtest.rskcomputing.net}"
REGTEST_NODE_HOST="${REGTEST_NODE_HOST:-node-use2-1.regtest.rskcomputing.net}"
REGTEST_POWPEG_HOST="${REGTEST_POWPEG_HOST:-powpeg-use2-1.regtest.rskcomputing.net}"
REGTEST_SSH_USER="${REGTEST_SSH_USER:-ubuntu}"

REGTEST_UNION_REPO_ROOT="${REGTEST_UNION_REPO_ROOT:-/home/${REGTEST_SSH_USER}/union-bridge-client}"
REGTEST_NODE_CONTRACTS_DIR="${REGTEST_NODE_CONTRACTS_DIR:-bitvmx-union-bridge-contracts}"
REGTEST_CONTRACTS_REPO="${REGTEST_CONTRACTS_REPO:-git@github.com:rsksmart/bitvmx-union-bridge-contracts.git}"

REGTEST_CONTRACTS_TAG="${REGTEST_CONTRACTS_TAG:-v0.2.0-alpha.1}"
REGTEST_MAINWALLET_TARGET_BTC="${REGTEST_MAINWALLET_TARGET_BTC:-2000}"
REGTEST_TESTWALLET_TARGET_BTC="${REGTEST_TESTWALLET_TARGET_BTC:-500}"
REGTEST_DEPLOYER_TARGET_BALANCE_WEI="${REGTEST_DEPLOYER_TARGET_BALANCE_WEI:-5000000000000000000}"
REGTEST_DEPLOY_MNEMONIC="${REGTEST_DEPLOY_MNEMONIC:-calm truth betray steel define people rookie weird actor door spatial diagram}"
REGTEST_COW_PRIVATE_KEY="${REGTEST_COW_PRIVATE_KEY:-0xc85ef7d79691fe79573b1a7064c19c1a9819ebdbd1faaab1a8ec92344438aaf4}"
REGTEST_USER_BITCOIN_WIF="${REGTEST_USER_BITCOIN_WIF:-cNg5o9Y66xNDT1EBBRExi1mkd2Yv8eXgn3TD41w5pRLFgmCRnuRC}"
REGTEST_RUN_STEP_A="${REGTEST_RUN_STEP_A:-false}"
REGTEST_RUN_STEP_B="${REGTEST_RUN_STEP_B:-true}"
REGTEST_RUN_STEP_E2_VERIFY="${REGTEST_RUN_STEP_E2_VERIFY:-false}"

REGTEST_RSK_RPC_URL="${REGTEST_RSK_RPC_URL:-http://node-use2-1.regtest.rskcomputing.net:4444}"
REGTEST_RSK_RPC_LOCAL_URL="${REGTEST_RSK_RPC_LOCAL_URL:-http://127.0.0.1:4444}"
REGTEST_BITCOIN_RPC_USER="${REGTEST_BITCOIN_RPC_USER:-user}"
REGTEST_BITCOIN_RPC_PASSWORD="${REGTEST_BITCOIN_RPC_PASSWORD:-pass}"
REGTEST_BRIDGE_AUTH_PRIVATE_KEY="${REGTEST_BRIDGE_AUTH_PRIVATE_KEY:-7880a81a4591568b0e87947e5150fe8e330091678654f3bc661b516f91a5f00a}"
REGTEST_BRIDGE_GAS_PRICE_WEI="${REGTEST_BRIDGE_GAS_PRICE_WEI:-4325612}"
REGTEST_NATIVE_BRIDGE_ADDRESS="${REGTEST_NATIVE_BRIDGE_ADDRESS:-0x0000000000000000000000000000000001000006}"
REGTEST_SSH_CONNECT_TIMEOUT="${REGTEST_SSH_CONNECT_TIMEOUT:-15}"
REGTEST_CHECK_FORK_GUEST_ELF_PATH="${REGTEST_CHECK_FORK_GUEST_ELF_PATH:-${REGTEST_UNION_REPO_ROOT}/docker/bitvmx-client/config/regtest/client/config/check-fork-guest.bin}"
REGTEST_RESTART_OP1_DISPATCHER="${REGTEST_RESTART_OP1_DISPATCHER:-true}"
REGTEST_OP1_DISPATCHER_IP="${REGTEST_OP1_DISPATCHER_IP:-127.0.0.1}"
REGTEST_OP1_DISPATCHER_PORT="${REGTEST_OP1_DISPATCHER_PORT:-22222}"
REGTEST_BITVMX_WORKSPACE_ROOT="${REGTEST_BITVMX_WORKSPACE_ROOT:-/home/${REGTEST_SSH_USER}/rust-bitvmx-workspace-v0.1.4-alpha}"
REGTEST_OP1_DISPATCHER_CWD="${REGTEST_OP1_DISPATCHER_CWD:-${REGTEST_BITVMX_WORKSPACE_ROOT}/rust-bitvmx-client}"
REGTEST_OP1_DISPATCHER_BIN="${REGTEST_OP1_DISPATCHER_BIN:-${REGTEST_BITVMX_WORKSPACE_ROOT}/rust-bitvmx-job-dispatcher/target/release/bitvmx-risczero-dispatcher}"
REGTEST_OP1_DISPATCHER_LOG="${REGTEST_OP1_DISPATCHER_LOG:-/tmp/op1-dispatcher-v014.log}"

ARTIFACT_BASE="${HOME}/.union-bridge/regtest-fresh"
RUN_TIMESTAMP="$(date -u +%Y%m%dT%H%M%SZ)"
RUN_DIR="${ARTIFACT_BASE}/runs/${RUN_TIMESTAMP}"

SSH_BASE_CMD=(ssh -A -o BatchMode=yes -o ConnectTimeout="${REGTEST_SSH_CONNECT_TIMEOUT}" -o ServerAliveInterval=15 -o ServerAliveCountMax=2)

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

ssh_target() {
  local host="$1"
  echo "${REGTEST_SSH_USER}@${host}"
}

ssh_exec() {
  local host="$1"
  shift
  "${SSH_BASE_CMD[@]}" "$(ssh_target "$host")" "$@"
}

shell_quote() {
  printf '%q' "$1"
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
  printf '%s' "$value" | tr '[:upper:]' '[:lower:]'
}

is_true() {
  case "$(to_lower "$1")" in
    1|true|yes|y|on) return 0 ;;
    *) return 1 ;;
  esac
}

assert_code_exists_on_node() {
  local label="$1"
  local address="$2"
  local code
  code="$(ssh_exec "$REGTEST_NODE_HOST" "export PATH=\"\$HOME/.foundry/bin:\$PATH\"; cast code '$address' --rpc-url '$REGTEST_RSK_RPC_LOCAL_URL'" | tr -d '\r\n')"
  if [[ -z "$code" || "$code" == "0x" ]]; then
    die "${label} has no deployed code at ${address}"
  fi
  echo "RESULT ${label}_code_ok=${address}"
}

mkdir -p "$RUN_DIR"

log "Artifacts directory: ${RUN_DIR}"

require_cmd ssh
require_cmd jq
require_cmd curl

if is_true "$REGTEST_RUN_STEP_A"; then
  log "Step A: preflight checks"
  {
    for host in "$REGTEST_UNION_HOST" "$REGTEST_POWPEG_HOST" "$REGTEST_NODE_HOST"; do
      ssh_exec "$host" "echo connected: \$(hostname)" >/dev/null
      echo "RESULT ssh_ok_${host}=true"
    done

    chain_id="$(
      curl -sS -H 'Content-Type: application/json' \
        --data '{"jsonrpc":"2.0","method":"eth_chainId","params":[],"id":1}' \
        "$REGTEST_RSK_RPC_URL" | jq -r '.result // empty'
    )"
    [[ -n "$chain_id" ]] || die "Could not query eth_chainId on ${REGTEST_RSK_RPC_URL}"
    echo "RESULT rsk_chain_id=${chain_id}"

    bitcoin_payload='{"jsonrpc":"1.0","id":"ub","method":"getblockcount","params":[]}'
    bitcoin_response="$(
      ssh_exec "$REGTEST_POWPEG_HOST" \
        "curl -sS --user ${REGTEST_BITCOIN_RPC_USER}:${REGTEST_BITCOIN_RPC_PASSWORD} -H 'content-type:text/plain' --data-binary '${bitcoin_payload}' http://127.0.0.1:18332"
    )"
    bitcoin_blockcount="$(echo "$bitcoin_response" | jq -r '.result // empty')"
    [[ "$bitcoin_blockcount" =~ ^[0-9]+$ ]] || die "Could not query bitcoind blockcount on powpeg host"
    echo "RESULT bitcoin_blockcount=${bitcoin_blockcount}"
  } 2>&1 | tee "$PRE_FLIGHT_LOG"
else
  log "Step A: skipped (set REGTEST_RUN_STEP_A=true to enable)"
  echo "RESULT step_a_skipped=true" | tee "$PRE_FLIGHT_LOG"
fi

MAINWALLET_BALANCE="skipped"
TEST_WALLET_BALANCE="skipped"

if is_true "$REGTEST_RUN_STEP_B"; then
  log "Step B: ensure/fund Bitcoin wallets on ${REGTEST_POWPEG_HOST}"
  {
    remote_cmd="bash -s -- $(shell_quote "$REGTEST_MAINWALLET_TARGET_BTC") $(shell_quote "$REGTEST_TESTWALLET_TARGET_BTC") $(shell_quote "$REGTEST_USER_BITCOIN_WIF") $(shell_quote "$REGTEST_BITCOIN_RPC_USER") $(shell_quote "$REGTEST_BITCOIN_RPC_PASSWORD")"
    ssh_exec "$REGTEST_POWPEG_HOST" "$remote_cmd" <<'EOS'
set -euo pipefail

main_target="$1"
test_target="$2"
faucet_wif="$3"
rpc_user="$4"
rpc_password="$5"
rpc_url="http://127.0.0.1:18332"

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
else
  log "Step B: skipped (set REGTEST_RUN_STEP_B=false to disable)"
  echo "RESULT step_b_skipped=true" | tee "$WALLETS_LOG"
fi

log "Step C+D: fund deployer and deploy contracts on ${REGTEST_NODE_HOST}"
{
  remote_cmd="bash -s -- $(shell_quote "$REGTEST_DEPLOY_MNEMONIC") $(shell_quote "$REGTEST_COW_PRIVATE_KEY") $(shell_quote "$REGTEST_CONTRACTS_TAG") $(shell_quote "$REGTEST_CONTRACTS_REPO") $(shell_quote "$REGTEST_NODE_CONTRACTS_DIR") $(shell_quote "$REGTEST_RSK_RPC_LOCAL_URL") $(shell_quote "$REGTEST_DEPLOYER_TARGET_BALANCE_WEI")"
  ssh_exec "$REGTEST_NODE_HOST" "$remote_cmd" <<'EOS'
set -euo pipefail

deploy_mnemonic="$1"
cow_private_key="$2"
contracts_tag="$3"
contracts_repo="$4"
contracts_dir_input="$5"
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

ensure_foundry
export PATH="${HOME}/.foundry/bin:${PATH}"

deployer_address="$(cast wallet address --mnemonic "$deploy_mnemonic" --mnemonic-index 0)"
deployer_balance="$(cast balance "$deployer_address" --rpc-url "$rsk_rpc_local" | tr -d '\r\n[:space:]')"
if [[ ! "$deployer_balance" =~ ^[0-9]+$ ]]; then
  echo "Unexpected deployer balance: ${deployer_balance}" >&2
  exit 1
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

if [[ ! -d "${contracts_dir}/.git" ]]; then
  if [[ -d "${contracts_dir}" && -n "$(ls -A "${contracts_dir}" 2>/dev/null)" ]]; then
    echo "Using existing non-git contracts directory: ${contracts_dir}"
  else
    git clone "$contracts_repo" "$contracts_dir"
  fi
fi

cd "$contracts_dir"
if [[ -d ".git" ]]; then
  git fetch --tags origin
  git checkout --force "$contracts_tag"
else
  echo "Warning: ${contracts_dir} is not a git checkout; skipping tag checkout (${contracts_tag}) and using current files."
fi

# Regtest must use the real native bridge precompile in PegManager.
# Some contract tags deploy BridgeMock for RSK_REGTEST by default.
if [[ -f "script/deploy/01_DeployImplAndProxy.s.sol" ]]; then
  python3 - <<'PY'
from pathlib import Path

p = Path("script/deploy/01_DeployImplAndProxy.s.sol")
s = p.read_text()

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
            // Rootstock regtest must use the real native bridge precompile
            btcBtcNetwork = BtcNetwork.REGTEST;
            bridgeAddress = RSK_BRIDGE_ADDRESS;
        } else {
"""

if old in s:
    p.write_text(s.replace(old, new))
    print("RESULT deploy_script_patch_applied=true")
else:
    print("RESULT deploy_script_patch_applied=false")
PY
fi

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

deploy_log="/tmp/deploy-regtest-$(date -u +%Y%m%dT%H%M%SZ).log"

if [[ ! -f "script/deploy/DeployScript.s.sol" ]]; then
  echo "Missing deploy script: script/deploy/DeployScript.s.sol" >&2
  exit 1
fi

set +e
forge script script/deploy/DeployScript.s.sol \
  --rpc-url "$rsk_rpc_local" \
  --legacy \
  --broadcast \
  --gas-price 60000000 \
  --slow \
  -vvvv \
  --force 2>&1 | tee "$deploy_log"
deploy_main_exit="${PIPESTATUS[0]}"
set -e
if [[ "$deploy_main_exit" -ne 0 ]]; then
  echo "DeployScript.s.sol failed with exit code ${deploy_main_exit}" >&2
  exit "$deploy_main_exit"
fi

if [[ -f "script/deploy/DeployFakePegManager.s.sol" ]]; then
  set +e
  forge script script/deploy/DeployFakePegManager.s.sol \
    --rpc-url "$rsk_rpc_local" \
    --legacy \
    --broadcast \
    --gas-price 60000000 \
    --slow \
    -vvvv \
    --force 2>&1 | tee -a "$deploy_log"
  deploy_fake_exit="${PIPESTATUS[0]}"
  set -e
  if [[ "$deploy_fake_exit" -ne 0 ]]; then
    echo "DeployFakePegManager.s.sol failed with exit code ${deploy_fake_exit}" >&2
    exit "$deploy_fake_exit"
  fi
else
  echo "Warning: script/deploy/DeployFakePegManager.s.sol not found; FakePegManager will fallback to PegManager address." | tee -a "$deploy_log"
fi

fake_peg_manager="$(extract_address "$deploy_log" 'FakePegManager(\.sol)?[[:space:]]+address:[[:space:]]*0x|FakePegManager.*deployed at.*0x')"
peg_manager="$(extract_address "$deploy_log" 'PegManager(\.sol)?[[:space:]]+address:[[:space:]]*0x|pegManager:[[:space:]]*0x')"
signature_manager="$(extract_address "$deploy_log" 'SignatureManager(\.sol)?[[:space:]]+address:[[:space:]]*0x|signatureManager:[[:space:]]*0x')"
committee_registry="$(extract_address "$deploy_log" 'CommitteeRegistry(\.sol)?[[:space:]]+address:[[:space:]]*0x|committeeRegistry:[[:space:]]*0x')"
member_registry="$(extract_address "$deploy_log" 'MemberRegistry(\.sol)?[[:space:]]+address:[[:space:]]*0x|memberRegistry:[[:space:]]*0x')"
stream_manager="$(extract_address "$deploy_log" 'StreamManager(\.sol)?[[:space:]]+address:[[:space:]]*0x|streamManager:[[:space:]]*0x')"

require_address "PegManager" "$peg_manager"
require_address "SignatureManager" "$signature_manager"
require_address "CommitteeRegistry" "$committee_registry"
require_address "MemberRegistry" "$member_registry"
require_address "StreamManager" "$stream_manager"

if [[ -z "$fake_peg_manager" ]]; then
  fake_peg_manager="$peg_manager"
  echo "Warning: FakePegManager address not present in deploy log; using PegManager address (${fake_peg_manager}) as fallback."
fi

for addr in "$fake_peg_manager" "$peg_manager" "$signature_manager" "$committee_registry" "$member_registry" "$stream_manager"; do
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
echo "RESULT peg_manager=${peg_manager}"
echo "RESULT signature_manager=${signature_manager}"
echo "RESULT committee_registry=${committee_registry}"
echo "RESULT member_registry=${member_registry}"
echo "RESULT stream_manager=${stream_manager}"
EOS
} 2>&1 | tee "$DEPLOY_LOG"

DEPLOYER_ADDRESS="$(extract_result_value "deployer_address" "$DEPLOY_LOG")"
DEPLOYER_BALANCE_WEI="$(extract_result_value "deployer_balance_wei" "$DEPLOY_LOG")"
DEPLOY_LOG_PATH_ON_NODE="$(extract_result_value "deploy_log_path" "$DEPLOY_LOG")"
DEPLOY_SCRIPT_PATCH_APPLIED="$(grep -E "^RESULT deploy_script_patch_applied=" "$DEPLOY_LOG" | tail -n1 | cut -d'=' -f2- || true)"
FAKE_PEG_MANAGER_ADDRESS="$(extract_result_value "fake_peg_manager" "$DEPLOY_LOG")"
PEG_MANAGER_ADDRESS="$(extract_result_value "peg_manager" "$DEPLOY_LOG")"
SIGNATURE_MANAGER_ADDRESS="$(extract_result_value "signature_manager" "$DEPLOY_LOG")"
COMMITTEE_REGISTRY_ADDRESS="$(extract_result_value "committee_registry" "$DEPLOY_LOG")"
MEMBER_REGISTRY_ADDRESS="$(extract_result_value "member_registry" "$DEPLOY_LOG")"
STREAM_MANAGER_ADDRESS="$(extract_result_value "stream_manager" "$DEPLOY_LOG")"

CONTRACT_UPDATE_PAIRS=(
  "FakePegManager=${FAKE_PEG_MANAGER_ADDRESS}"
  "PegManager=${PEG_MANAGER_ADDRESS}"
  "SignatureManager=${SIGNATURE_MANAGER_ADDRESS}"
  "CommitteeRegistry=${COMMITTEE_REGISTRY_ADDRESS}"
  "MemberRegistry=${MEMBER_REGISTRY_ADDRESS}"
  "StreamManager=${STREAM_MANAGER_ADDRESS}"
)

log "Step E: backup/update regtest.toml on union host"
{
  remote_cmd_parts=("bash -s --" "$(shell_quote "$REGTEST_UNION_REPO_ROOT")")
  for contract_pair in "${CONTRACT_UPDATE_PAIRS[@]}"; do
    remote_cmd_parts+=("$(shell_quote "$contract_pair")")
  done
  remote_cmd="${remote_cmd_parts[*]}"
  ssh_exec "$REGTEST_UNION_HOST" "$remote_cmd" <<'EOS'
set -euo pipefail

union_repo_root="$1"
shift

config_path="${union_repo_root}/config/environment/regtest.toml"
if [[ ! -f "$config_path" ]]; then
  echo "Missing config file: ${config_path}" >&2
  exit 1
fi

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
      echo "Could not find contract entry: ${contract_name}" >&2
      exit 1
    fi
    echo "Failed to update contract entry: ${contract_name}" >&2
    exit 1
  fi

  mv "$tmp_file" "$config_path"
}

for contract_pair in "$@"; do
  contract_name="${contract_pair%%=*}"
  contract_address="${contract_pair#*=}"
  if [[ -z "$contract_name" || -z "$contract_address" || "$contract_name" == "$contract_address" ]]; then
    echo "Invalid contract pair: ${contract_pair}" >&2
    exit 1
  fi
  update_contract_address "$contract_name" "$contract_address"
done

echo "RESULT regtest_toml=${config_path}"
echo "RESULT regtest_toml_backup=${backup_path}"
EOS
} 2>&1 | tee "$CONFIG_LOG"

REGTEST_TOML_PATH="$(extract_result_value "regtest_toml" "$CONFIG_LOG")"
REGTEST_TOML_BACKUP_PATH="$(extract_result_value "regtest_toml_backup" "$CONFIG_LOG")"

log "Step E.2: verify deployed code for updated addresses"
if is_true "$REGTEST_RUN_STEP_E2_VERIFY"; then
  {
    CODE_VERIFY_LABELS=(fake_peg_manager peg_manager signature_manager committee_registry member_registry stream_manager)
    CODE_VERIFY_ADDRESSES=("$FAKE_PEG_MANAGER_ADDRESS" "$PEG_MANAGER_ADDRESS" "$SIGNATURE_MANAGER_ADDRESS" "$COMMITTEE_REGISTRY_ADDRESS" "$MEMBER_REGISTRY_ADDRESS" "$STREAM_MANAGER_ADDRESS")
    for idx in "${!CODE_VERIFY_LABELS[@]}"; do
      assert_code_exists_on_node "${CODE_VERIFY_LABELS[$idx]}" "${CODE_VERIFY_ADDRESSES[$idx]}"
    done
  } 2>&1 | tee "$VERIFY_LOG"
else
  log "Step E.2: skipped (set REGTEST_RUN_STEP_E2_VERIFY=true to enable)"
  echo "RESULT step_e2_skipped=true" | tee "$VERIFY_LOG"
fi

log "Step E.3: verify PegManager uses native bridge precompile"
{
  remote_cmd="bash -s -- $(shell_quote "$PEG_MANAGER_ADDRESS") $(shell_quote "$REGTEST_RSK_RPC_LOCAL_URL") $(shell_quote "$REGTEST_NATIVE_BRIDGE_ADDRESS")"
  ssh_exec "$REGTEST_NODE_HOST" "$remote_cmd" <<'EOS'
set -euo pipefail
peg_manager="$1"
rpc_url="$2"
expected_bridge="$3"

export PATH="${HOME}/.foundry/bin:${PATH}"
actual_bridge="$(cast call "$peg_manager" "bridge()(address)" --rpc-url "$rpc_url" | tr -d '\r\n')"
echo "RESULT peg_manager_bridge=${actual_bridge}"
echo "RESULT peg_manager_bridge_expected=${expected_bridge}"
EOS
} 2>&1 | tee -a "$VERIFY_LOG"

PEG_MANAGER_BRIDGE="$(extract_result_value "peg_manager_bridge" "$VERIFY_LOG")"
if [[ "$(to_lower "$PEG_MANAGER_BRIDGE")" != "$(to_lower "$REGTEST_NATIVE_BRIDGE_ADDRESS")" ]]; then
  die "PegManager bridge mismatch: expected ${REGTEST_NATIVE_BRIDGE_ADDRESS}, got ${PEG_MANAGER_BRIDGE}"
fi

log "Step F: authorize new PegManager in Native Bridge"
{
  remote_cmd="bash -s -- $(shell_quote "$REGTEST_NATIVE_BRIDGE_ADDRESS") $(shell_quote "$PEG_MANAGER_ADDRESS") $(shell_quote "$REGTEST_RSK_RPC_LOCAL_URL") $(shell_quote "$REGTEST_BRIDGE_GAS_PRICE_WEI") $(shell_quote "$REGTEST_BRIDGE_AUTH_PRIVATE_KEY")"
  ssh_exec "$REGTEST_NODE_HOST" "$remote_cmd" <<'EOS'
set -euo pipefail
bridge_address="$1"
peg_manager="$2"
rpc_url="$3"
gas_price="$4"
auth_private_key="$5"

export PATH="${HOME}/.foundry/bin:${PATH}"
cast send "$bridge_address" \
  "setUnionBridgeContractAddressForTestnet(address)" "$peg_manager" \
  --rpc-url "$rpc_url" \
  --legacy \
  --value 0 \
  --gas-limit 500100 \
  --gas-price "$gas_price" \
  --private-key "$auth_private_key" >/dev/null

authorized="$(cast call "$bridge_address" "getUnionBridgeContractAddress()(address)" --rpc-url "$rpc_url" | tr -d '\r\n')"
echo "RESULT authorized_peg_manager=${authorized}"
EOS
} 2>&1 | tee "$BRIDGE_LOG"

AUTHORIZED_PEG_MANAGER="$(extract_result_value "authorized_peg_manager" "$BRIDGE_LOG")"
if [[ "$(to_lower "$AUTHORIZED_PEG_MANAGER")" != "$(to_lower "$PEG_MANAGER_ADDRESS")" ]]; then
  die "Native Bridge authorization mismatch: expected ${PEG_MANAGER_ADDRESS}, got ${AUTHORIZED_PEG_MANAGER}"
fi

log "Step G: restart regtest operators with --fresh"
{
  remote_cmd="bash -s -- $(shell_quote "$REGTEST_UNION_REPO_ROOT") $(shell_quote "$REGTEST_USER_BITCOIN_WIF") $(shell_quote "$REGTEST_CHECK_FORK_GUEST_ELF_PATH") $(shell_quote "$REGTEST_RESTART_OP1_DISPATCHER") $(shell_quote "$REGTEST_OP1_DISPATCHER_BIN") $(shell_quote "$REGTEST_OP1_DISPATCHER_CWD") $(shell_quote "$REGTEST_OP1_DISPATCHER_LOG") $(shell_quote "$REGTEST_OP1_DISPATCHER_IP") $(shell_quote "$REGTEST_OP1_DISPATCHER_PORT")"
  ssh_exec "$REGTEST_UNION_HOST" "$remote_cmd" <<'EOS'
set -euo pipefail

union_repo_root="$1"
user_bitcoin_wif="$2"
dispatcher_elf_path="$3"
restart_dispatcher="$4"
dispatcher_bin="$5"
dispatcher_cwd="$6"
dispatcher_log="$7"
dispatcher_ip="$8"
dispatcher_port="$9"

operator_root="${union_repo_root}/docker/operator"
if [[ ! -f "${operator_root}/start_operators.sh" ]]; then
  echo "Missing operator script: ${operator_root}/start_operators.sh" >&2
  exit 1
fi

if [[ ! -f "${dispatcher_elf_path}" ]]; then
  echo "Missing check-fork guest ELF path for host dispatcher: ${dispatcher_elf_path}" >&2
  exit 1
fi

cd "$operator_root"
UB_CHECK_FORK_GUEST_ELF_PATH="$dispatcher_elf_path" USER_BITCOIN_WIF="$user_bitcoin_wif" bash start_operators.sh --env regtest --fresh up -d

timeout_secs=240
deadline=$((SECONDS + timeout_secs))
while (( SECONDS < deadline )); do
  coordinator_count="$(docker ps --format '{{.Names}}' | grep -Ec '^op_[1-4]-coordinator-1$' || true)"
  if [[ "$coordinator_count" -eq 4 ]]; then
    echo "RESULT coordinator_count=${coordinator_count}"
    break
  fi
  sleep 5
done

coordinator_count="$(docker ps --format '{{.Names}}' | grep -Ec '^op_[1-4]-coordinator-1$' || true)"
if [[ "$coordinator_count" -ne 4 ]]; then
  echo "Timed out waiting for 4 coordinator containers" >&2
  exit 1
fi

if [[ "${restart_dispatcher}" =~ ^(1|true|yes|y|on)$ ]]; then
  if [[ ! -x "${dispatcher_bin}" ]]; then
    echo "Missing dispatcher binary: ${dispatcher_bin}" >&2
    exit 1
  fi

  if [[ ! -d "${dispatcher_cwd}" ]]; then
    echo "Missing dispatcher cwd: ${dispatcher_cwd}" >&2
    exit 1
  fi

  pkill -f "bitvmx-risczero-dispatcher --ip ${dispatcher_ip} --port ${dispatcher_port}" || true
  sleep 1

  cd "${dispatcher_cwd}"
  nohup "${dispatcher_bin}" --ip "${dispatcher_ip}" --port "${dispatcher_port}" >> "${dispatcher_log}" 2>&1 &
  sleep 2

  dispatcher_pid="$(pgrep -f "bitvmx-risczero-dispatcher --ip ${dispatcher_ip} --port ${dispatcher_port}" | head -n1 || true)"
  if [[ -z "${dispatcher_pid}" ]]; then
    echo "Failed to start op1 dispatcher on ${dispatcher_ip}:${dispatcher_port}" >&2
    exit 1
  fi

  echo "RESULT dispatcher_pid=${dispatcher_pid}"
  echo "RESULT dispatcher_bin=${dispatcher_bin}"
  echo "RESULT dispatcher_cwd=${dispatcher_cwd}"
  echo "RESULT dispatcher_log=${dispatcher_log}"
  echo "RESULT dispatcher_elf_path=${dispatcher_elf_path}"
fi
EOS
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
  --arg deploy_script_patch_applied "${DEPLOY_SCRIPT_PATCH_APPLIED:-}" \
  --arg fake_peg_manager "$FAKE_PEG_MANAGER_ADDRESS" \
  --arg peg_manager "$PEG_MANAGER_ADDRESS" \
  --arg signature_manager "$SIGNATURE_MANAGER_ADDRESS" \
  --arg committee_registry "$COMMITTEE_REGISTRY_ADDRESS" \
  --arg member_registry "$MEMBER_REGISTRY_ADDRESS" \
  --arg stream_manager "$STREAM_MANAGER_ADDRESS" \
  --arg bridge_address "$REGTEST_NATIVE_BRIDGE_ADDRESS" \
  --arg authorized_peg_manager "$AUTHORIZED_PEG_MANAGER" \
  --arg peg_manager_bridge "$PEG_MANAGER_BRIDGE" \
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
      balance_wei: $deployer_balance_wei,
      deploy_script_patch_applied: $deploy_script_patch_applied
    },
    contracts: {
      fake_peg_manager: $fake_peg_manager,
      peg_manager: $peg_manager,
      signature_manager: $signature_manager,
      committee_registry: $committee_registry,
      member_registry: $member_registry,
      stream_manager: $stream_manager
    },
    bridge: {
      native_bridge_address: $bridge_address,
      authorized_peg_manager: $authorized_peg_manager,
      peg_manager_bridge: $peg_manager_bridge
    },
    config: {
      regtest_toml_path: $regtest_toml,
      regtest_toml_backup: $regtest_toml_backup
    },
    operators: {
      coordinator_count: ($coordinator_count | tonumber)
    },
    artifacts: {
      preflight_log: "preflight.log",
      wallets_log: "wallets.log",
      deploy_log: "deploy.log",
      config_log: "config.log",
      verify_log: "verify.log",
      bridge_log: "bridge.log",
      operators_log: "operators.log",
      deploy_log_path_on_node: $deploy_log_path_on_node
    }
  }' > "$SUMMARY_JSON"

ln -sfn "$RUN_DIR" "${ARTIFACT_BASE}/latest"

ok "Regtest fresh run completed"
ok "Summary: ${SUMMARY_JSON}"
ok "Latest: ${ARTIFACT_BASE}/latest"
