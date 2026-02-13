#!/usr/bin/env bash

# Regtest integration check for CheckFork -> BitVMX proof lifecycle.
# Runs the happy path and then validates CheckFork ZKP logs by request_id correlation.

set -euo pipefail

REGTEST_HOST="${REGTEST_HOST:-union-bridge-use2-1.regtest.rskcomputing.net}"
REGTEST_USER="${REGTEST_USER:-ubuntu}"
REGTEST_ROOT="${REGTEST_ROOT:-union-bridge-client}"

CHECKFORK_DISPATCH_MAX_BLOCKS="${CHECKFORK_DISPATCH_MAX_BLOCKS:-40}"
CHECKFORK_PROOF_MAX_BLOCKS="${CHECKFORK_PROOF_MAX_BLOCKS:-80}"
CHECKFORK_LOG_SINCE="${CHECKFORK_LOG_SINCE:-30m}"
CHECKFORK_LOG_TAIL_LINES="${CHECKFORK_LOG_TAIL_LINES:-1200}"
CHECKFORK_ACCEPT_PROOF_NOT_READY="${CHECKFORK_ACCEPT_PROOF_NOT_READY:-true}"
HAPPYPATH_COMMITTEE_SETUP_MAX_BLOCKS="${HAPPYPATH_COMMITTEE_SETUP_MAX_BLOCKS:-}"
HAPPYPATH_PEGIN_MAX_BLOCKS="${HAPPYPATH_PEGIN_MAX_BLOCKS:-}"
HAPPYPATH_PEGOUT_MAX_BLOCKS="${HAPPYPATH_PEGOUT_MAX_BLOCKS:-}"

BITCOIN_RPC_HOST="${BITCOIN_RPC_HOST:-10.1.0.107}"
BITCOIN_RPC_PORT="${BITCOIN_RPC_PORT:-18332}"
BITCOIN_RPC_USER="${BITCOIN_RPC_USER:-user}"
BITCOIN_RPC_PASSWORD="${BITCOIN_RPC_PASSWORD:-pass}"
BITCOIN_WALLET_NAME="${BITCOIN_WALLET_NAME:-mainwallet}"
CHECKFORK_AUTO_MINE="${CHECKFORK_AUTO_MINE:-true}"

GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m'

log() { echo -e "${BLUE}[INFO]${NC} $1"; }
success() { echo -e "${GREEN}[OK]${NC} $1"; }
warn() { echo -e "${YELLOW}[WARN]${NC} $1"; }
die() { echo "Error: $1" >&2; exit 1; }

if [[ "${REGTEST_REMOTE:-}" != "1" ]]; then
    log "Connecting to regtest instance: ${REGTEST_HOST}"
    exec ssh -A "${REGTEST_USER}@${REGTEST_HOST}" \
        "cd ~/${REGTEST_ROOT} && REGTEST_REMOTE=1 CHECKFORK_DISPATCH_MAX_BLOCKS='${CHECKFORK_DISPATCH_MAX_BLOCKS}' CHECKFORK_PROOF_MAX_BLOCKS='${CHECKFORK_PROOF_MAX_BLOCKS}' CHECKFORK_LOG_SINCE='${CHECKFORK_LOG_SINCE}' CHECKFORK_LOG_TAIL_LINES='${CHECKFORK_LOG_TAIL_LINES}' CHECKFORK_ACCEPT_PROOF_NOT_READY='${CHECKFORK_ACCEPT_PROOF_NOT_READY}' HAPPYPATH_COMMITTEE_SETUP_MAX_BLOCKS='${HAPPYPATH_COMMITTEE_SETUP_MAX_BLOCKS}' HAPPYPATH_PEGIN_MAX_BLOCKS='${HAPPYPATH_PEGIN_MAX_BLOCKS}' HAPPYPATH_PEGOUT_MAX_BLOCKS='${HAPPYPATH_PEGOUT_MAX_BLOCKS}' BITCOIN_RPC_HOST='${BITCOIN_RPC_HOST}' BITCOIN_RPC_PORT='${BITCOIN_RPC_PORT}' BITCOIN_RPC_USER='${BITCOIN_RPC_USER}' BITCOIN_RPC_PASSWORD='${BITCOIN_RPC_PASSWORD}' BITCOIN_WALLET_NAME='${BITCOIN_WALLET_NAME}' CHECKFORK_AUTO_MINE='${CHECKFORK_AUTO_MINE}' bash tests/run-check-fork-integration-regtest.sh"
fi

