#!/usr/bin/env bash

# operator take path e2e test - automated local flow
#
# this test exercises the operator take path by:
#   1. running the normal pegin flow with auto-mining
#   2. stopping mining after pegin completes
#   3. requesting pegout with mining stopped, then manually mining RSK blocks
#   4. waiting for the PegOutAccepted message from BitVMX
#
# prerequisites:
#   - union bridge clients running (via: cargo run -- run)
#   - anvil running on localhost:8545
#   - bitcoin regtest node running with RPC enabled
#   - USER_BITCOIN_WIF and MEMBER_BITCOIN_WIF environment variables set
#
# usage: bash tests/run-operator-take-path.sh

set -euo pipefail

SCRIPT_ENV="local"

usage() {
    echo "Usage: $0 [--env <local|local-docker>]"
    exit 1
}

while [[ $# -gt 0 ]]; do
    case "$1" in
    --env)
        SCRIPT_ENV="${2:-}"
        if [[ -z "$SCRIPT_ENV" ]]; then
            usage
        fi
        shift 2
        ;;
    *)
        usage
        ;;
    esac
done

# change to project root (parent of tests directory)
cd "$(dirname "$0")/.."

# hardcoded configuration
STREAM_ID=0
RSK_ADDRESS="0x$(openssl rand -hex 20)" # random address each run
VALUE=100000
COMMITTEE_REGISTRY_ADDRESS="0x0DCd1Bf9A1b36cE34237eEaFef220932846BCD82"

# colors
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
PURPLE='\033[0;35m'
NC='\033[0m'

log() { echo -e "${BLUE}[INFO]${NC} $1"; }
success() { echo -e "${GREEN}[✓]${NC} $1"; }
warn() { echo -e "${YELLOW}[!]${NC} $1"; }
step() {
    echo ""
    echo -e "${GREEN}========== $1 ==========${NC}"
    echo ""
}

# get current bitcoin block height
get_current_bitcoin_height() {
    local height=$(bitcoin-cli -regtest -rpcuser=foo -rpcpassword=rpcpassword getblockcount 2>/dev/null || echo "0")
    height=${height:-0}  # ensure it's set to 0 if empty
    echo "$height"
}

# check if bitcoin node is accessible
check_bitcoin_connectivity() {
    bitcoin-cli -regtest -rpcuser=foo -rpcpassword=rpcpassword getblockcount &> /dev/null
}

# check prerequisites
if ! command -v cargo &> /dev/null || ! command -v cast &> /dev/null || ! command -v bitcoin-cli &> /dev/null; then
    echo "Error: cargo, cast, and bitcoin-cli required"
    exit 1
fi

if ! cast rpc eth_chainId --rpc-url http://localhost:8545 &> /dev/null; then
    echo "Error: Anvil not running on localhost:8545"
    exit 1
fi

if ! check_bitcoin_connectivity; then
    echo "Error: Bitcoin regtest node not accessible"
    echo "Please ensure Bitcoin Core is running with:"
    echo "  bitcoind -regtest -rpcuser=foo -rpcpassword=rpcpassword"
    exit 1
fi

echo "All prerequisites met!"
echo ""

# Find recent log match in docker compose logs (all 4 operators)
# Checks all logs (no time restriction) since we're already polling in a loop
# Output format: "source:line" or empty if not found
find_recent_docker_log_match() {
    local pattern="$1"

    for op_id in 1 2 3 4; do
        local project="op_${op_id}"
        local line
        # Check all logs - the polling loop already handles timing
        line=$(docker compose -p "$project" logs coordinator 2>/dev/null | grep -E "$pattern" | tail -1)
        if [ -n "$line" ]; then
            echo "${project}:${line}"
            return 0
        fi
    done
}

