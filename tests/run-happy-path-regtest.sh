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

# Verify operators are deployed
check_operators_deployed() {
    local missing=0
    for op_id in 1 2 3 4; do
        if ! docker ps --format "{{.Names}}" | grep -q "op_${op_id}-coordinator-1"; then
            missing=$((missing + 1))
        fi
    done
    if [[ $missing -gt 0 ]]; then
        echo -e "\033[1;33m[!]\033[0m Operators not fully deployed ($((4 - missing))/4 running)"
        echo "    Start them with: ./cli-infra.sh --start-regtest"
        exit 1
    fi
}
check_operators_deployed

SCRIPT_ENV="regtest"

RSK_RPC_URL="${RSK_RPC_URL:-http://node-use2-1.regtest.rskcomputing.net:4444}"
USER_API_HOST="${USER_API_HOST:-localhost}"
BITCOIN_RPC_HOST="${BITCOIN_RPC_HOST:-10.1.0.107}"
BITCOIN_RPC_PORT="${BITCOIN_RPC_PORT:-18332}"
BITCOIN_RPC_USER="${BITCOIN_RPC_USER:-user}"
BITCOIN_RPC_PASSWORD="${BITCOIN_RPC_PASSWORD:-pass}"
BITCOIN_WALLET_NAME="${BITCOIN_WALLET_NAME:-mainwallet}"
BITVMX_WALLET_NAME="${BITVMX_WALLET_NAME:-test_wallet}"
RESTART_BITVMX_ON_WALLET_CREATE="${RESTART_BITVMX_ON_WALLET_CREATE:-true}"

BITVMX_FUND_AMOUNT="${BITVMX_FUND_AMOUNT:-32002000}"
RSK_FUND_AMOUNT_WEI="${RSK_FUND_AMOUNT_WEI:-0x3782dace9d90000}"
RSK_GAS_PRICE_WEI="${RSK_GAS_PRICE_WEI:-0x3938700}"

STREAM_ID="${STREAM_ID:-0}"
VALUE="${VALUE:-100000}"
PACKET_NUMBER="${PACKET_NUMBER:-0}"
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

    # Descriptor wallets need importdescriptors. Import using the *WIF* (not the expanded pubkey)
    # so the wallet definitely has the private key available for spending.
    local checksum desc import_req
    checksum=$(bitcoin_cli_base getdescriptorinfo "wpkh(${USER_BITCOIN_WIF})" | jq -r '.checksum // empty')
    if [[ -z "$checksum" ]]; then
        die "Failed to compute descriptor checksum for USER_BITCOIN_WIF"
    fi
    desc="wpkh(${USER_BITCOIN_WIF})#${checksum}"

    # Mark it active so the wallet recognizes derived addresses immediately.
    import_req=$(printf '[{"desc":"%s","timestamp":"now","active":true,"label":"user"}]' "$desc")
    if ! bitcoin_cli_wallet "$BITCOIN_WALLET_NAME" importdescriptors "$import_req" >/dev/null 2>&1; then
        die "Failed to import USER_BITCOIN_WIF into descriptor wallet (${BITCOIN_WALLET_NAME})"
    fi
}

import_watch_address_into_wallet() {
    local wallet_name="$1"
    local addr="$2"

    local checksum desc import_req isrange active
    checksum=$(bitcoin_cli_base getdescriptorinfo "addr(${addr})" | jq -r '.checksum // empty')
    if [[ -z "$checksum" ]]; then
        die "Failed to compute descriptor checksum for addr(${addr})"
    fi
    desc="addr(${addr})#${checksum}"
    isrange=$(bitcoin_cli_base getdescriptorinfo "addr(${addr})" | jq -r '.isrange // false')
    if [[ "$isrange" == "true" ]]; then
        active="true"
    else
        active="false"
    fi

    # Non-ranged descriptors (like addr()) must not be active.
    import_req=$(printf '[{"desc":"%s","timestamp":"now","active":%s,"label":"watch"}]' "$desc" "$active")
    if ! bitcoin_cli_wallet "$wallet_name" importdescriptors "$import_req" >/dev/null 2>&1; then
        die "Failed to import watch address into wallet ${wallet_name}: ${addr}"
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
    for op_id in 1 2 3 4; do
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
    for op_id in 1 2 3 4; do
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
    echo "${host}:40001 ${host}:40002 ${host}:40003 ${host}:40004"
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
    for op_id in 1 2 3 4; do
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
    for op_id in 1 2 3 4; do
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

    if [[ "${#member_addrs[@]}" -lt 4 || "${#user_addrs[@]}" -lt 4 ]]; then
        die "Missing RSK addresses (member=${#member_addrs[@]}, user=${#user_addrs[@]})"
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

    sleep 5

    mapfile -t bitvmx_addrs < <(collect_bitvmx_addresses)
    if [[ "${#bitvmx_addrs[@]}" -lt 4 ]]; then
        die "Missing BitVMX funding addresses (found ${#bitvmx_addrs[@]})"
    fi

    local amount_btc
    amount_btc=$(sats_to_btc "$BITVMX_FUND_AMOUNT")
    local outputs
    outputs=$(printf '%s\n' "${bitvmx_addrs[@]}" | jq -Rsc --argjson amt "$amount_btc" \
        'split("\n") | map(select(. != "")) | reduce .[] as $addr ({}; .[$addr] = $amt)')
    log "Funding ${#bitvmx_addrs[@]} BitVMX addresses in one transaction"
    # Set a fixed fee rate for regtest (fallback fee not enabled on node)
    bitcoin_cli_wallet "$BITCOIN_WALLET_NAME" settxfee 0.0001 >/dev/null 2>&1 || true
    bitcoin_cli_wallet "$BITCOIN_WALLET_NAME" sendmany "" "$outputs" >/dev/null

    mine_blocks 1
}

