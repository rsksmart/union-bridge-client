#!/usr/bin/env bash

# Script to complete all steps after a pegout is executed outside this repo
#
# This script assumes a pegout transaction has already been created/executed externally.
# It will:
#   - Start background mining (Anvil and Bitcoin)
#   - Wait for blocks to allow the pegout flow to process
#   - Monitor logs for PegoutFlow completion
#   - Stop mining when completion is detected
#
# prerequisites:
#   - union bridge clients running (via: cargo run -- run)
#   - anvil running on localhost:8545
#   - bitcoin regtest node running with RPC enabled
#   - A pegout transaction has already been executed externally
#
# usage: bash tests/complete-after-pegout.sh [--env <local|local-docker>] [--wait-blocks <N>]

set -euo pipefail

SCRIPT_ENV="local"
INITIAL_WAIT_BLOCKS=15

usage() {
    echo "Usage: $0 [--env <local|local-docker>] [--wait-blocks <N>]"
    echo ""
    echo "Options:"
    echo "  --env <local|local-docker>  Environment type (default: local)"
    echo "  --wait-blocks <N>           Number of blocks to wait initially (default: 15)"
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
    --wait-blocks)
        INITIAL_WAIT_BLOCKS="${2:-}"
        if [[ -z "$INITIAL_WAIT_BLOCKS" ]] || ! [[ "$INITIAL_WAIT_BLOCKS" =~ ^[0-9]+$ ]]; then
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

# check prerequisites
if ! command -v cargo &> /dev/null || ! command -v cast &> /dev/null || ! command -v bitcoin-cli &> /dev/null; then
    echo "Error: cargo, cast, and bitcoin-cli required"
    exit 1
fi

if ! cast rpc eth_chainId --rpc-url http://localhost:8545 &> /dev/null; then
    echo "Error: Anvil not running on localhost:8545"
    exit 1
fi

if ! bitcoin-cli -regtest -rpcuser=foo -rpcpassword=rpcpassword getblockcount &> /dev/null; then
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

