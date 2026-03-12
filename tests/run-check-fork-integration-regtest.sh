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
CHECKFORK_EXECUTOR_OPERATOR_ID="${CHECKFORK_EXECUTOR_OPERATOR_ID:-1}"

BITCOIN_RPC_HOST="${BITCOIN_RPC_HOST:-10.1.0.107}"
BITCOIN_RPC_PORT="${BITCOIN_RPC_PORT:-18332}"
BITCOIN_RPC_USER="${BITCOIN_RPC_USER:-user}"
BITCOIN_RPC_PASSWORD="${BITCOIN_RPC_PASSWORD:-pass}"
BITCOIN_WALLET_NAME="${BITCOIN_WALLET_NAME:-mainwallet}"
CHECKFORK_AUTO_MINE="${CHECKFORK_AUTO_MINE:-true}"
RSK_RPC_URL="${RSK_RPC_URL:-http://node-use2-1.regtest.rskcomputing.net:4444}"
CHECKFORK_MOCK_RPC_URL="${CHECKFORK_MOCK_RPC_URL:-ws://node-use2-1.regtest.rskcomputing.net:4445}"
CHECKFORK_MOCK_PRIVATE_KEY="${CHECKFORK_MOCK_PRIVATE_KEY:-0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80}"
CHECKFORK_REQUIRED_NUM_BLOCKS="${CHECKFORK_REQUIRED_NUM_BLOCKS:-5}"
CHECKFORK_RSK_BLOCKS_AFTER_ADVANCE="${CHECKFORK_RSK_BLOCKS_AFTER_ADVANCE:-}"

GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m'

log() { echo -e "${BLUE}[INFO]${NC} $1"; }
success() { echo -e "${GREEN}[OK]${NC} $1"; }
warn() { echo -e "${YELLOW}[WARN]${NC} $1"; }
die() { echo "Error: $1" >&2; exit 1; }
require_cmd() { command -v "$1" >/dev/null 2>&1 || die "Required command not found: $1"; }

export PATH="${HOME}/.cargo/bin:${HOME}/.foundry/bin:${PATH}"
STOPPED_BITVMX_CLIENTS=()

if [[ "${REGTEST_REMOTE:-}" != "1" ]]; then
    log "Connecting to regtest instance: ${REGTEST_HOST}"
    exec ssh -A "${REGTEST_USER}@${REGTEST_HOST}" \
        "cd ~/${REGTEST_ROOT} && REGTEST_REMOTE=1 CHECKFORK_DISPATCH_MAX_BLOCKS='${CHECKFORK_DISPATCH_MAX_BLOCKS}' CHECKFORK_PROOF_MAX_BLOCKS='${CHECKFORK_PROOF_MAX_BLOCKS}' CHECKFORK_LOG_SINCE='${CHECKFORK_LOG_SINCE}' CHECKFORK_LOG_TAIL_LINES='${CHECKFORK_LOG_TAIL_LINES}' CHECKFORK_ACCEPT_PROOF_NOT_READY='${CHECKFORK_ACCEPT_PROOF_NOT_READY}' HAPPYPATH_COMMITTEE_SETUP_MAX_BLOCKS='${HAPPYPATH_COMMITTEE_SETUP_MAX_BLOCKS}' HAPPYPATH_PEGIN_MAX_BLOCKS='${HAPPYPATH_PEGIN_MAX_BLOCKS}' HAPPYPATH_PEGOUT_MAX_BLOCKS='${HAPPYPATH_PEGOUT_MAX_BLOCKS}' CHECKFORK_EXECUTOR_OPERATOR_ID='${CHECKFORK_EXECUTOR_OPERATOR_ID}' BITCOIN_RPC_HOST='${BITCOIN_RPC_HOST}' BITCOIN_RPC_PORT='${BITCOIN_RPC_PORT}' BITCOIN_RPC_USER='${BITCOIN_RPC_USER}' BITCOIN_RPC_PASSWORD='${BITCOIN_RPC_PASSWORD}' BITCOIN_WALLET_NAME='${BITCOIN_WALLET_NAME}' CHECKFORK_AUTO_MINE='${CHECKFORK_AUTO_MINE}' RSK_RPC_URL='${RSK_RPC_URL}' CHECKFORK_MOCK_RPC_URL='${CHECKFORK_MOCK_RPC_URL}' CHECKFORK_MOCK_PRIVATE_KEY='${CHECKFORK_MOCK_PRIVATE_KEY}' CHECKFORK_REQUIRED_NUM_BLOCKS='${CHECKFORK_REQUIRED_NUM_BLOCKS}' CHECKFORK_RSK_BLOCKS_AFTER_ADVANCE='${CHECKFORK_RSK_BLOCKS_AFTER_ADVANCE}' bash tests/run-check-fork-integration-regtest.sh"
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
    local payload
    payload=$(printf '{"jsonrpc":"1.0","id":"union","method":"%s","params":%s}' "$method" "$params")

    local response
    response=$(curl -sS --user "${BITCOIN_RPC_USER}:${BITCOIN_RPC_PASSWORD}" \
        -H "content-type: text/plain;" \
        --data-binary "$payload" \
        "http://${BITCOIN_RPC_HOST}:${BITCOIN_RPC_PORT}")

    local err
    err=$(echo "$response" | jq -r '.error.message // empty' 2>/dev/null || true)
    if [[ -n "$err" ]]; then
        echo "Bitcoin RPC error: $err" >&2
        return 1
    fi

    echo "$response" | jq -cr '.result'
}