apply_stream() {
    local endpoints
    endpoints=($(user_api_endpoints))
    local roles=("Prover" "Verifier" "Prover" "Verifier")

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

request_pegin_address() {
    local endpoint="http://${USER_API_HOST}:40001/user/pegin-address"
    # Get x-only public key (32 bytes) with 0x prefix
    local xonly_pubkey
    xonly_pubkey="0x$(user_xonly_pubkey_from_wif)"
    local payload
    payload=$(printf '{"rootstock_deposit_address":"%s","value":%s,"btc_reimbursement_pub_key":"%s"}' "$RSK_ADDRESS" "$VALUE" "$xonly_pubkey")
    local response
    response=$(curl_json_post "$endpoint" "$payload")
    local addr
    addr=$(echo "$response" | jq -r '.address')
    if [[ -z "$addr" || "$addr" == "null" ]]; then
        die "Failed to parse pegin address from user-api response"
    fi
    echo "$addr"
}

create_pegin_tx() {
    local pegin_address="$1"
    local user_address="$2"
    local rsk_hex
    rsk_hex=$(echo "${RSK_ADDRESS#0x}" | tr 'A-F' 'a-f')
    if [[ ${#rsk_hex} -ne 40 ]]; then
        die "Invalid RSK address: $RSK_ADDRESS"
    fi

    local packet_hex
    packet_hex=$(printf "%016x" "$PACKET_NUMBER")
    local xonly
    xonly=$(user_xonly_pubkey_from_wif)
    local op_return_hex="52534b5f504547494e${packet_hex}${rsk_hex}${xonly}"

    local amount_btc
    amount_btc=$(sats_to_btc "$VALUE")

    local outputs
    outputs=$(printf '[{"%s":%s},{"data":"%s"}]' "$pegin_address" "$amount_btc" "$op_return_hex")

    local raw
    raw=$(bitcoin_cli_wallet "$BITCOIN_WALLET_NAME" createrawtransaction "[]" "$outputs")
    local funded
    # Keep output ordering stable for downstream parsers:
    # - vout[0] = pegin address payment
    # - vout[1] = OP_RETURN marker
    # - vout[2] = change (if any)
    funded=$(bitcoin_cli_wallet "$BITCOIN_WALLET_NAME" fundrawtransaction "$raw" "{\"changeAddress\":\"$user_address\",\"changePosition\":2}")
    local funded_hex
    funded_hex=$(echo "$funded" | jq -r '.hex')
    local signed
    signed=$(bitcoin_cli_wallet "$BITCOIN_WALLET_NAME" signrawtransactionwithwallet "$funded_hex")
    local complete
    complete=$(echo "$signed" | jq -r '.complete')
    if [[ "$complete" != "true" ]]; then
        die "Failed to sign pegin transaction"
    fi
    local signed_hex
    signed_hex=$(echo "$signed" | jq -r '.hex')
    local txid
    txid=$(bitcoin_cli_wallet "$BITCOIN_WALLET_NAME" sendrawtransaction "$signed_hex")
    log "Pegin txid: $txid"
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

MAIN_WALLET_CREATED=false
if ensure_bitcoind_wallet "$BITCOIN_WALLET_NAME"; then
    MAIN_WALLET_CREATED=true
fi

BITVMX_WALLET_CREATED=false
if ensure_bitcoind_wallet "$BITVMX_WALLET_NAME"; then
    BITVMX_WALLET_CREATED=true
fi
ensure_user_bitcoin_wif
import_user_wif

USER_BTC_ADDRESS=$(user_address_from_wif)
success "User BTC address ready: ${USER_BTC_ADDRESS}"

required_sats=$((VALUE + BITVMX_FUND_AMOUNT * 4 + 100000))
confirmed_sats=$(wallet_confirmed_balance_sats)
if (( confirmed_sats >= required_sats )); then
    success "Wallet already funded (${confirmed_sats} sats confirmed)"
else
    log "Mining 101 blocks to fund user wallet..."
    mine_blocks_to_address 101 "$USER_BTC_ADDRESS"
    success "Bitcoin wallet funded"
fi

step "Step 1: Fund Operator Wallets"
fund_rsk_wallets
fund_bitvmx_wallets
success "Operator wallets funded"

step "Step 2: Apply Operators to Stream"
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

step "Step 3: Request Pegin"
log "RSK Address: $RSK_ADDRESS"
log "Amount: $VALUE sats"
log "Packet: $PACKET_NUMBER"
pegin_address=$(request_pegin_address)
log "Pegin address: $pegin_address"
# BitVMX emits PeginTransactionFound based on what its bitcoind wallet sees.
# Import the temporary pegin address into the BitVMX wallet (watch-only) before broadcasting the tx.
import_watch_address_into_wallet "$BITVMX_WALLET_NAME" "$pegin_address"
create_pegin_tx "$pegin_address" "$USER_BTC_ADDRESS"
wait_for_log_with_block_timeout "PeginFlow Done" "$PEGIN_MAX_BLOCKS"
success "Pegin flow completed"

step "Step 4: Request Pegout"
request_pegout
wait_for_log_with_block_timeout "PegoutFlow Done" "$PEGOUT_MAX_BLOCKS"
success "Pegout flow completed"

step "Complete"
success "Regtest happy path completed successfully"
