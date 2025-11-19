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

# change to project root (parent of tests directory)
cd "$(dirname "$0")/.."

# hardcoded configuration
STREAM_ID=0
RSK_ADDRESS="0x$(openssl rand -hex 20)"  # random address each run
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

cleanup() {
    [ -n "${ANVIL_MINE_PID:-}" ] && kill $ANVIL_MINE_PID 2>/dev/null || true
    [ -n "${BITCOIN_MINE_PID:-}" ] && kill $BITCOIN_MINE_PID 2>/dev/null || true
    rm -f /tmp/apply-operators-$$ /tmp/pegout-$$
}
trap cleanup EXIT INT TERM

clear
log "Configuration: stream=$STREAM_ID, rsk=$RSK_ADDRESS, amount=$VALUE"
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
step "Step 1: Fund Operator Wallets"
log "Command: bash cli-operations.sh operator fund --env local --execute"
echo ""
if ! bash cli-operations.sh operator fund --env local --execute; then
    warn "Command failed!"
    exit 1
fi
success "Operator wallets funded"
echo ""
log "Allowing time (blocks) for BitVMX to detect confirmed transactions..."
wait_for_bitcoin_blocks 6
echo ""

# step 2: apply operators
step "Step 2: Apply Operators to Stream"
log "Command: bash cli-operations.sh operator apply-stream -s $STREAM_ID --env local"
echo ""
if ! bash cli-operations.sh operator apply-stream -s $STREAM_ID --env local > /tmp/apply-operators-$$ 2>&1; then
    warn "Command failed! Output:"
    cat /tmp/apply-operators-$$
    rm -f /tmp/apply-operators-$$
    exit 1
fi
rm -f /tmp/apply-operators-$$
success "Operators applied to stream $STREAM_ID"
echo ""
wait_for_bitcoin_blocks 15
echo ""

# step 3: request pegin
step "Step 3: Request Pegin"
log "RSK Address: $RSK_ADDRESS"
log "Amount: $VALUE sats"
log "Packet: $PACKET_NUMBER"
log "Command: bash cli-operations.sh user pegin -a $RSK_ADDRESS -v $VALUE -p $PACKET_NUMBER --env local --execute"
echo ""
if ! bash cli-operations.sh user pegin -a $RSK_ADDRESS -v $VALUE -p $PACKET_NUMBER --env local --execute; then
    warn "Command failed!"
    exit 1
fi
success "Pegin transaction created"
echo ""
wait_for_bitcoin_blocks 15
echo ""

# step 4: request pegout
step "Step 4: Request Pegout"
log "Command: bash cli-operations.sh user pegout -v $VALUE --env local"
log "Amount: $VALUE sats"
echo ""

# capture current time (with margin for clock differences)
PEGOUT_START_TIME=$(date +%s)

if ! bash cli-operations.sh user pegout -v $VALUE --env local > /tmp/pegout-$$ 2>&1; then
    warn "Command failed! Output:"
    cat /tmp/pegout-$$
    rm -f /tmp/pegout-$$
    exit 1
fi
rm -f /tmp/pegout-$$
success "Pegout requested"
echo ""

wait_for_bitcoin_blocks 15
echo ""

# step 5: verify pegout completion
step "Step 5: Verify Pegout Completion"
log "Checking logs/coordinator-1.log for PegoutFlow completion..."
echo ""

# search for PegoutFlow completion with recent timestamp
# pattern: "2025-11-12 16:54:39.725 [ INFO] [...] PegoutFlow <uuid>: Done"
# allow 5 minutes margin (300 seconds) before pegout start for clock differences
TIME_MARGIN=300
MIN_TIME=$((PEGOUT_START_TIME - TIME_MARGIN))

MATCHING_LINE=$(grep -E "PegoutFlow [0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}: Done" logs/coordinator-1.log 2>/dev/null | while read -r line; do
    # extract timestamp from log line (format: "2025-11-12 16:54:39")
    LOG_TIMESTAMP=$(echo "$line" | grep -oE '^[0-9]{4}-[0-9]{2}-[0-9]{2} [0-9]{2}:[0-9]{2}:[0-9]{2}')
    
    if [ -n "$LOG_TIMESTAMP" ]; then
        # convert to epoch seconds (macOS and Linux compatible)
        LOG_TIME=$(date -j -f "%Y-%m-%d %H:%M:%S" "$LOG_TIMESTAMP" +%s 2>/dev/null || date -d "$LOG_TIMESTAMP" +%s 2>/dev/null || echo "0")
        
        # check if log is recent (after MIN_TIME)
        if [ "$LOG_TIME" -ge "$MIN_TIME" ]; then
            echo "$line"
            break
        fi
    fi
done | tail -1)

if [ -n "$MATCHING_LINE" ]; then
    success "PegoutFlow completed successfully!"
    echo ""
    log "Found in logs:"
    echo "$MATCHING_LINE"
    SUCCESS=true
else
    warn "PegoutFlow completion not detected in logs"
    warn "Check logs/coordinator-1.log manually"
    SUCCESS=false
fi
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