# Find recent log match in local log files
# Uses awk with string comparison (ISO timestamps are lexicographically sortable)
# Output format: "source:line" or empty if not found
find_recent_file_log_match() {
    local pattern="$1"

    # min timestamp as string (1 minute ago)
    local min_ts
    min_ts=$(date -v-1M "+%Y-%m-%d %H:%M:%S" 2>/dev/null || date -d "1 minute ago" "+%Y-%m-%d %H:%M:%S")

    shopt -s nullglob
    for log_file in logs/coordinator-*.log; do
        [[ -f "$log_file" ]] || continue

        local found_line
        found_line=$(awk -v pattern="$pattern" -v min_ts="$min_ts" '
            $0 ~ pattern && substr($0, 1, 19) >= min_ts { print; exit }
        ' "$log_file")

        if [ -n "$found_line" ]; then
            echo "${log_file}:${found_line}"
            return 0
        fi
    done
}

# count transactions in blocks from start_height to end_height (excluding coinbase)
count_transactions_in_blocks() {
    local start_height=$1
    local end_height=$2
    local total_txs=0
    for ((h=start_height + 1; h<=end_height; h++)); do
        local stats
        stats=$(bitcoin-cli -regtest -rpcuser=foo -rpcpassword=rpcpassword \
                getblockstats "$h" 2>/dev/null) || continue
        # Extract "txs" (total tx count including coinbase) - use jq if available, fallback to sed
        local total_tx_count
        total_tx_count=$(jq -r '.txs // empty' 2>/dev/null <<<"$stats" || echo "$stats" | tr -d '\n' | sed -E 's/.*"txs"[[:space:]]*:[[:space:]]*([0-9]+).*/\1/')
        # Subtract 1 to exclude the coinbase
        if [ -n "$total_tx_count" ] && [ "$total_tx_count" -gt 1 ]; then
            local non_coinbase_count=$((total_tx_count - 1))
            total_txs=$((total_txs + non_coinbase_count))
        fi
        # If total_tx_count <= 1, there's only coinbase, so we add 0
    done
    echo "$total_txs"
}

# wait for N bitcoin transactions to be created in mined blocks with confirmations
# expected_count: number of transactions to wait for
# max_blocks: maximum blocks to wait for transactions to appear
# confirmations: number of confirmations required (blocks after transaction is mined)
wait_for_bitcoin_transactions() {
    local expected_count=$1
    local max_blocks=$2
    local confirmations=$3
    local start_height=$(get_current_bitcoin_height)

    # target height accounts for finding transactions + getting confirmations
    local target_height=$((start_height + max_blocks + confirmations))
    local first_tx_height=0  # block height where transactions were first found

    log "Waiting for $expected_count Bitcoin transactions in mined blocks (max $max_blocks blocks to find) with $confirmations confirmations..."
    log "Starting height: $start_height, Max target height: $target_height"

    while true; do
        local current_height=$(get_current_bitcoin_height)
        local blocks_mined=$((current_height - start_height))

        # safeguard
        if [ $blocks_mined -lt 0 ]; then
            sleep 1
            continue
        fi

        # count transactions in blocks from start_height+1 to current_height
        local transactions_found=0
        if [ $blocks_mined -gt 0 ]; then
            transactions_found=$(count_transactions_in_blocks $start_height $current_height)
        fi

        # track the first block height where we found the expected count of transactions
        if [ $transactions_found -ge $expected_count ] && [ $first_tx_height -eq 0 ]; then
            first_tx_height=$current_height
            log "Found $expected_count transactions at height $first_tx_height, waiting for $confirmations confirmations..."
        fi

        # calculate confirmations if we've found transactions
        local current_confirmations=0
        if [ $first_tx_height -gt 0 ]; then
            current_confirmations=$((current_height - first_tx_height))
        fi

        if [ $first_tx_height -gt 0 ]; then
            echo -ne "\r  Blocks mined: $blocks_mined | Transactions: $transactions_found/$expected_count | Confirmations: $current_confirmations/$confirmations  "
        else
            echo -ne "\r  Blocks mined: $blocks_mined/$max_blocks | Transactions found: $transactions_found/$expected_count  "
        fi

        # check if we have enough transactions AND enough confirmations
        if [ $transactions_found -ge $expected_count ] && [ $first_tx_height -gt 0 ] && [ $current_confirmations -ge $confirmations ]; then
            echo ""  # newline after the progress display
            success "$expected_count Bitcoin transactions found with $confirmations confirmations (height: $start_height -> $current_height, tx at $first_tx_height)"
            return 0
        fi

        # Check if we've exceeded the block limit before finding transactions
        if [ $current_height -ge $target_height ] && [ $first_tx_height -eq 0 ]; then
            echo ""  # newline after the progress display
            warn "Only $transactions_found/$expected_count Bitcoin transactions found after $max_blocks blocks (height: $start_height -> $current_height)"
            return 1
        fi

        # Check if we found transactions but didn't get enough confirmations in time
        if [ $first_tx_height -gt 0 ] && [ $current_height -ge $target_height ]; then
            echo ""  # newline after the progress display
            warn "Found $expected_count transactions at height $first_tx_height but only got $current_confirmations/$confirmations confirmations (timeout at height $target_height)"
            return 1
        fi

        sleep 0.5
    done
}

# wait for N bitcoin blocks to be mined
wait_for_bitcoin_blocks() {
    local count=$1
    local start_height=$(get_current_bitcoin_height)
    local target_height=$((start_height + count))

    log "Waiting for $count Bitcoin blocks to be mined..."

    while true; do
        local current_height=$(get_current_bitcoin_height)
        local blocks_mined=$((current_height - start_height))
        # safeguard
        if [ $blocks_mined -lt 0 ]; then
            continue
        fi

        echo -ne "\r  Bitcoin Blocks mined: $blocks_mined/$count (current height: $current_height)  "

        if [ $current_height -ge $target_height ]; then
            echo ""  # newline after the progress display
            success "$count Bitcoin blocks mined (height: $start_height -> $current_height)"
            break
        fi

        sleep 0.25s
    done
    echo ""
}

# Wait for a log pattern to appear, with a block-based timeout
# Usage: wait_for_log_with_block_timeout <pattern> <max_blocks>
wait_for_log_with_block_timeout() {
    local pattern="$1"
    local max_blocks=$2

    local start_height=$(get_current_bitcoin_height)
    local target_height=$((start_height + max_blocks))

    log "Waiting for log pattern: $pattern (max $max_blocks blocks)..."

    while true; do
        local current_height=$(get_current_bitcoin_height)
        local blocks_mined=$((current_height - start_height))

        # safeguard
        if [ $blocks_mined -lt 0 ]; then
            sleep 1
            continue
        fi

        echo -ne "\r  Blocks mined: $blocks_mined/$max_blocks | Checking logs...  "

        # Check for log pattern in coordinator logs
        local result=""
        if [[ "$SCRIPT_ENV" == "local-docker" ]]; then
            result=$(find_recent_docker_log_match "$pattern")
        else
            result=$(find_recent_file_log_match "$pattern")
        fi

        if [ -n "$result" ]; then
            # parse "source:line" format
            local found_source="${result%%:*}"
            local found_line="${result#*:}"
            echo ""  # newline after the progress display
            success "Log pattern found after $blocks_mined blocks!"
            log "Found in: $found_source"
            echo "$found_line"
            return 0
        fi

        # Check if we've exceeded the block limit
        if [ $current_height -ge $target_height ]; then
            echo ""  # newline after the progress display
            warn "Log pattern not found after $max_blocks blocks (height: $start_height -> $current_height)"
            if [[ "$SCRIPT_ENV" == "local-docker" ]]; then
                warn "Check Docker logs manually: docker compose -p op_{1..4} logs coordinator"
            else
                warn "Check logs/coordinator-*.log manually"
            fi
            return 1
        fi

        sleep 1
    done
}

wait_for_log_in_all_operators() {
    local pattern="$1"
    local max_blocks=$2
    local operator_count=${3:-4}

    local start_height=$(get_current_bitcoin_height)
    local target_height=$((start_height + max_blocks))

    log "Waiting for log pattern in all $operator_count operators: $pattern (max $max_blocks blocks)..."

    declare -A found_operators=()

    while true; do
        local current_height=$(get_current_bitcoin_height)
        local blocks_mined=$((current_height - start_height))

        if [ $blocks_mined -lt 0 ]; then
            sleep 1
            continue
        fi

        local found_count="${#found_operators[@]}"
        echo -ne "\r  Blocks mined: $blocks_mined/$max_blocks | Operators matched: $found_count/$operator_count  "

        if [[ "$SCRIPT_ENV" == "local-docker" ]]; then
            for op_id in $(seq 1 $operator_count); do
                [[ -n "${found_operators[$op_id]:-}" ]] && continue
                local project="op_${op_id}"
                local line
                line=$(docker compose -p "$project" logs coordinator 2>/dev/null | grep -E "$pattern" | tail -1)
                if [ -n "$line" ]; then
                    found_operators[$op_id]="$line"
                    echo ""
                    success "Pattern found in $project"
                    echo "  $line"
                fi
            done
        else
            local min_ts
            min_ts=$(date -v-1M "+%Y-%m-%d %H:%M:%S" 2>/dev/null || date -d "1 minute ago" "+%Y-%m-%d %H:%M:%S")

            for op_id in $(seq 1 $operator_count); do
                [[ -n "${found_operators[$op_id]:-}" ]] && continue
                local log_file="logs/coordinator-${op_id}.log"
                [[ -f "$log_file" ]] || continue
                local found_line
                found_line=$(awk -v pattern="$pattern" -v min_ts="$min_ts" '
                    $0 ~ pattern && substr($0, 1, 19) >= min_ts { print; exit }
                ' "$log_file")
                if [ -n "$found_line" ]; then
                    found_operators[$op_id]="$found_line"
                    echo ""
                    success "Pattern found in coordinator-${op_id}"
                    echo "  $found_line"
                fi
            done
        fi

        if [ ${#found_operators[@]} -ge $operator_count ]; then
            echo ""
            success "Log pattern found in all $operator_count operators after $blocks_mined blocks!"
            return 0
        fi

        if [ $current_height -ge $target_height ]; then
            echo ""
            warn "Log pattern not found in all operators after $max_blocks blocks (height: $start_height -> $current_height)"
            warn "Missing operators:"
            for op_id in $(seq 1 $operator_count); do
                if [[ -z "${found_operators[$op_id]:-}" ]]; then
                    warn "  - operator $op_id"
                fi
            done
            return 1
        fi

        sleep 1
    done
}

# Wait for a log pattern to appear, with a time-based timeout (seconds).
# Useful when mining is stopped and block-based timeouts don't progress.
wait_for_log_with_time_timeout() {
    local pattern="$1"
    local timeout_secs=$2
    local start_time=$(date +%s)

    log "Waiting for log pattern: $pattern (max ${timeout_secs}s)..."

    while true; do
        local elapsed=$(( $(date +%s) - start_time ))

        echo -ne "\r  Elapsed: ${elapsed}s/${timeout_secs}s | Checking logs...  "

        local result=""
        if [[ "$SCRIPT_ENV" == "local-docker" ]]; then
            result=$(find_recent_docker_log_match "$pattern")
        else
            result=$(find_recent_file_log_match "$pattern")
        fi

        if [ -n "$result" ]; then
            local found_source="${result%%:*}"
            local found_line="${result#*:}"
            echo ""
            success "Log pattern found after ${elapsed}s!"
            log "Found in: $found_source"
            echo "$found_line"
            return 0
        fi

        if [ $elapsed -ge $timeout_secs ]; then
            echo ""
            warn "Log pattern not found after ${timeout_secs}s"
            if [[ "$SCRIPT_ENV" == "local-docker" ]]; then
                warn "Check Docker logs manually: docker compose -p op_{1..4} logs coordinator"
            else
                warn "Check logs/coordinator-*.log manually"
            fi
            return 1
        fi

        sleep 1
    done
}

cleanup() {
    bash cli-infra.sh --stop-mine 2>/dev/null || true
    rm -f /tmp/apply-operators-$$ /tmp/pegout-$$
}
trap cleanup EXIT

clear
log "Configuration: stream=$STREAM_ID, rsk=$RSK_ADDRESS, amount=$VALUE, env=$SCRIPT_ENV"
log "Mining will be managed automatically by this script"
echo ""

# prepare wallets
step "Step 0: Prepare Wallets"
log "Clearing wallet databases and mining initial UTXOs..."
log "Note: First run compiles release binaries for bitcoin-wallet (~1 min), subsequent runs are fast"
echo ""

log "User wallet: clear_db"
bash cli-bitcoin-wallet.sh user clear_db || warn "User wallet clear_db failed (may be expected if db is empty)"

log "Member wallet: clear_db"
bash cli-bitcoin-wallet.sh member clear_db || warn "Member wallet clear_db failed (may be expected if db is empty)"

log "User wallet: mine_utxo 900000000"
if ! bash cli-bitcoin-wallet.sh user mine_utxo 900000000; then
  warn "User wallet mine_utxo failed!"
  exit 1
fi

log "Member wallet: mine_utxo 900000000"
if ! bash cli-bitcoin-wallet.sh member mine_utxo 900000000; then
  warn "Member wallet mine_utxo failed!"
  exit 1
fi

success "Wallets prepared with initial UTXOs"
echo ""

# start auto-mining for the pegin flow
bash cli-infra.sh --stop-mine 2>/dev/null || true
log "Starting auto-mining..."
if ! bash cli-infra.sh --start-mine; then
    warn "Failed to start mining"
    exit 1
fi
success "Auto-mining started"
echo ""

# step 1: fund operator wallets
step "Step 1: Fund Operator Wallets"
# operator fund with --execute handles BitVMX funding automatically
# Must pass $SCRIPT_ENV so it knows where to find the BitVMX addresses (local logs vs Docker logs)
log "Command: bash cli-operations.sh operator fund --env $SCRIPT_ENV --execute"
echo ""
if ! bash cli-operations.sh operator fund --env "$SCRIPT_ENV" --execute; then
    warn "Command failed!"
    exit 1
fi
echo ""
if ! wait_for_bitcoin_transactions 1 15 5; then
    warn "Failed to detect 1 Bitcoin transaction with 5 confirmations within 15 blocks"
    exit 1
fi
echo ""
success "Operator wallets funded (including BitVMX)"

# step 2: whitelist member addresses on CommitteeRegistry
step "Step 2: Whitelist Member Addresses"
log "Command: bash cli-operations.sh operator whitelist --env $SCRIPT_ENV --contract-address $COMMITTEE_REGISTRY_ADDRESS"
echo ""
if ! bash cli-operations.sh operator whitelist --env "$SCRIPT_ENV" \
    --contract-address "$COMMITTEE_REGISTRY_ADDRESS"; then
    warn "Whitelist command failed!"
    exit 1
fi
success "Member addresses whitelisted"
echo ""

# step 3: apply operators
step "Step 3: Apply Operators to Stream"
log "Command: bash cli-operations.sh operator apply-stream -s $STREAM_ID --env $SCRIPT_ENV"
echo ""
if ! bash cli-operations.sh operator apply-stream -s $STREAM_ID --env "$SCRIPT_ENV" > /tmp/apply-operators-$$ 2>&1; then
    warn "Command failed! Output:"
    cat /tmp/apply-operators-$$
    rm -f /tmp/apply-operators-$$
    exit 1
fi
rm -f /tmp/apply-operators-$$
success "Operators applied to stream $STREAM_ID"
echo ""
if ! wait_for_log_in_all_operators "CommitteeSetupFlow Done:" 60; then
    warn "Committee setup not completed by all operators within timeout"
    exit 1
fi
echo ""

# step 4: request pegin
step "Step 4: Request Pegin"
log "RSK Address: $RSK_ADDRESS"
log "Amount: $VALUE sats"
log "Command: bash cli-operations.sh user pegin -a $RSK_ADDRESS -v $VALUE --env $SCRIPT_ENV --execute"
echo ""
if ! bash cli-operations.sh user pegin -a $RSK_ADDRESS -v $VALUE --env "$SCRIPT_ENV" --execute; then
    warn "Command failed!"
    exit 1
fi
success "Pegin transaction created"
echo ""
if ! wait_for_log_with_block_timeout "PeginFlow Done" 15; then
    warn "PeginFlow completion log not found within timeout"
    exit 1
fi
echo ""

# stop auto-mining before pegout
log "Stopping auto-mining..."
if ! bash cli-infra.sh --stop-mine; then
    warn "Failed to stop mining"
    exit 1
fi
success "Auto-mining stopped"
echo ""

# step 5: request pegout (with mining stopped)
step "Step 5: Request Pegout (mining stopped)"
log "Command: bash cli-operations.sh user pegout -v $VALUE --env $SCRIPT_ENV"
log "Amount: $VALUE sats"
echo ""

if ! bash cli-operations.sh user pegout -v $VALUE --env "$SCRIPT_ENV" > /tmp/pegout-$$ 2>&1; then
    warn "Command failed! Output:"
    cat /tmp/pegout-$$
    rm -f /tmp/pegout-$$
    exit 1
fi
rm -f /tmp/pegout-$$
success "Pegout requested"
echo ""

# step 6: mine RSK blocks manually
step "Step 6: Mine RSK blocks manually"
log "Mining 4 RSK blocks with 1s interval..."
for i in 1 2 3 4; do
    cast rpc anvil_mine 1 --rpc-url http://localhost:8545 > /dev/null 2>&1
    success "RSK block $i/4 mined"
    if [ $i -lt 4 ]; then
        sleep 1
    fi
done
echo ""
success "4 RSK blocks mined"
echo ""

# step 7: wait for PegOutAccepted from BitVMX
step "Step 7: Wait for PegOutAccepted"
log "Waiting for 'Received PegOutAccepted variable from BitVMX' in logs..."
echo ""

if ! wait_for_log_with_time_timeout "Received PegOutAccepted variable from BitVMX" 120; then
    warn "PegOutAccepted log not found within timeout"
    exit 1
fi
echo ""

# step 8: kill one bitvmx operator (op_4)
step "Step 8: Kill BitVMX operator 4"
BITVMX_PID=$(ps -eo pid,args | grep '[b]itvmx-client op_4' | awk '{print $1}')
if [ -z "$BITVMX_PID" ]; then
    warn "Could not find bitvmx-client op_4 process"
    exit 1
fi
log "Killing bitvmx-client op_4 (PID: $BITVMX_PID)..."
kill -9 "$BITVMX_PID"
success "bitvmx-client op_4 killed (PID: $BITVMX_PID)"

REMAINING=$(ps -eo pid,args | grep '[b]itvmx-client op_' | wc -l | tr -d ' ')
if [ "$REMAINING" -ne 3 ]; then
    warn "Expected 3 bitvmx-client operators running, found $REMAINING"
    exit 1
fi
success "Verified: $REMAINING bitvmx-client operators still running"
echo ""

# restart auto-mining
bash cli-infra.sh --stop-mine 2>/dev/null || true
log "Restarting auto-mining..."
if ! bash cli-infra.sh --start-mine; then
    warn "Failed to restart mining"
    exit 1
fi
success "Auto-mining restarted"
echo ""


# step 9: wait for RegisterOperatorTake completion
step "Step 9: Wait for RegisterOperatorTake completion"
log "The operator take process is now running with 3 BitVMX operators (op_4 was killed)."
log "Waiting for 'RegisterOperatorTake -> Done' in logs..."
echo ""

if ! wait_for_log_with_time_timeout "RegisterOperatorTake -> Done" 240; then
    warn "RegisterOperatorTake -> Done log not found within timeout"
    exit 1
fi
success "RegisterOperatorTake completed successfully"
echo ""


step "Complete"
success "E2E operator take path test completed successfully!"
