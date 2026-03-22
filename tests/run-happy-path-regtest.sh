#!/usr/bin/env bash

# Regtest-only happy path for running inside the regtest AWS instance.
# Expected to run on union-bridge-use2-1 with dockerized operators and powpeg bitcoind.
# Can be executed from local machine - will SSH to regtest instance automatically.

set -euo pipefail

# Remote execution: SSH to regtest instance unless already there
REGTEST_HOST="union-bridge-use2-1.regtest.rskcomputing.net"
REGTEST_USER="ubuntu"
REGTEST_ROOT="union-bridge-client"

if [[ "${REGTEST_REMOTE:-}" != "1" ]]; then
    echo -e "\033[0;34m[INFO]\033[0m Connecting to regtest instance: ${REGTEST_HOST}"
    exec ssh -A "${REGTEST_USER}@${REGTEST_HOST}" \
        "cd ~/${REGTEST_ROOT} && REGTEST_REMOTE=1 bash tests/run-happy-path-regtest.sh"
fi

# Load NUM_OPERATORS: --ops flag > .env.regtest > default (4)
NUM_OPERATORS=""
_remaining_args=()
for arg in "$@"; do
    if [[ "$_prev_arg" == "--ops" ]]; then
        NUM_OPERATORS="$arg"
        _prev_arg=""
        continue
    fi
    if [[ "$arg" == "--ops" ]]; then
        _prev_arg="--ops"
        continue
    fi
    _remaining_args+=("$arg")
done
unset _prev_arg
set -- "${_remaining_args[@]}"
unset _remaining_args