check_operators_deployed() {
    local missing=0
    for op_id in 1 2 3 4; do
        if ! docker ps --format "{{.Names}}" | grep -q "op_${op_id}-coordinator-1"; then
            missing=$((missing + 1))
        fi
    done
    if [[ $missing -gt 0 ]]; then
        die "Operators not fully deployed ($((4 - missing))/4 running). Start them with ./cli-infra.sh --start-regtest"
    fi
}

bitcoin_rpc_call() {
    local method="$1"
    local params="$2"
    bitcoin_rpc_call_path "" "$method" "$params"
}

bitcoin_rpc_call_wallet() {
    local wallet_name="$1"
    local method="$2"
    local params="$3"
    bitcoin_rpc_call_path "/wallet/${wallet_name}" "$method" "$params"
}

bitcoin_rpc_call_path() {
    local path="$1"
    local method="$2"
    local params="$3"
    local payload
    payload=$(printf '{"jsonrpc":"1.0","id":"union","method":"%s","params":%s}' "$method" "$params")

    local response
    response=$(curl -sS --user "${BITCOIN_RPC_USER}:${BITCOIN_RPC_PASSWORD}" \
        -H "content-type: text/plain;" \
        --data-binary "$payload" \
        "http://${BITCOIN_RPC_HOST}:${BITCOIN_RPC_PORT}${path}")

    local err
    err=$(echo "$response" | jq -r '.error.message // empty' 2>/dev/null || true)
    if [[ -n "$err" ]]; then
        echo "Bitcoin RPC error: $err" >&2
        return 1
    fi

    echo "$response" | jq -cr '.result'
}

maybe_mine_one_block() {
    if [[ "${CHECKFORK_AUTO_MINE}" != "true" ]]; then
        return 0
    fi

    local miner_addr
    miner_addr=$(bitcoin_rpc_call_wallet "$BITCOIN_WALLET_NAME" "getnewaddress" '["checkfork-miner","bech32"]' 2>/dev/null || true)
    if [[ -z "$miner_addr" || "$miner_addr" == "null" ]]; then
        return 0
    fi
    bitcoin_rpc_call "generatetoaddress" "[1,\"${miner_addr}\"]" >/dev/null 2>&1 || true
}

get_current_bitcoin_height() {
    local height
    height=$(bitcoin_rpc_call "getblockcount" "[]" 2>/dev/null || echo "0")
    height=${height:-0}
    echo "$height"
}

find_recent_log_match() {
    local container_suffix="$1"
    local pattern="$2"

    for op_id in 1 2 3 4; do
        local container="op_${op_id}-${container_suffix}-1"
        if ! docker ps --format "{{.Names}}" | grep -qx "$container"; then
            continue
        fi

        local line
        line=$(docker logs --since "$CHECKFORK_LOG_SINCE" --tail "$CHECKFORK_LOG_TAIL_LINES" "$container" 2>/dev/null | grep -E "$pattern" | tail -1 || true)
        if [[ -n "$line" ]]; then
            echo "${container}:${line}"
            return 0
        fi
    done

    return 1
}

wait_for_log_match_with_block_timeout() {
    local container_suffix="$1"
    local pattern="$2"
    local max_blocks="$3"

    local start_height
    start_height=$(get_current_bitcoin_height)
    local target_height=$((start_height + max_blocks))
    if ((max_blocks > 0)); then
        log "Waiting for ${container_suffix} log pattern '${pattern}' (max ${max_blocks} blocks)"
    else
        log "Waiting for ${container_suffix} log pattern '${pattern}' (no block limit)"
    fi

    while true; do
        local current_height
        current_height=$(get_current_bitcoin_height)
        local blocks_mined=$((current_height - start_height))

        if ((blocks_mined < 0)); then
            sleep 1
            continue
        fi

        if ((max_blocks > 0)); then
            echo -ne "\r  Blocks mined: $blocks_mined/$max_blocks | Checking ${container_suffix} logs...  "
        else
            echo -ne "\r  Blocks mined: $blocks_mined | Checking ${container_suffix} logs...  "
        fi

        local result
        result=$(find_recent_log_match "$container_suffix" "$pattern" || true)
        if [[ -n "$result" ]]; then
            echo ""
            local source="${result%%:*}"
            local line="${result#*:}"
            success "Found pattern after ${blocks_mined} blocks in ${source}"
            echo "$line"
            return 0
        fi

        if ((max_blocks > 0 && current_height >= target_height)); then
            echo ""
            return 1
        fi

        maybe_mine_one_block
        sleep 2
    done
}