# wait for N bitcoin blocks to be mined
wait_for_bitcoin_blocks() {
    local count=$1
    local start_height=$(bitcoin-cli -regtest -rpcuser=foo -rpcpassword=rpcpassword getblockcount 2>/dev/null || echo "0")
    start_height=${start_height:-0}  # ensure it's set to 0 if empty
    local target_height=$((start_height + count))

    log "Waiting for $count Bitcoin blocks to be mined..."

    while true; do
        local current_height=$(bitcoin-cli -regtest -rpcuser=foo -rpcpassword=rpcpassword getblockcount 2>/dev/null || echo "0")
        current_height=${current_height:-0}  # ensure it's set to 0 if empty
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

        sleep 1
    done
    echo ""
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
timestamp_to_epoch() {
    local timestamp="$1"
    date -j -f "%Y-%m-%d %H:%M:%S" "$timestamp" +%s 2>/dev/null || \
    date -d "$timestamp" +%s 2>/dev/null || \
    echo "0"
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
            if [ "$log_time" -ge "$min_time" ]; then
                echo "$line"
                break
            fi
        fi
    done | tail -1
}

# Function to check for PegoutFlow completion
check_pegout_completion() {
    local min_time=$1
    local pattern="PegoutFlow [0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}: Done"
    local matching_line=""
    local matching_source=""

    if [[ "$SCRIPT_ENV" == "local-docker" ]]; then
        for op_id in 1 2 3 4; do
            project="op_${op_id}"
            found_line=$(find_recent_log_match "$pattern" "$project" "$min_time" "docker")
            if [ -n "$found_line" ]; then
                matching_line="$found_line"
                matching_source="${project}"
                break
            fi
        done
    else
        shopt -s nullglob
        for log_file in logs/coordinator-*.log; do
            [[ -f "$log_file" ]] || continue

            found_line=$(find_recent_log_match "$pattern" "$log_file" "$min_time" "file")
            if [ -n "$found_line" ]; then
                matching_line="$found_line"
                matching_source="$log_file"
                break
            fi
        done
    fi

    if [ -n "$matching_line" ]; then
        echo "$matching_source|$matching_line"
        return 0
    else
        return 1
    fi
}

cleanup() {
    [ -n "${ANVIL_MINE_PID:-}" ] && kill $ANVIL_MINE_PID 2>/dev/null || true
    [ -n "${BITCOIN_MINE_PID:-}" ] && kill $BITCOIN_MINE_PID 2>/dev/null || true
}
trap cleanup EXIT INT TERM

clear
log "Configuration: env=$SCRIPT_ENV, initial_wait_blocks=$INITIAL_WAIT_BLOCKS"
log "Background mining: Anvil (every 1s) | Bitcoin (every 5s) - runs until pegout completion"
log "Note: This script assumes a pegout transaction has already been executed externally"
echo ""

# Capture start time (with margin for clock differences)
# Use current time minus a margin to catch any recent pegout flows
SCRIPT_START_TIME=$(date +%s)
TIME_MARGIN=600  # 10 minutes margin to catch pegouts executed just before script start
MIN_TIME=$((SCRIPT_START_TIME - TIME_MARGIN))

# start background mining (runs indefinitely)
step "Starting Background Mining"
mine_anvil &
ANVIL_MINE_PID=$!

mine_bitcoin &
BITCOIN_MINE_PID=$!

sleep 1  # give mining a moment to start
success "Background mining started"
echo ""

# wait for initial blocks to allow pegout flow to process
step "Waiting for Initial Blocks"
log "Waiting for $INITIAL_WAIT_BLOCKS blocks to allow pegout flow to process..."
wait_for_bitcoin_blocks $INITIAL_WAIT_BLOCKS
echo ""

# monitor for pegout completion
step "Monitoring for PegoutFlow Completion"
log "Checking logs from all operators for PegoutFlow completion..."
if [[ "$SCRIPT_ENV" == "local-docker" ]]; then
    log "Checking Docker logs from operators op_1 through op_4..."
else
    log "Checking local logs from logs/coordinator-*.log..."
fi
echo ""

# Poll for completion with periodic status updates
POLL_INTERVAL=5
MAX_ITERATIONS=120  # 10 minutes max (120 * 5 seconds)
ITERATION=0
FOUND=false

while [ $ITERATION -lt $MAX_ITERATIONS ]; do
    if check_pegout_completion $MIN_TIME; then
        result=$(check_pegout_completion $MIN_TIME)
        matching_source=$(echo "$result" | cut -d'|' -f1)
        matching_line=$(echo "$result" | cut -d'|' -f2-)
        
        success "PegoutFlow completed successfully!"
        echo ""
        log "Found in: $matching_source"
        echo "$matching_line"
        FOUND=true
        break
    fi
    
    ITERATION=$((ITERATION + 1))
    if [ $((ITERATION % 12)) -eq 0 ]; then
        # Print status every minute (12 * 5 seconds)
        local elapsed=$((ITERATION * POLL_INTERVAL))
        log "Still waiting for PegoutFlow completion... (${elapsed}s elapsed)"
    fi
    
    sleep $POLL_INTERVAL
done

echo ""

if [ "$FOUND" = true ]; then
    SUCCESS=true
else
    warn "PegoutFlow completion not detected after waiting $((MAX_ITERATIONS * POLL_INTERVAL)) seconds"
    if [[ "$SCRIPT_ENV" == "local-docker" ]]; then
        warn "Check Docker logs manually: docker compose -p op_{1..4} logs coordinator"
    else
        warn "Check logs/coordinator-*.log manually"
    fi
    SUCCESS=false
fi

step "Complete"
log "Stopping background mining processes..."
echo ""

# stop mining processes
[ -n "${ANVIL_MINE_PID:-}" ] && kill $ANVIL_MINE_PID 2>/dev/null || true
[ -n "${BITCOIN_MINE_PID:-}" ] && kill $BITCOIN_MINE_PID 2>/dev/null || true

if [ "$SUCCESS" = true ]; then
    success "Pegout completion verified successfully!"
    exit 0
else
    warn "Script completed but PegoutFlow completion was not verified"
    exit 1
fi




