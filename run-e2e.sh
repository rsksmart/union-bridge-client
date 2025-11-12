#!/usr/bin/env bash

# automated local flow (no manual intervention for API calls)
# bitcoin wallet operations still need manual execution (pegin funding)
# 
# prerequisites:
#   - union bridge clients running (via: cargo run -- run)
#   - anvil running on localhost:8545
#   - bitcoin regtest node running with RPC enabled
#   - USER_BITCOIN_WIF environment variable set
#
# usage: bash run-e2e.sh

set -euo pipefail

# hardcoded configuration
STREAM_ID=0
RSK_ADDRESS="0x$(openssl rand -hex 20)"  # random address each run
AMOUNT=100000
PACKET_NUMBER=0
PEGOUT_AMOUNT=$AMOUNT

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
if ! command -v cargo &> /dev/null || ! command -v cast &> /dev/null; then
    echo "Error: cargo and cast required"
    exit 1
fi

if ! cast rpc eth_chainId --rpc-url http://localhost:8545 &> /dev/null; then
    echo "Error: Anvil not running on localhost:8545"
    exit 1
fi

# check wallet is accessible
if ! bash cli-bitcoin-wallet.sh user block_height &>/dev/null; then
    echo "Error: Bitcoin wallet not accessible"
    echo "Please ensure:"
    echo "  - USER_BITCOIN_WIF environment variable is set"
    echo "  - Bitcoin regtest node is running with RPC enabled"
    echo "  - Wallet configuration is correct"
    exit 1
fi

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
    # test if wallet is accessible
    if ! bash cli-bitcoin-wallet.sh user mine_block &>/dev/null; then
        echo -e "${YELLOW}[WARN]${NC} Bitcoin wallet mining failed - is the wallet configured?" >&2
        return 1
    fi
    
    # mine indefinitely using programmatic wallet access
    while true; do
        bash cli-bitcoin-wallet.sh user mine_block &>/dev/null || true
        sleep 5
    done
}

# wait for N bitcoin blocks to be mined
wait_for_bitcoin_blocks() {
    local count=$1
    local start_height=$(bash cli-bitcoin-wallet.sh user block_height 2>/dev/null | tail -1 || echo "0")
    local target_height=$((start_height + count))
    
    log "Waiting for $count Bitcoin blocks to be mined..."
    
    while true; do
        local current_height=$(bash cli-bitcoin-wallet.sh user block_height 2>/dev/null | tail -1 || echo "0")
        local blocks_mined=$((current_height - start_height))
        
        if [ $current_height -ge $target_height ]; then
            success "$count Bitcoin blocks mined (height: $start_height → $current_height)"
            break
        fi
        
        echo -ne "\r  Bitcoin Blocks mined: $blocks_mined/$count (current height: $current_height)  "
        sleep 2
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
log "Configuration: stream=$STREAM_ID, rsk=$RSK_ADDRESS, amount=$AMOUNT"
log "Background mining: Anvil (every 1s) | Bitcoin (every 5s) - runs until Ctrl+C"
echo ""

# start background mining (runs indefinitely)
mine_anvil &
ANVIL_MINE_PID=$!

mine_bitcoin &
BITCOIN_MINE_PID=$!

sleep 1  # give mining a moment to start

# step 1: fund operator wallets
step "Step 1: Fund Operator Wallets"
echo ""
warn "Run offline: bash cli-operations.sh operator fund --env local"
warn "Note: Wait a few seconds for all Bitcoin addresses to appear in the output"
echo ""
read -p "Press Enter after completing the command above: "
success "Operator wallets funded"
echo ""
wait_for_bitcoin_blocks 1
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
wait_for_bitcoin_blocks 6
echo ""

# step 3: request pegin
step "Step 3: Request Pegin"
log "RSK Address: $RSK_ADDRESS"
log "Amount: $AMOUNT sats"
log "Packet: $PACKET_NUMBER"
echo ""
warn "Run offline: bash cli-operations.sh user pegin -a $RSK_ADDRESS -v $AMOUNT -p $PACKET_NUMBER --env local"
echo ""
read -p "Press Enter after completing the command above: "
success "Pegin transaction confirmed"
echo ""
wait_for_bitcoin_blocks 6
echo ""

# step 4: request pegout
step "Step 4: Request Pegout"
log "Command: bash cli-operations.sh user pegout -a $PEGOUT_AMOUNT --env local"
log "Amount: $PEGOUT_AMOUNT sats"
echo ""
if ! bash cli-operations.sh user pegout -a $PEGOUT_AMOUNT --env local > /tmp/pegout-$$ 2>&1; then
    warn "Command failed! Output:"
    cat /tmp/pegout-$$
    rm -f /tmp/pegout-$$
    exit 1
fi
rm -f /tmp/pegout-$$
success "Pegout requested"
echo ""

step "✓ Complete"
success "All automated steps done!"
log "Mining continues in background (Anvil every 1s, Bitcoin every 5s)"
log "Press Ctrl+C to stop mining and exit"
echo ""

# wait indefinitely - mining continues until user interrupts
while true; do
    sleep 60
done