dump_recent_logs() {
    local suffix="$1"
    log "Recent logs for ${suffix}"
    for op_id in 1 2 3 4; do
        local container="op_${op_id}-${suffix}-1"
        if docker ps --format "{{.Names}}" | grep -qx "$container"; then
            echo "----- ${container} -----"
            docker logs --since "$CHECKFORK_LOG_SINCE" --tail "$CHECKFORK_LOG_TAIL_LINES" "$container" 2>/dev/null || true
        fi
    done
}

extract_request_id() {
    local line="$1"
    printf '%s\n' "$line" | sed -n 's/.*request_id=\([0-9a-fA-F-]\{36\}\).*/\1/p' | tail -1
}

wait_for_checkfork_proof_state() {
    local request_id="$1"
    local start_height
    start_height=$(get_current_bitcoin_height)
    local target_height=$((start_height + CHECKFORK_PROOF_MAX_BLOCKS))
    local saw_not_ready=false

    while true; do
        local current_height
        current_height=$(get_current_bitcoin_height)
        local blocks_mined=$((current_height - start_height))

        if ((blocks_mined < 0)); then
            sleep 1
            continue
        fi

        if ((CHECKFORK_PROOF_MAX_BLOCKS > 0)); then
            echo -ne "\r  Blocks mined: $blocks_mined/$CHECKFORK_PROOF_MAX_BLOCKS | Waiting proof state for request_id=${request_id}...  "
        else
            echo -ne "\r  Blocks mined: $blocks_mined | Waiting proof state for request_id=${request_id}...  "
        fi

        local fail_line
        fail_line=$(find_recent_log_match "coordinator" "event=checkfork_proof_generation_error.*request_id=${request_id}" || true)
        if [[ -n "$fail_line" ]]; then
            echo ""
            echo "${fail_line#*:}"
            return 2
        fi

        local success_line
        success_line=$(find_recent_log_match "coordinator" "event=checkfork_(proof_ready|zkp_result).*request_id=${request_id}" || true)
        if [[ -n "$success_line" ]]; then
            echo ""
            echo "${success_line#*:}"
            return 0
        fi

        local not_ready_line
        not_ready_line=$(find_recent_log_match "coordinator" "event=checkfork_proof_not_ready.*request_id=${request_id}" || true)
        if [[ -n "$not_ready_line" && "$saw_not_ready" != "true" ]]; then
            echo ""
            warn "ProofNotReady observed for request_id=${request_id}; continuing to wait"
            saw_not_ready=true
            if [[ "$CHECKFORK_ACCEPT_PROOF_NOT_READY" == "true" ]]; then
                echo "${not_ready_line#*:}"
                return 0
            fi
        fi

        if ((CHECKFORK_PROOF_MAX_BLOCKS > 0 && current_height >= target_height)); then
            echo ""
            return 1
        fi

        maybe_mine_one_block
        sleep 2
    done
}

check_operators_deployed

log "Running regtest happy path to trigger advance_funds/checkfork"
if ! LOG_SINCE="$CHECKFORK_LOG_SINCE" REGTEST_REMOTE=1 COMMITTEE_SETUP_MAX_BLOCKS="$HAPPYPATH_COMMITTEE_SETUP_MAX_BLOCKS" PEGIN_MAX_BLOCKS="$HAPPYPATH_PEGIN_MAX_BLOCKS" PEGOUT_MAX_BLOCKS="$HAPPYPATH_PEGOUT_MAX_BLOCKS" bash tests/run-happy-path-regtest.sh; then
    dump_recent_logs "coordinator"
    dump_recent_logs "bitvmx-client"
    die "run-happy-path-regtest.sh failed"
fi

if ! dispatch_line=$(wait_for_log_match_with_block_timeout "coordinator" "event=checkfork_zkp_dispatched" "$CHECKFORK_DISPATCH_MAX_BLOCKS"); then
    dump_recent_logs "coordinator"
    dump_recent_logs "bitvmx-client"
    die "Timeout waiting for checkfork dispatch log"
fi

request_id=$(extract_request_id "$dispatch_line")
if [[ -z "$request_id" ]]; then
    dump_recent_logs "coordinator"
    die "Could not extract request_id from dispatch log"
fi

log "Correlating proof lifecycle for request_id=${request_id}"
if proof_line=$(wait_for_checkfork_proof_state "$request_id"); then
    success "CheckFork integration reached proof state for request_id=${request_id}"
    echo "$proof_line"
    exit 0
fi

proof_status=$?
if [[ $proof_status -eq 2 ]]; then
    dump_recent_logs "coordinator"
    dump_recent_logs "bitvmx-client"
    die "ProofGenerationError observed for request_id=${request_id}"
fi

dump_recent_logs "coordinator"
dump_recent_logs "bitvmx-client"
die "Timeout waiting for ProofReady/ZKPResult for request_id=${request_id}"