bitcoin_rpc_call_wallet() {
    local wallet_name="$1"
    local method="$2"
    local params="$3"
    local payload
    payload=$(printf '{"jsonrpc":"1.0","id":"union","method":"%s","params":%s}' "$method" "$params")

    local response
    response=$(curl -sS --user "${BITCOIN_RPC_USER}:${BITCOIN_RPC_PASSWORD}" \
        -H "content-type: text/plain;" \
        --data-binary "$payload" \
        "http://${BITCOIN_RPC_HOST}:${BITCOIN_RPC_PORT}/wallet/${wallet_name}")

    local err
    err=$(echo "$response" | jq -r '.error.message // empty' 2>/dev/null || true)
    if [[ -n "$err" ]]; then
        echo "Bitcoin RPC error: $err" >&2
        return 1
    fi

    echo "$response" | jq -cr '.result'
}

get_contract_address() {
    local contract_name="$1"
    awk -v contract_name="$contract_name" '
        /^\[\[contracts\]\]$/ { target=0 }
        $0 == "name = \"" contract_name "\"" { target=1; next }
        target && $0 ~ /^address = / {
            gsub(/"/, "", $3)
            print $3
            exit
        }
    ' config/environment/regtest.toml
}

rsk_rpc_call() {
    local method="$1"
    local params="$2"
    local payload
    payload=$(printf '{"jsonrpc":"2.0","method":"%s","params":%s,"id":1}' "$method" "$params")
    curl -sS -H "Content-Type: application/json" --data "$payload" "$RSK_RPC_URL"
}

wait_for_rsk_tx_receipt() {
    local tx_hash="$1"
    local attempts="${2:-30}"
    local i

    for ((i = 0; i < attempts; i++)); do
        local response status
        response=$(rsk_rpc_call "eth_getTransactionReceipt" "[\"${tx_hash}\"]")
        status=$(echo "$response" | jq -r '.result.status // empty')
        if [[ "$status" == "0x1" ]]; then
            return 0
        fi
        if [[ "$status" == "0x0" ]]; then
            die "RSK contract tx reverted: ${tx_hash}"
        fi
        sleep 1
    done

    die "Timeout waiting RSK tx receipt: ${tx_hash}"
}

emit_fake_peg_manager_tx() {
    local fake_address="$1"
    local signature="$2"
    shift 2

    local data
    data=$(cast calldata "$signature" "$@")

    local from_addr
    from_addr=$(rsk_rpc_call "eth_accounts" "[]" | jq -r '.result[0] // empty')
    [[ -n "$from_addr" && "$from_addr" != "null" ]] || die "Failed to get unlocked RSK account"

    local response tx_hash err
    response=$(rsk_rpc_call \
        "eth_sendTransaction" \
        "[{\"from\":\"${from_addr}\",\"to\":\"${fake_address}\",\"data\":\"${data}\",\"gas\":\"0x493e0\",\"gasPrice\":\"0x0\"}]")
    err=$(echo "$response" | jq -r '.error.message // empty')
    if [[ -n "$err" ]]; then
        die "RSK contract tx failed: $err"
    fi

    tx_hash=$(echo "$response" | jq -r '.result // empty')
    [[ -n "$tx_hash" && "$tx_hash" != "null" ]] || die "RSK contract tx did not return a hash"

    wait_for_rsk_tx_receipt "$tx_hash"
}

emit_followup_rsk_blocks() {
    local fake_address="$1"
    local pegout_id="$2"
    local required_num_blocks="${CHECKFORK_REQUIRED_NUM_BLOCKS}"
    local followup_blocks="${CHECKFORK_RSK_BLOCKS_AFTER_ADVANCE:-$((required_num_blocks + 2))}"
    local i

    for i in $(seq 1 "$followup_blocks"); do
        emit_fake_peg_manager_tx "$fake_address" "checkForkComplete(string)" "noop_${pegout_id}_${i}"
        sleep 1
    done
}

restore_restricted_bitvmx_clients() {
    local container
    for container in "${STOPPED_BITVMX_CLIENTS[@]}"; do
        if docker ps -a --format "{{.Names}}" | grep -qx "$container"; then
            log "Restoring ${container}"
            docker start "$container" >/dev/null || true
        fi
    done
}

restrict_checkfork_prover_to_operator() {
    local executor_id="${CHECKFORK_EXECUTOR_OPERATOR_ID}"
    local op_id

    log "Keeping only op_${executor_id} bitvmx-client active for CheckFork proving"
    for op_id in 1 2 3 4; do
        local container="op_${op_id}-bitvmx-client-1"
        if [[ "$op_id" == "$executor_id" ]]; then
            continue
        fi
        if docker ps --format "{{.Names}}" | grep -qx "$container"; then
            docker stop "$container" >/dev/null
            STOPPED_BITVMX_CLIENTS+=("$container")
        fi
    done
}

trigger_advance_funds_mock_events() {
    require_cmd cargo
    require_cmd cast
    require_cmd curl
    require_cmd jq

    local fake_peg_manager_address
    fake_peg_manager_address=$(get_contract_address "FakePegManager")
    local peg_manager_address
    peg_manager_address=$(get_contract_address "PegManager")

    [[ -n "$fake_peg_manager_address" ]] || die "FakePegManager address not found in config/environment/regtest.toml"
    [[ -n "$peg_manager_address" ]] || die "PegManager address not found in config/environment/regtest.toml"
    [[ "${fake_peg_manager_address,,}" != "${peg_manager_address,,}" ]] \
        || die "FakePegManager must differ from PegManager on regtest"

    local mock_pegout_id
    mock_pegout_id="checkfork_$(date -u +%Y%m%dT%H%M%SZ)"
    log "Triggering RequestAdvanceFunds/AdvanceFunds via cli/mocks for pegout_id=${mock_pegout_id}"

    local mock_cli_output
    mock_cli_output=$(
        printf 'raf %s\nkaf %s\nexit\n' "$mock_pegout_id" "$mock_pegout_id" |
            MOCKS_PRIVATE_KEY="${CHECKFORK_MOCK_PRIVATE_KEY}" \
                FAKE_PEG_MANAGER_ADDRESS="${fake_peg_manager_address}" \
                CHECK_FORK_REQUIRED_NUM_BLOCKS="${CHECKFORK_REQUIRED_NUM_BLOCKS}" \
                cargo run --manifest-path cli/mocks/Cargo.toml -- \
                    --rpc-url "${CHECKFORK_MOCK_RPC_URL}" \
                    --no-deploy 2>&1
    ) || {
        echo "$mock_cli_output"
        die "cli/mocks failed while emitting RequestAdvanceFunds/AdvanceFunds"
    }

    echo "$mock_cli_output"
    emit_followup_rsk_blocks "$fake_peg_manager_address" "$mock_pegout_id"
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
trap restore_restricted_bitvmx_clients EXIT

log "Running regtest happy path to trigger advance_funds/checkfork"
if ! LOG_SINCE="$CHECKFORK_LOG_SINCE" REGTEST_REMOTE=1 COMMITTEE_SETUP_MAX_BLOCKS="$HAPPYPATH_COMMITTEE_SETUP_MAX_BLOCKS" PEGIN_MAX_BLOCKS="$HAPPYPATH_PEGIN_MAX_BLOCKS" PEGOUT_MAX_BLOCKS="$HAPPYPATH_PEGOUT_MAX_BLOCKS" bash tests/run-happy-path-regtest.sh; then
    dump_recent_logs "coordinator"
    dump_recent_logs "bitvmx-client"
    die "run-happy-path-regtest.sh failed"
fi

restrict_checkfork_prover_to_operator
trigger_advance_funds_mock_events

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
