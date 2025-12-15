#!/usr/bin/env bash

# happy path e2e test - fully automated local flow (no manual intervention)
#
# prerequisites:
#   - union bridge clients running (via: cargo run -- run)
#   - anvil running on localhost:8545
#   - bitcoin regtest node running with RPC enabled
#   - USER_BITCOIN_WIF and MEMBER_BITCOIN_WIF environment variables set
#
# usage: bash tests/run-happy-path.sh

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
PACKET_NUMBER=0

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

# mining functions
mine_anvil() {
    # test first call
    if ! cast rpc anvil_mine 1 --rpc-url http://localhost:8545 &>/dev/null; then
        echo -e "${YELLOW}[WARN]${NC} Anvil mining failed - is Anvil running?" >&2
        return 1
    fi

    # mine indefinitely
    while true; do
        cast rpc anvil_mine 1 --rpc-url http://localhost:8545 &>/dev/null || true
        sleep 1
    done
}

mine_bitcoin() {
    # get address to mine to (once)
    local mine_address=$(bitcoin-cli -regtest -rpcuser=foo -rpcpassword=rpcpassword getnewaddress 2>/dev/null)
    if [ -z "$mine_address" ]; then
        echo -e "${YELLOW}[WARN]${NC} Failed to get Bitcoin address for mining" >&2
        return 1
    fi

    # mine indefinitely
    while true; do
        bitcoin-cli -regtest -rpcuser=foo -rpcpassword=rpcpassword generatetoaddress 1 "$mine_address" &>/dev/null || true
        sleep 5
    done
}

# Helper function to extract timestamp from log line
extract_log_timestamp() {
    local line="$1"
    local mode="$2"  # "docker" or "file"
    
    if [ "$mode" = "docker" ]; then
        # Docker logs format: "2025-11-19 20:22:10.394 | 2025-11-19 23:22:10.394 [INFO] ..."
        # We want the second timestamp (after the pipe)
        echo "$line" | grep -oE '\| [0-9]{4}-[0-9]{2}-[0-9]{2} [0-9]{2}:[0-9]{2}:[0-9]{2}' | grep -oE '[0-9]{4}-[0-9]{2}-[0-9]{2} [0-9]{2}:[0-9]{2}:[0-9]{2}'
    else
        # File logs format: "^2025-11-19 23:22:10.394 [INFO] ..."
        echo "$line" | grep -oE '^[0-9]{4}-[0-9]{2}-[0-9]{2} [0-9]{2}:[0-9]{2}:[0-9]{2}'
    fi
}

# Helper function to convert timestamp to epoch (macOS and Linux compatible)
# Returns empty string on failure (caller should handle this explicitly)
timestamp_to_epoch() {
    local timestamp="$1"
    date -j -f "%Y-%m-%d %H:%M:%S" "$timestamp" +%s 2>/dev/null || \
    date -d "$timestamp" +%s 2>/dev/null || \
    echo ""
}