if [[ -z "$NUM_OPERATORS" ]]; then
    _env_file="docker/operator/.env.regtest"
    if [[ -f "$_env_file" ]]; then
        _num=$(grep -E '^\s*NUM_OPERATORS=' "$_env_file" | tail -1 | cut -d= -f2 | tr -d ' "'\''')
        if [[ -n "$_num" ]] && [[ "$_num" =~ ^[0-9]+$ ]] && [[ "$_num" -ge 1 ]] && [[ "$_num" -le 10 ]]; then
            NUM_OPERATORS="$_num"
        fi
    fi
    unset _env_file _num
fi
NUM_OPERATORS="${NUM_OPERATORS:-4}"

if ! [[ "$NUM_OPERATORS" =~ ^(10|[1-9])$ ]]; then
    echo "Error: --ops must be between 1 and 10"
    exit 1
fi

# Verify operators are deployed
check_operators_deployed() {
    local missing=0
    for op_id in $(seq 1 "$NUM_OPERATORS"); do
        if ! docker ps --format "{{.Names}}" | grep -q "op_${op_id}-coordinator-1"; then
            missing=$((missing + 1))
        fi
    done
    if [[ $missing -gt 0 ]]; then
        echo -e "\033[1;33m[!]\033[0m Operators not fully deployed ($(($NUM_OPERATORS - missing))/$NUM_OPERATORS running)"
        echo "    Start them with: ./cli-infra.sh --start-regtest"
        exit 1
    fi
}
check_operators_deployed

SCRIPT_ENV="regtest"
REGTEST_FRESH_ENV_FILE="${REGTEST_FRESH_ENV_FILE:-${HOME}/regtest-fresh/.env}"
REGTEST_FRESH_RUNS_DIR="${REGTEST_FRESH_RUNS_DIR:-${HOME}/.union-bridge/regtest-fresh/runs}"

RSK_RPC_URL="${RSK_RPC_URL:-http://node-use2-1.regtest.rskcomputing.net:4444}"
USER_API_HOST="${USER_API_HOST:-localhost}"
BITCOIN_RPC_HOST="${BITCOIN_RPC_HOST:-10.1.0.107}"
BITCOIN_RPC_PORT="${BITCOIN_RPC_PORT:-18332}"
BITCOIN_RPC_USER="${BITCOIN_RPC_USER:-user}"
BITCOIN_RPC_PASSWORD="${BITCOIN_RPC_PASSWORD:-pass}"
BITCOIN_WALLET_NAME="${BITCOIN_WALLET_NAME:-mainwallet}"
BITCOIN_FUNDING_WALLET_NAME="${BITCOIN_FUNDING_WALLET_NAME:-test_wallet}"
BITVMX_WALLET_NAME="${BITVMX_WALLET_NAME:-test_wallet_watch}"
RESTART_BITVMX_ON_WALLET_CREATE="${RESTART_BITVMX_ON_WALLET_CREATE:-true}"

BITVMX_FUND_AMOUNT="${BITVMX_FUND_AMOUNT:-32002000}"
RSK_FUND_AMOUNT_WEI="${RSK_FUND_AMOUNT_WEI:-0x3782dace9d90000}"
RSK_GAS_PRICE_WEI="${RSK_GAS_PRICE_WEI:-0x3938700}"

STREAM_ID="${STREAM_ID:-0}"
VALUE="${VALUE:-100000}"
RSK_ADDRESS="${RSK_ADDRESS:-0x$(openssl rand -hex 20)}"

LOG_SINCE="${LOG_SINCE:-30m}"
LOG_TAIL_LINES="${LOG_TAIL_LINES:-400}"
MAX_BLOCKS_WAIT="${MAX_BLOCKS_WAIT:-30}"
COMMITTEE_SETUP_MAX_BLOCKS="${COMMITTEE_SETUP_MAX_BLOCKS:-20}"
PEGIN_MAX_BLOCKS="${PEGIN_MAX_BLOCKS:-40}"
PEGOUT_MAX_BLOCKS="${PEGOUT_MAX_BLOCKS:-40}"
COMMITTEE_LOG_STRICT="${COMMITTEE_LOG_STRICT:-false}"

GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m'

log() { echo -e "${BLUE}[INFO]${NC} $1"; }
success() { echo -e "${GREEN}[OK]${NC} $1"; }
warn() { echo -e "${YELLOW}[WARN]${NC} $1"; }
die() { echo "Error: $1" >&2; exit 1; }
step() {
    echo ""
    echo -e "${GREEN}========== $1 ==========${NC}"
    echo ""
}

require_cmd() {
    command -v "$1" >/dev/null 2>&1 || die "Missing required command: $1"
}

is_true() {
    case "${1,,}" in
        1|true|yes|y|on) return 0 ;;
        *) return 1 ;;
    esac
}

docker_container_status() {
    local container_name="$1"
    docker inspect -f '{{if .State.Health}}{{.State.Health.Status}}{{else}}{{.State.Status}}{{end}}' "$container_name" 2>/dev/null || true
}

wait_for_container_status() {
    local container_name="$1"
    local expected_status="$2"
    local timeout_secs="${3:-60}"
    local deadline=$((SECONDS + timeout_secs))

    while (( SECONDS < deadline )); do
        if [[ "$(docker_container_status "$container_name")" == "$expected_status" ]]; then
            return 0
        fi
        sleep 2
    done

    return 1
}

wait_for_container_log_pattern() {
    local container_name="$1"
    local pattern="$2"
    local since_timestamp="$3"
    local timeout_secs="${4:-60}"
    local deadline=$((SECONDS + timeout_secs))

    while (( SECONDS < deadline )); do
        if docker logs --since "$since_timestamp" "$container_name" 2>/dev/null | grep -E -q "$pattern"; then
            return 0
        fi
        sleep 2
    done

    return 1
}

latest_regtest_summary_path() {
    [[ -d "$REGTEST_FRESH_RUNS_DIR" ]] || die "Missing regtest fresh runs dir: $REGTEST_FRESH_RUNS_DIR"

    local latest_run
    latest_run=$(ls -1t "$REGTEST_FRESH_RUNS_DIR" 2>/dev/null | head -1)
    [[ -n "$latest_run" ]] || die "Could not resolve latest regtest fresh run under $REGTEST_FRESH_RUNS_DIR"

    local summary_path="${REGTEST_FRESH_RUNS_DIR}/${latest_run}/summary.json"
    [[ -f "$summary_path" ]] || die "Missing regtest fresh summary: $summary_path"
    printf '%s\n' "$summary_path"
}

resolve_current_committee_registry_address() {
    local summary_path="$1"
    local address
    address=$(jq -r '.contracts.committee_registry // empty' "$summary_path")
    [[ -n "$address" && "$address" != "null" ]] || die "Could not resolve CommitteeRegistry address from $summary_path"
    printf '%s\n' "$address"
}

resolve_current_whitelister_private_key() {
    [[ -f "$REGTEST_FRESH_ENV_FILE" ]] || die "Missing regtest fresh env file: $REGTEST_FRESH_ENV_FILE"
    command -v cast >/dev/null 2>&1 || die "Missing required command: cast"

    local private_key
    # The current CommitteeRegistry whitelister is the deployer derived from the same mnemonic used by regtest_fresh.
    private_key=$(
        (
            set -a
            # shellcheck disable=SC1090
            source "$REGTEST_FRESH_ENV_FILE"
            set +a
            [[ -n "${REGTEST_DEPLOY_MNEMONIC:-}" ]] || exit 1
            cast wallet private-key --mnemonic "$REGTEST_DEPLOY_MNEMONIC" --mnemonic-index 0
        ) | tr -d '\r\n[:space:]'
    ) || die "Could not derive whitelister private key from $REGTEST_FRESH_ENV_FILE"

    [[ -n "$private_key" ]] || die "Derived empty whitelister private key"
    printf '%s\n' "$private_key"
}

bitcoin_cli_base() {
    local method="$1"
    shift
    bitcoin_rpc_call "" "$method" "$@"
}

bitcoin_cli_wallet() {
    local wallet_name="$1"
    shift
    local method="$1"
    shift
    bitcoin_rpc_call "$wallet_name" "$method" "$@"
}

bitcoin_rpc_call() {
    local wallet_name="$1"
    shift
    local method="$1"
    shift
    local params
    params=$(bitcoin_rpc_build_params "$@")

    local url="http://${BITCOIN_RPC_HOST}:${BITCOIN_RPC_PORT}"
    if [[ -n "$wallet_name" ]]; then
        url="${url}/wallet/${wallet_name}"
    fi

    local payload
    payload=$(printf '{"jsonrpc":"1.0","id":"union","method":"%s","params":%s}' "$method" "$params")

    local response
    if ! response=$(curl -sS --user "${BITCOIN_RPC_USER}:${BITCOIN_RPC_PASSWORD}" \
        -H "content-type: text/plain;" \
        --data-binary "$payload" \
        "$url"); then
        return 1
    fi

    local err
    if ! err=$(echo "$response" | jq -r '.error.message // empty' 2>/dev/null); then
        echo "Bitcoin RPC unexpected response: $response" >&2
        return 1
    fi
    if [[ -n "$err" ]]; then
        echo "Bitcoin RPC error: $err" >&2
        return 1
    fi

    echo "$response" | jq -cr '.result'
}

bitcoin_rpc_build_params() {
    local args=("$@")
    local params="["
    local first=true
    local arg json_arg

    for arg in "${args[@]}"; do
        if [[ "$arg" =~ ^\{.*\}$ || "$arg" =~ ^\[.*\]$ ]]; then
            json_arg="$arg"
        elif [[ "$arg" == "true" || "$arg" == "false" ]]; then
            json_arg="$arg"
        elif [[ "$arg" =~ ^-?[0-9]+(\.[0-9]+)?$ ]]; then
            json_arg="$arg"
        else
            json_arg=$(printf '%s' "$arg" | jq -Rs .)
        fi

        if [[ "$first" == "true" ]]; then
            first=false
        else
            params+=","
        fi
        params+="$json_arg"
    done

    params+="]"
    echo "$params"
}

ensure_bitcoind_wallet() {
    local wallet_name="$1"
    local wallets
    wallets=$(bitcoin_cli_base listwallets | jq -r '.[]' 2>/dev/null || true)
    if grep -qx "$wallet_name" <<< "$wallets"; then
        return 1
    fi

    if bitcoin_cli_base createwallet "$wallet_name" >/dev/null 2>&1; then
        return 0
    fi

    if bitcoin_cli_base loadwallet "$wallet_name" >/dev/null 2>&1; then
        return 0
    fi

    die "Failed to load bitcoind wallet: $wallet_name"
}

ensure_bitcoind_watch_wallet() {
    local wallet_name="$1"
    local wallets
    wallets=$(bitcoin_cli_base listwallets | jq -r '.[]' 2>/dev/null || true)
    if grep -qx "$wallet_name" <<< "$wallets"; then
        return 1
    fi

    if bitcoin_cli_base createwallet "$wallet_name" true true "" false true >/dev/null 2>&1; then
        return 0
    fi

    if bitcoin_cli_base loadwallet "$wallet_name" >/dev/null 2>&1; then
        return 0
    fi

    die "Failed to load bitcoind watch wallet: $wallet_name"
}

ensure_user_bitcoin_wif() {
    if [[ -n "${USER_BITCOIN_WIF:-}" ]]; then
        return 0
    fi

    local user_api_container
    user_api_container=$(docker ps --format "{{.Names}}" | grep -m1 "user-api" || true)
    if [[ -z "$user_api_container" ]]; then
        die "USER_BITCOIN_WIF not set and user-api container not found"
    fi

    USER_BITCOIN_WIF=$(docker exec "$user_api_container" printenv USER_BITCOIN_WIF || true)
    if [[ -z "$USER_BITCOIN_WIF" ]]; then
        die "USER_BITCOIN_WIF not available in user-api container"
    fi

    export USER_BITCOIN_WIF
}

import_user_wif() {
    # First try legacy import (works on legacy/non-descriptor wallets).
    if bitcoin_cli_wallet "$BITCOIN_WALLET_NAME" importprivkey "$USER_BITCOIN_WIF" "user" false >/dev/null 2>&1; then
        return 0
    fi

    local desc import_req
    desc=$(user_descriptor_from_wif)
    import_req=$(printf '[{"desc":"%s","timestamp":"now","active":true,"label":"user"}]' "$desc")
    if ! bitcoin_cli_wallet "$BITCOIN_WALLET_NAME" importdescriptors "$import_req" >/dev/null 2>&1; then
        warn "Failed to import USER_BITCOIN_WIF into descriptor wallet (${BITCOIN_WALLET_NAME}); continuing"
    fi
}

rescan_user_wif_history() {
    # Try both legacy and descriptor flows; ignore failures from unsupported wallet types.
    bitcoin_cli_wallet "$BITCOIN_WALLET_NAME" importprivkey "$USER_BITCOIN_WIF" "user-rescan" true >/dev/null 2>&1 || true

    local desc import_req
    desc=$(user_descriptor_from_wif)
    if [[ -n "$desc" ]]; then
        import_req=$(printf '[{"desc":"%s","timestamp":0,"active":true,"label":"user"}]' "$desc")
        bitcoin_cli_wallet "$BITCOIN_WALLET_NAME" importdescriptors "$import_req" >/dev/null 2>&1 || true
    fi

    bitcoin_cli_wallet "$BITCOIN_WALLET_NAME" rescanblockchain 0 >/dev/null 2>&1 || true
}

import_watch_address_into_wallet() {
    local wallet_name="$1"
    local addr="$2"

    local descriptor_info desc import_req import_response
    descriptor_info=$(bitcoin_cli_base getdescriptorinfo "addr(${addr})")
    desc=$(echo "$descriptor_info" | jq -r '.descriptor // empty')
    if [[ -z "$desc" || "$desc" == "null" ]]; then
        die "Failed to compute descriptor for addr(${addr})"
    fi

    import_req=$(printf '[{"desc":"%s","timestamp":"now","active":false,"label":"watch"}]' "$desc")
    import_response=$(bitcoin_cli_wallet "$wallet_name" importdescriptors "$import_req") || {
        die "Failed to import watch address into wallet ${wallet_name}: ${addr}"
    }

    if ! jq -e 'all(.success or ((.error.message // "") | test("already|exists|active"; "i")))' >/dev/null 2>&1 <<< "$import_response"; then
        die "Watch address import was rejected by wallet ${wallet_name}: ${import_response}"
    fi
}

user_descriptor_from_wif() {
    local desc
    desc=$(bitcoin_cli_base getdescriptorinfo "wpkh(${USER_BITCOIN_WIF})" | jq -r '.descriptor')
    if [[ -z "$desc" || "$desc" == "null" ]]; then
        die "Failed to derive descriptor from USER_BITCOIN_WIF"
    fi
    echo "$desc"
}

user_address_from_wif() {
    local desc
    desc=$(user_descriptor_from_wif)
    local addr
    addr=$(bitcoin_cli_base deriveaddresses "$desc" | jq -r '.[0]')
    if [[ -z "$addr" || "$addr" == "null" ]]; then
        die "Failed to derive user address from descriptor"
    fi
    echo "$addr"
}

user_xonly_pubkey_from_wif() {
    local desc pubkey xonly
    desc=$(user_descriptor_from_wif)
    pubkey=$(echo "$desc" | sed -n 's/^wpkh(\([0-9a-fA-F]\{66\}\)).*/\1/p')
    if [[ -z "$pubkey" ]]; then
        die "Failed to extract pubkey from descriptor"
    fi
    xonly="${pubkey:2}"
    if [[ ${#xonly} -ne 64 ]]; then
        die "Unexpected x-only pubkey length: ${#xonly}"
    fi
    echo "$xonly"
}

user_compressed_pubkey_from_wif() {
    local desc pubkey
    desc=$(user_descriptor_from_wif)
    pubkey=$(echo "$desc" | sed -n 's/^wpkh(\([0-9a-fA-F]\{66\}\)).*/\1/p')
    if [[ -z "$pubkey" ]]; then
        die "Failed to extract compressed pubkey from descriptor"
    fi
    echo "$pubkey"
}

sats_to_btc() {
    awk -v sats="$1" 'BEGIN { printf "%.8f", sats / 100000000 }'
}

curl_json_post() {
    local url="$1"
    local payload="$2"
    local tmp_file
    tmp_file=$(mktemp)
    local status
    status=$(curl -sS -o "$tmp_file" -w "%{http_code}" -H "Content-Type: application/json" -d "$payload" "$url") || {
        rm -f "$tmp_file"
        return 1
    }
    local body
    body=$(cat "$tmp_file")
    rm -f "$tmp_file"
    if [[ "$status" != 2* ]]; then
        echo "Error: ${url} returned ${status}: ${body}" >&2
        return 1
    fi
    echo "$body"
}

rsk_rpc() {
    local method="$1"
    local params="$2"
    local payload
    payload=$(printf '{"jsonrpc":"2.0","method":"%s","params":%s,"id":1}' "$method" "$params")
    curl -sS -H "Content-Type: application/json" --data "$payload" "$RSK_RPC_URL"
}

rsk_bridge_btc_best_height() {
    # eth_bridgeState returns an object including btcBlockchainBestChainHeight.
    # If this stays at 0 while bitcoind advances, the native bridge is not being fed headers
    # (e.g., powpeg/federator not running or pointing at a different Bitcoin chain).
    local response height
    response=$(rsk_rpc "eth_bridgeState" "[]")
    height=$(echo "$response" | jq -r '.result.btcBlockchainBestChainHeight // 0' 2>/dev/null || echo "0")
    # Ensure it's a number
    if [[ ! "$height" =~ ^-?[0-9]+$ ]]; then
        echo 0
        return
    fi
    echo "$height"
}

rsk_send_value() {
    local from_addr="$1"
    local to_addr="$2"
    local payload
    payload=$(printf '[{"from":"%s","to":"%s","value":"%s","gas":"0x5208","gasPrice":"%s"}]' \
        "$from_addr" "$to_addr" "$RSK_FUND_AMOUNT_WEI" "$RSK_GAS_PRICE_WEI")
    local response
    response=$(rsk_rpc "eth_sendTransaction" "$payload")
    local err
    err=$(echo "$response" | jq -r '.error.message // empty')
    if [[ -n "$err" ]]; then
        die "RSK funding failed: $err"
    fi
}

extract_rsk_addresses_from_logs() {
    local marker="$1"
    awk -v marker="$marker" '
        index($0, marker) {
            for (i = 1; i <= NF; i++) {
                if ($i ~ /^0x[0-9a-fA-F]{40}/) {
                    gsub(/[;,.\r]/, "", $i)
                    print $i
                }
            }
        }
    '
}

extract_bitvmx_address_from_logs() {
    awk -v marker="Received BitVMX Funding Address:" '
        index($0, marker) {
            line = substr($0, index($0, marker) + length(marker))
            n = split(line, parts, /[[:space:]]+/)
            for (i = 1; i <= n; i++) {
                if (parts[i] != "") {
                    # Remove derivation path suffix like /0, /1 etc.
                    sub(/\/[0-9]+$/, "", parts[i])
                    gsub(/[^[:alnum:]]/, "", parts[i])
                    print parts[i]
                    break
                }
            }
        }
    '
}

collect_rsk_addresses() {
    local marker="$1"
    local container_suffix="$2"
    local all=""
    for op_id in $(seq 1 "$NUM_OPERATORS"); do
        local container="op_${op_id}-${container_suffix}-1"
        local logs
        logs=$(docker logs "$container" 2>/dev/null || true)
        all+=$(extract_rsk_addresses_from_logs "$marker" <<< "$logs")
        all+=$'\n'
    done
    echo "$all" | sort -u | sed '/^$/d'
}

collect_bitvmx_addresses() {
    local addresses=()
    for op_id in $(seq 1 "$NUM_OPERATORS"); do
        local container="op_${op_id}-coordinator-1"
        local logs
        logs=$(docker logs --since "$LOG_SINCE" "$container" 2>/dev/null || true)
        local addr
        addr=$(extract_bitvmx_address_from_logs <<< "$logs" | tail -1 || true)
        if [[ -n "$addr" ]]; then
            addresses+=("$addr")
        fi
    done
    printf "%s\n" "${addresses[@]}" | sort -u
}

user_api_endpoints() {
    local host="$USER_API_HOST"
    local ports=""
    for i in $(seq 1 "$NUM_OPERATORS"); do
        ports+="${host}:$((40000 + i)) "
    done
    echo "$ports"
}

mine_blocks_to_address() {
    local count="$1"
    local addr="$2"
    bitcoin_cli_wallet "$BITCOIN_WALLET_NAME" generatetoaddress "$count" "$addr" >/dev/null
}

wallet_confirmed_balance_sats() {
    local balance
    balance=$(bitcoin_cli_wallet "$BITCOIN_WALLET_NAME" getbalances | jq -r '.mine.trusted // 0')
    if [[ -z "$balance" || "$balance" == "null" ]]; then
        echo 0
        return
    fi
    awk -v btc="$balance" 'BEGIN { printf "%.0f", btc * 100000000 }'
}

mine_blocks() {
    local count="$1"
    local miner_addr
    miner_addr=$(bitcoin_cli_wallet "$BITCOIN_WALLET_NAME" getnewaddress "miner" "bech32")
    mine_blocks_to_address "$count" "$miner_addr"
}

get_current_bitcoin_height() {
    local height
    height=$(bitcoin_cli_base getblockcount 2>/dev/null || echo "0")
    height=${height:-0}
    echo "$height"
}

find_recent_coordinator_log_match() {
    local pattern="$1"
    for op_id in $(seq 1 "$NUM_OPERATORS"); do
        local container="op_${op_id}-coordinator-1"
        if ! docker ps --format "{{.Names}}" | grep -qx "$container"; then
            continue
        fi
        local line
        line=$(docker logs --since "$LOG_SINCE" --tail "$LOG_TAIL_LINES" "$container" 2>/dev/null | grep -E "$pattern" | tail -1 || true)
        if [[ -n "$line" ]]; then
            echo "${container}:${line}"
            return 0
        fi
    done
    return 1
}

wait_for_log_with_block_timeout() {
    local pattern="$1"
    local max_blocks="$2"
    local start_height
    start_height=$(get_current_bitcoin_height)
    local target_height=$((start_height + max_blocks))

    log "Waiting for log pattern: $pattern (max $max_blocks blocks)..."

    while true; do
        local current_height
        current_height=$(get_current_bitcoin_height)
        local blocks_mined=$((current_height - start_height))

        if ((blocks_mined < 0)); then
            sleep 1
            continue
        fi

        echo -ne "\r  Blocks mined: $blocks_mined/$max_blocks | Checking logs...  "

        local result=""
        result=$(find_recent_coordinator_log_match "$pattern" || true)
        if [[ -n "$result" ]]; then
            local found_source="${result%%:*}"
            local found_line="${result#*:}"
            echo ""
            success "Log pattern found after $blocks_mined blocks!"
            log "Found in: $found_source"
            echo "$found_line"
            return 0
        fi

        if ((current_height >= target_height)); then
            echo ""
            warn "Log pattern not found after $max_blocks blocks (height: $start_height -> $current_height)"
            return 1
        fi

        sleep 2
    done
}

wait_for_any_log_with_block_timeout() {
    local max_blocks="${@: -1}"
    local patterns=("${@:1:$#-1}")
    local combined
    combined=$(printf "%s|" "${patterns[@]}")
    combined="${combined%|}"

    local start_height
    start_height=$(get_current_bitcoin_height)
    local target_height=$((start_height + max_blocks))

    log "Waiting for log patterns: ${combined} (max ${max_blocks} blocks)..."

    while true; do
        local current_height
        current_height=$(get_current_bitcoin_height)
        local blocks_mined=$((current_height - start_height))

        if ((blocks_mined < 0)); then
            sleep 1
            continue
        fi

        echo -ne "\r  Blocks mined: $blocks_mined/$max_blocks | Checking logs...  "

        local result=""
        result=$(find_recent_coordinator_log_match "$combined" || true)
        if [[ -n "$result" ]]; then
            local found_source="${result%%:*}"
            local found_line="${result#*:}"
            echo ""
            success "Log pattern found after $blocks_mined blocks!"
            log "Found in: $found_source"
            echo "$found_line"
            return 0
        fi

        if ((current_height >= target_height)); then
            echo ""
            warn "Log patterns not found after $max_blocks blocks (height: $start_height -> $current_height)"
            return 1
        fi

        sleep 1
    done
}

dump_recent_logs() {
    local label="$1"
    log "Recent logs (${label})"
    for op_id in $(seq 1 "$NUM_OPERATORS"); do
        local container="op_${op_id}-${label}-1"
        if docker ps --format "{{.Names}}" | grep -qx "$container"; then
            echo "----- ${container} -----"
            docker logs --since "$LOG_SINCE" --tail "$LOG_TAIL_LINES" "$container" 2>/dev/null || true
        fi
    done
}

fund_rsk_wallets() {
    local member_marker="Got member signer with address"
    local user_marker="Got user signer with address"

    mapfile -t member_addrs < <(collect_rsk_addresses "$member_marker" "coordinator")
    mapfile -t user_addrs < <(collect_rsk_addresses "$user_marker" "user-api")

    if [[ "${#member_addrs[@]}" -lt "$NUM_OPERATORS" || "${#user_addrs[@]}" -lt "$NUM_OPERATORS" ]]; then
        die "Missing RSK addresses (member=${#member_addrs[@]}, user=${#user_addrs[@]}, expected=${NUM_OPERATORS})"
    fi

    local funder
    funder=$(rsk_rpc "eth_accounts" "[]" | jq -r '.result[0]')
    if [[ -z "$funder" || "$funder" == "null" ]]; then
        die "Failed to read RSK funder account"
    fi

    for addr in "${member_addrs[@]}"; do
        log "Funding member RSK address: $addr"
        rsk_send_value "$funder" "$addr" >/dev/null
    done

    for addr in "${user_addrs[@]}"; do
        log "Funding user RSK address: $addr"
        rsk_send_value "$funder" "$addr" >/dev/null
    done
}

restart_bitvmx_and_coordinators() {
    local reason="$1"
    log "Restarting BitVMX and coordinator containers to ${reason}"

    for op_id in $(seq 1 "$NUM_OPERATORS"); do
        local bitvmx_container="op_${op_id}-bitvmx-client-1"
        local coordinator_container="op_${op_id}-coordinator-1"
        local bitvmx_restart_since
        bitvmx_restart_since=$(date -u +"%Y-%m-%dT%H:%M:%SZ")

        docker restart "$bitvmx_container" >/dev/null
        wait_for_container_status "$bitvmx_container" "healthy" 90 || \
            die "BitVMX container did not become healthy after restart: ${bitvmx_container}"
        wait_for_container_log_pattern \
            "$bitvmx_container" \
            "Sync complete, starting normal operation" \
            "$bitvmx_restart_since" \
            90 || die "BitVMX container did not finish startup after restart: ${bitvmx_container}"

        docker restart "$coordinator_container" >/dev/null
        wait_for_container_status "$coordinator_container" "healthy" 90 || \
            die "Coordinator container did not become healthy after restart: ${coordinator_container}"
    done
}

fund_bitvmx_wallets() {
    local endpoints
    endpoints=($(user_api_endpoints))

    for endpoint in "${endpoints[@]}"; do
        local url="http://${endpoint}/member/bitvmx-address"
        local status
        status=$(curl -sS -o /dev/null -w "%{http_code}" "$url" || true)
        if [[ "$status" != 2* ]]; then
            die "Failed to trigger BitVMX address on ${endpoint} (status=${status})"
        fi
    done

    local deadline=$((SECONDS + 60))
    local bitvmx_addrs=()
    while (( SECONDS < deadline )); do
        mapfile -t bitvmx_addrs < <(collect_bitvmx_addresses)
        if [[ "${#bitvmx_addrs[@]}" -ge "$NUM_OPERATORS" ]]; then
            break
        fi
        sleep 2
    done

    if [[ "${#bitvmx_addrs[@]}" -lt "$NUM_OPERATORS" ]]; then
        die "Missing BitVMX funding addresses (found ${#bitvmx_addrs[@]}, expected=${NUM_OPERATORS})"
    fi

    local amount_btc
    amount_btc=$(sats_to_btc "$BITVMX_FUND_AMOUNT")
    local outputs
    outputs=$(printf '%s\n' "${bitvmx_addrs[@]}" | jq -Rsc --argjson amt "$amount_btc" \
        'split("\n") | map(select(. != "")) | reduce .[] as $addr ({}; .[$addr] = $amt)')
    log "Funding ${#bitvmx_addrs[@]} BitVMX addresses in one transaction"
    # Set a fixed fee rate for regtest (fallback fee not enabled on node)
    bitcoin_cli_wallet "$BITCOIN_WALLET_NAME" settxfee 0.0001 >/dev/null 2>&1 || true
    if ! bitcoin_cli_wallet "$BITCOIN_WALLET_NAME" sendmany "" "$outputs" >/dev/null 2>&1; then
        warn "BitVMX funding tx failed (likely insufficient funds); rescanning wallet and retrying once..."
        rescan_user_wif_history
        bitcoin_cli_wallet "$BITCOIN_WALLET_NAME" sendmany "" "$outputs" >/dev/null
    fi

    mine_blocks 1

    if is_true "$RESTART_BITVMX_ON_WALLET_CREATE"; then
        restart_bitvmx_and_coordinators "refresh wallet balances"
    fi
}

apply_stream() {
    local endpoints
    endpoints=($(user_api_endpoints))
    local roles=()
    for i in $(seq 1 "$NUM_OPERATORS"); do
        if (( i % 2 == 1 )); then roles+=("Prover"); else roles+=("Verifier"); fi
    done

    for i in "${!endpoints[@]}"; do
        local endpoint="${endpoints[$i]}"
        local role="${roles[$i]}"
        local payload
        payload=$(printf '{"ApplyToStream":{"stream_id":%s,"role":"%s","funding_utxo":{"value":10000000},"speed_up_utxo":{"value":10000000},"advance_funds":{"value":10000000}}}' "$STREAM_ID" "$role")
        log "Applying op_$((i + 1)) as ${role}"
        local response
        response=$(curl_json_post "http://${endpoint}/member/apply-stream" "$payload" || true)
        if [[ -z "$response" ]]; then
            die "Apply-stream failed on ${endpoint}"
        fi
        if ! jq -e '.error == null' >/dev/null 2>&1 <<< "$response"; then
            die "Apply-stream error on ${endpoint}: $(jq -r '.error.message // .error' <<< "$response")"
        fi
        sleep 1
    done
}

whitelist_member_addresses() {
    export PATH="${HOME}/.cargo/bin:${HOME}/.foundry/bin:${PATH}"
    command -v cast >/dev/null 2>&1 || die "Missing required command: cast"

    local summary_path
    summary_path=$(latest_regtest_summary_path)

    local committee_registry_address
    committee_registry_address=$(resolve_current_committee_registry_address "$summary_path")

    local whitelister_private_key
    whitelister_private_key=$(resolve_current_whitelister_private_key)
    local whitelister_address
    whitelister_address=$(cast wallet address --private-key "$whitelister_private_key" | tr -d '\r\n[:space:]')
    [[ -n "$whitelister_address" ]] || die "Could not derive whitelister address"

    local member_marker="Got member signer with address"
    mapfile -t member_addrs < <(collect_rsk_addresses "$member_marker" "coordinator")
    if [[ "${#member_addrs[@]}" -lt "$NUM_OPERATORS" ]]; then
        die "Missing member addresses for whitelist (found ${#member_addrs[@]}, expected=${NUM_OPERATORS})"
    fi

    local member_addr_array
    member_addr_array=$(printf '[%s]' "$(IFS=,; echo "${member_addrs[*]}")")
    local nonce
    nonce=$(rsk_rpc "eth_getTransactionCount" "[\"$whitelister_address\",\"latest\"]" | jq -r '.result // empty')
    [[ "$nonce" =~ ^0x[0-9a-fA-F]+$ ]] || die "Could not resolve whitelister nonce"

    log "Whitelisting member addresses on CommitteeRegistry ${committee_registry_address}"
    if ! cast send \
        "$committee_registry_address" \
        'whitelistAddresses(address[])' \
        "$member_addr_array" \
        --legacy \
        --gas-price 1 \
        --gas-limit 1000000 \
        --chain 33 \
        --nonce "$nonce" \
        --async \
        --private-key "$whitelister_private_key" \
        --rpc-url "$RSK_RPC_URL" >/dev/null; then
        die "Whitelist command failed"
    fi
    mine_blocks 1
}

request_pegin_data() {
    local endpoint="http://${USER_API_HOST}:40001/user/pegin-address"
    # Get x-only public key (32 bytes) with 0x prefix
    local xonly_pubkey
    xonly_pubkey="0x$(user_xonly_pubkey_from_wif)"
    local payload
    payload=$(printf '{"rootstock_deposit_address":"%s","value":%s,"btc_reimbursement_pub_key":"%s"}' "$RSK_ADDRESS" "$VALUE" "$xonly_pubkey")
    local response
    response=$(curl_json_post "$endpoint" "$payload")
    local address packet_number enabler_script_pubkey
    address=$(echo "$response" | jq -r '.address')
    packet_number=$(echo "$response" | jq -r '.packet_number')
    enabler_script_pubkey=$(echo "$response" | jq -r '.enabler_script_pubkey')

    if [[ -z "$address" || "$address" == "null" ]]; then
        die "Failed to parse pegin address from user-api response"
    fi
    if [[ -z "$packet_number" || "$packet_number" == "null" ]]; then
        die "Failed to parse packet_number from user-api response"
    fi
    if [[ -z "$enabler_script_pubkey" || "$enabler_script_pubkey" == "null" ]]; then
        die "Failed to parse enabler_script_pubkey from user-api response"
    fi

    jq -cn \
        --arg address "$address" \
        --argjson packet_number "$packet_number" \
        --arg enabler_script_pubkey "$enabler_script_pubkey" \
        '{address: $address, packet_number: $packet_number, enabler_script_pubkey: $enabler_script_pubkey}'
}

create_pegin_tx() {
    local pegin_address="$1"
    local packet_number="$2"
    local enabler_script_pubkey="$3"
    prepare_wallet_cli_for_pegin

    local cli_output
    if ! cli_output=$(
        ./cli-bitcoin-wallet.sh user create_pegin_tx \
            "$VALUE" \
            "$packet_number" \
            "$pegin_address" \
            "$RSK_ADDRESS" \
            "$enabler_script_pubkey" 2>&1
    ); then
        echo "$cli_output" >&2
        die "Failed to create pegin transaction"
    fi

    echo "$cli_output"

    local txid
    txid=$(echo "$cli_output" | sed -n 's/^  txid=//p' | tail -1)
    [[ -n "$txid" ]] || die "Failed to parse pegin txid from cli-bitcoin-wallet output"

    if ! echo "$cli_output" | grep -q "Transaction broadcasted successfully:"; then
        die "cli-bitcoin-wallet did not broadcast the pegin transaction successfully"
    fi

    log "Pegin txid: $txid"
}

prepare_wallet_cli_for_pegin() {
    export PATH="${HOME}/.cargo/bin:${HOME}/.foundry/bin:${PATH}"
    export BASE_STORAGE_PATH="${HOME}"
    export WALLET_RPC_URL="http://${BITCOIN_RPC_HOST}:${BITCOIN_RPC_PORT}/"
    export WALLET_RPC_USER="${BITCOIN_RPC_USER}"
    export WALLET_RPC_PASSWORD="${BITCOIN_RPC_PASSWORD}"

    local user_address pegin_wallet_topup_sats pegin_wallet_topup_btc funding_txid funding_wallet_tx funding_tx block_hash vout
    user_address=$(user_address_from_wif)
    pegin_wallet_topup_sats=$((VALUE + 100000))
    pegin_wallet_topup_btc=$(sats_to_btc "$pegin_wallet_topup_sats")
    log "Funding ${user_address} with ${pegin_wallet_topup_sats} sats for cli-bitcoin-wallet"
    funding_txid=$(bitcoin_cli_wallet "$BITCOIN_WALLET_NAME" sendtoaddress "$user_address" "$pegin_wallet_topup_btc")
    [[ -n "$funding_txid" && "$funding_txid" != "null" ]] || die "Failed to fund user address for cli-bitcoin-wallet"

    mine_blocks 1

    funding_wallet_tx=$(bitcoin_cli_wallet "$BITCOIN_WALLET_NAME" gettransaction "$funding_txid")
    block_hash=$(echo "$funding_wallet_tx" | jq -r '.blockhash // empty')
    [[ -n "$block_hash" && "$block_hash" != "null" ]] || die "Failed to resolve block hash for user funding tx ${funding_txid}"

    funding_tx=$(bitcoin_cli_base getrawtransaction "$funding_txid" true "$block_hash")
    vout=$(echo "$funding_tx" | jq -r --arg addr "$user_address" '.vout | to_entries[] | select(.value.scriptPubKey.address == $addr) | .key' | head -1)

    [[ -n "$vout" && "$vout" != "null" ]] || die "Failed to resolve funding vout for user address ${user_address}"

    ./cli-bitcoin-wallet.sh user clear_db >/dev/null 2>&1 || true
    ./cli-bitcoin-wallet.sh user register_utxo "$funding_txid" "$block_hash" "$vout" "$pegin_wallet_topup_sats" >/dev/null
}

request_pegout() {
    local endpoint="http://${USER_API_HOST}:40001/user/request-pegout"
    local amount_in_wei=$((VALUE * 10000000000))
    # Get compressed public key (33 bytes) with 0x prefix
    local compressed_pubkey
    compressed_pubkey="0x$(user_compressed_pubkey_from_wif)"
    local payload
    payload=$(printf '{"amount_in_wei":%s,"usr_pub_key":"%s"}' "$amount_in_wei" "$compressed_pubkey")
    if ! curl_json_post "$endpoint" "$payload" >/dev/null; then
        die "Pegout request failed"
    fi
}

# change to project root (parent of tests directory)
cd "$(dirname "$0")/.."

step "Step 0: Preflight"
log "Configuration: stream=${STREAM_ID}, rsk=${RSK_ADDRESS}, amount=${VALUE}, env=${SCRIPT_ENV}"
log "Bitcoin RPC: ${BITCOIN_RPC_HOST}:${BITCOIN_RPC_PORT} (wallet=${BITCOIN_WALLET_NAME})"
require_cmd docker
require_cmd curl
require_cmd jq
require_cmd openssl

if ! bitcoin_cli_base getblockcount >/dev/null 2>&1; then
    die "Bitcoin RPC not reachable at ${BITCOIN_RPC_HOST}:${BITCOIN_RPC_PORT}"
fi

if ! rsk_rpc "eth_chainId" "[]" >/dev/null; then
    die "Rootstock RPC not accessible at ${RSK_RPC_URL}"
fi

if ! curl -sS "http://${USER_API_HOST}:40001/health" >/dev/null; then
    die "user-api not reachable at http://${USER_API_HOST}:40001/health"
fi

# Preflight: native bridge must be tracking Bitcoin headers for pegin SPV proofs to be accepted on-chain.
# Otherwise, PegManager calls will keep failing with MissingConfirmationsOnNativeBridge.
if [[ "${SKIP_NATIVE_BRIDGE_CHECK:-false}" != "true" ]]; then
    btc_height=$(get_current_bitcoin_height)
    bridge_btc_height=$(rsk_bridge_btc_best_height)
    log "Native Bridge BTC best height: ${bridge_btc_height} (bitcoind height: ${btc_height})"
    if (( bridge_btc_height <= 0 && btc_height > 1 )); then
        warn "Native Bridge is not tracking Bitcoin headers (btcBlockchainBestChainHeight=${bridge_btc_height})."
        warn "This will block peg-in (PegManager will revert with MissingConfirmationsOnNativeBridge)."
        warn "Fix requires powpeg/federator header relay to be enabled and pointed at the same bitcoind chain."
        warn "To bypass this check (not recommended), re-run with: SKIP_NATIVE_BRIDGE_CHECK=true"
        die "Native Bridge not synced with Bitcoin (cannot complete happy path)."
    fi
fi

ensure_bitcoind_wallet "$BITCOIN_WALLET_NAME"
ensure_bitcoind_watch_wallet "$BITVMX_WALLET_NAME"
ensure_user_bitcoin_wif
import_user_wif

USER_BTC_ADDRESS=$(user_address_from_wif)
success "User BTC address ready: ${USER_BTC_ADDRESS}"

required_sats=$((VALUE + BITVMX_FUND_AMOUNT * NUM_OPERATORS + 100000))
confirmed_sats=$(wallet_confirmed_balance_sats)
if (( confirmed_sats >= required_sats )); then
    success "Wallet already funded (${confirmed_sats} sats confirmed)"
else
    log "Wallet under target (${confirmed_sats}/${required_sats} sats). Trying rescan/import recovery..."
    rescan_user_wif_history
    confirmed_sats=$(wallet_confirmed_balance_sats)
    if (( confirmed_sats >= required_sats )); then
        success "Bitcoin wallet funded after rescan (${confirmed_sats} sats confirmed)"
    else
        local_shortfall=$((required_sats - confirmed_sats))
        topup_sats=$((local_shortfall + 100000))
        topup_btc=$(sats_to_btc "$topup_sats")
        refill_addr=$(bitcoin_cli_wallet "$BITCOIN_WALLET_NAME" getnewaddress "replenish" "bech32")
        log "Replenishing ${BITCOIN_WALLET_NAME} from ${BITCOIN_FUNDING_WALLET_NAME}: ${topup_sats} sats"
        bitcoin_cli_wallet "$BITCOIN_FUNDING_WALLET_NAME" settxfee "0.00001000" >/dev/null 2>&1 || true
        bitcoin_cli_wallet "$BITCOIN_FUNDING_WALLET_NAME" sendtoaddress "$refill_addr" "$topup_btc" >/dev/null
        mine_blocks 1
        confirmed_sats=$(wallet_confirmed_balance_sats)
        if (( confirmed_sats < required_sats )); then
            die "Insufficient BTC funds after wallet top-up (have=${confirmed_sats}, need=${required_sats})"
        fi
        success "Bitcoin wallet funded (${confirmed_sats} sats confirmed)"
    fi
fi

step "Step 1: Fund Operator Wallets"
fund_rsk_wallets
fund_bitvmx_wallets
success "Operator wallets funded"

step "Step 2: Whitelist Member Addresses"
whitelist_member_addresses
success "Member addresses whitelisted"

step "Step 3: Apply Operators to Stream"
apply_stream
    if ! wait_for_log_with_block_timeout "CommitteeSetupFlow Done" "$COMMITTEE_SETUP_MAX_BLOCKS"; then
    if [[ "$COMMITTEE_LOG_STRICT" == "true" ]]; then
        dump_recent_logs "coordinator"
        dump_recent_logs "user-api"
        die "Committee setup did not complete (see logs above)"
    fi

    warn "CommitteeSetupFlow Done not found; checking for apply-stream success logs instead"
    if ! wait_for_any_log_with_block_timeout \
        "Applied to stream StreamId\\(${STREAM_ID}\\) successfully" \
        "Member already registered for stream" \
        "$COMMITTEE_SETUP_MAX_BLOCKS"; then
        dump_recent_logs "coordinator"
        dump_recent_logs "user-api"
        die "Committee setup did not complete (see logs above)"
    fi
fi
success "Operators applied to stream ${STREAM_ID}"

step "Step 4: Request Pegin"
log "RSK Address: $RSK_ADDRESS"
log "Amount: $VALUE sats"
pegin_data=$(request_pegin_data)
pegin_address=$(echo "$pegin_data" | jq -r '.address')
pegin_packet_number=$(echo "$pegin_data" | jq -r '.packet_number')
pegin_enabler_script_pubkey=$(echo "$pegin_data" | jq -r '.enabler_script_pubkey')
log "Packet: $pegin_packet_number"
log "Pegin address: $pegin_address"
log "Enabler script: $pegin_enabler_script_pubkey"
# BitVMX emits PeginTransactionFound based on what its bitcoind wallet sees.
# Import the temporary pegin address into the BitVMX wallet (watch-only) before broadcasting the tx.
import_watch_address_into_wallet "$BITVMX_WALLET_NAME" "$pegin_address"
if is_true "$RESTART_BITVMX_ON_WALLET_CREATE"; then
    restart_bitvmx_and_coordinators "reload the imported pegin watch address"
fi
create_pegin_tx "$pegin_address" "$pegin_packet_number" "$pegin_enabler_script_pubkey"
wait_for_log_with_block_timeout "PeginFlow Done" "$PEGIN_MAX_BLOCKS"
success "Pegin flow completed"

step "Step 4: Request Pegout"
request_pegout
wait_for_log_with_block_timeout "PegoutFlow Done" "$PEGOUT_MAX_BLOCKS"
success "Pegout flow completed"

step "Complete"
success "Regtest happy path completed successfully"