# Unified function to find recent log matches
# Uses pipes to avoid loading entire logs into memory
find_recent_log_match() {
    local pattern="$1"
    local source="$2"
    local min_time="$3"
    local mode="${4:-file}"  # default to file mode
    
    local log_stream
    if [ "$mode" = "docker" ]; then
        log_stream=$(docker compose -p "$source" logs coordinator 2>/dev/null) || return 1
    else
        log_stream=$(cat "$source" 2>/dev/null) || return 1
    fi
    
    echo "$log_stream" | grep -E "$pattern" | while read -r line; do
        local log_timestamp=$(extract_log_timestamp "$line" "$mode")
        
        if [ -n "$log_timestamp" ]; then
            local log_time=$(timestamp_to_epoch "$log_timestamp")
            
            # check if log is recent (after min_time)
            # skip if timestamp parsing failed (empty string)
            if [ -n "$log_time" ] && [ "$log_time" -ge "$min_time" ]; then
                echo "$line"
                break
            fi
        fi
    done | tail -1
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
# start_height: the last block mined before transactions should start appearing
# expected_count: number of transactions to wait for
# max_blocks: maximum blocks to wait for transactions to appear
# confirmations: number of confirmations required (blocks after transaction is mined)
wait_for_bitcoin_transactions() {
    local start_height=$1
    local expected_count=$2
    local max_blocks=$3
    local confirmations=$4
    
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
# Automatically determines start_time based on mode (docker container time or host time)
wait_for_log_with_block_timeout() {
    local pattern="$1"
    local max_blocks=$2

    # determine start_time based on mode
    # for docker mode, use container's time as reference to avoid timezone issues
    # for file mode, use host time
    local start_time
    if [[ "$SCRIPT_ENV" == "local-docker" ]]; then
        # get time from container (all containers share the same time)
        start_time=$(docker compose -p "op_1" exec -T coordinator date +%s 2>/dev/null || echo "")
        # fallback to host time if container time unavailable
        start_time=${start_time:-$(date +%s)}
    else
        start_time=$(date +%s)
    fi

    # allow 1 minute margin (60 seconds) before start_time for clock differences
    local TIME_MARGIN=60
    local min_time=$((start_time - TIME_MARGIN))

    local start_height=$(get_current_bitcoin_height)
    local target_height=$((start_height + max_blocks))

    # Convert min_time to formatted date string
    local min_time_formatted=$(date -d "@$min_time" "+%Y-%m-%d %H:%M:%S" 2>/dev/null || date -r "$min_time" "+%Y-%m-%d %H:%M:%S")
    log "Waiting for log pattern: $pattern (max $max_blocks blocks, after $min_time_formatted)..."
    
    while true; do
        local current_height=$(get_current_bitcoin_height)
        local blocks_mined=$((current_height - start_height))
        
        # safeguard
        if [ $blocks_mined -lt 0 ]; then
            sleep 1
            continue
        fi
        
        echo -ne "\r  Blocks mined: $blocks_mined/$max_blocks | Checking logs...  "
        
        # Check for log pattern in all coordinator logs
        local found_line=""
        local found_source=""
        
        if [[ "$SCRIPT_ENV" == "local-docker" ]]; then
            for op_id in 1 2 3 4; do
                project="op_${op_id}"
                found_line=$(find_recent_log_match "$pattern" "$project" "$min_time" "docker")
                if [ -n "$found_line" ]; then
                    found_source="${project}"
                    break
                fi
            done
        else
            shopt -s nullglob
            for log_file in logs/coordinator-*.log; do
                [[ -f "$log_file" ]] || continue
                
                found_line=$(find_recent_log_match "$pattern" "$log_file" "$min_time" "file")
                if [ -n "$found_line" ]; then
                    found_source="$log_file"
                    break
                fi
            done
        fi
        
        if [ -n "$found_line" ]; then
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

cleanup() {
    echo ""
    log "Cleaning up background processes..."
    # kill background mining processes
    if [ -n "${ANVIL_MINE_PID:-}" ] && kill -0 "$ANVIL_MINE_PID" 2>/dev/null; then
        kill -TERM "$ANVIL_MINE_PID" 2>/dev/null || true
        sleep 0.2
        kill -9 "$ANVIL_MINE_PID" 2>/dev/null || true
    fi
    if [ -n "${BITCOIN_MINE_PID:-}" ] && kill -0 "$BITCOIN_MINE_PID" 2>/dev/null; then
        kill -TERM "$BITCOIN_MINE_PID" 2>/dev/null || true
        sleep 0.2
        kill -9 "$BITCOIN_MINE_PID" 2>/dev/null || true
    fi
    rm -f /tmp/apply-operators-$$ /tmp/pegout-$$
}

# handle Ctrl+C immediately - kill background processes and exit
handle_interrupt() {
    echo ""
    echo ""
    warn "Interrupted by user (Ctrl+C)"
    cleanup
    exit 130  # standard exit code for SIGINT
}
trap handle_interrupt INT TERM
trap cleanup EXIT

clear
log "Configuration: stream=$STREAM_ID, rsk=$RSK_ADDRESS, amount=$VALUE, env=$SCRIPT_ENV"
log "Background mining: Anvil (every 1s) | Bitcoin (every 5s) - runs until Ctrl+C"
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

# start background mining (runs indefinitely)
mine_anvil &
ANVIL_MINE_PID=$!

mine_bitcoin &
BITCOIN_MINE_PID=$!

sleep 1  # give mining a moment to start

# step 1: fund operator wallets
# capture block height before funding starts (last block mined before transactions should appear)
FUND_START_HEIGHT=$(get_current_bitcoin_height)

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
if ! wait_for_bitcoin_transactions "$FUND_START_HEIGHT" 1 15 5; then
    warn "Failed to detect 1 Bitcoin transaction with 5 confirmations within 15 blocks"
    exit 1
fi
echo ""
success "Operator wallets funded (including BitVMX)"

# step 2: apply operators
step "Step 2: Apply Operators to Stream"
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
if ! wait_for_log_with_block_timeout "CommitteeSetupFlow Done" 15; then
    warn "Committee setup complete log not found within timeout"
    exit 1
fi
echo ""

# step 3: request pegin
step "Step 3: Request Pegin"
log "RSK Address: $RSK_ADDRESS"
log "Amount: $VALUE sats"
log "Packet: $PACKET_NUMBER"
log "Command: bash cli-operations.sh user pegin -a $RSK_ADDRESS -v $VALUE -p $PACKET_NUMBER --env $SCRIPT_ENV --execute"
echo ""
if ! bash cli-operations.sh user pegin -a $RSK_ADDRESS -v $VALUE -p $PACKET_NUMBER --env "$SCRIPT_ENV" --execute; then
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

# step 4: request pegout
step "Step 4: Request Pegout"
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

if ! wait_for_log_with_block_timeout "PegoutFlow Done" 15; then
    warn "PegoutFlow completion log not found within timeout"
    exit 1
fi
echo ""

# step 5: verify pegout completion
step "Step 5: Verify Pegout Completion"
# Note: PegoutFlow completion is already checked in the wait_for_log_with_block_timeout call above
# This step is kept for consistency but the verification already happened
success "PegoutFlow completion verified"
SUCCESS=true
echo ""

step "Complete"
log "Stopping background mining processes..."
echo ""

# stop mining processes
[ -n "${ANVIL_MINE_PID:-}" ] && kill $ANVIL_MINE_PID 2>/dev/null || true
[ -n "${BITCOIN_MINE_PID:-}" ] && kill $BITCOIN_MINE_PID 2>/dev/null || true

if [ "$SUCCESS" = true ]; then
    success "E2E test completed successfully!"
else
    warn "E2E test completed with warnings - PegoutFlow completion not verified"
    exit 1
fi
