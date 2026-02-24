#!/usr/bin/env bash

# happy path e2e test - fully automated local flow (no manual intervention)
#
# prerequisites:
#   - union bridge clients running (via: cargo run -- run)
#   - anvil running on localhost:8545
#   - bitcoin regtest node running with RPC enabled
#   - USER_BITCOIN_WIF environment variable set (for bitcoin-wallet operations)
#   - MEMBER_BITCOIN_WIF environment variable set (for member operations)
#
# usage: bash tests/run-happy-path.sh

set -euo pipefail

# Try to load UC_ENV from .envrc if not already set and direnv isn't active
# This ensures UC_ENV is available even when scripts are run from subdirectories
if [[ -z "${UC_ENV:-}" ]]; then
    # change to project root first to find .envrc
    cd "$(dirname "$0")/.."
    ENVRC_FILE="$(pwd)/.envrc"
    if [[ -f "$ENVRC_FILE" ]]; then
        # Parse only UC_ENV from .envrc — does not execute arbitrary shell code
        _uc_env_line=$(grep -E '^\s*export\s+UC_ENV=' "$ENVRC_FILE" | tail -1 | sed 's/^\s*export\s*//')
        if [[ -n "$_uc_env_line" ]]; then
            UC_ENV="${_uc_env_line#UC_ENV=}"
            UC_ENV=$(echo "$UC_ENV" | sed 's/^["'\''"]//;s/["'\''"]$//')
            export UC_ENV
        fi
        unset _uc_env_line
    fi
fi

# Initialize SCRIPT_ENV from UC_ENV if set and valid, otherwise default to "local"
if [[ -n "${UC_ENV:-}" ]]; then
    if [[ "$UC_ENV" == "local" || "$UC_ENV" == "local-docker" ]]; then
        SCRIPT_ENV="$UC_ENV"
    else
        SCRIPT_ENV="local"
    fi
else
    SCRIPT_ENV="local"
fi

usage() {
    echo "Usage: $0 [--env <local|local-docker>] [--ops <1-10>]"
    echo ""
    echo "  --env    Environment: local or local-docker (default: from UC_ENV or local)"
    echo "  --ops    Number of operators (1-10, default: from .env.local or 4)"
    exit 1
}

OPS_FROM_FLAG=""
while [[ $# -gt 0 ]]; do
    case "$1" in
    --env)
        SCRIPT_ENV="${2:-}"
        if [[ -z "$SCRIPT_ENV" ]]; then
            usage
        fi
        if [[ "$SCRIPT_ENV" != "local" && "$SCRIPT_ENV" != "local-docker" ]]; then
            echo "Error: --env must be 'local' or 'local-docker'"
            usage
        fi
        shift 2
        ;;
    --ops)
        OPS_FROM_FLAG="${2:-}"
        if [[ -z "$OPS_FROM_FLAG" ]] || ! [[ "$OPS_FROM_FLAG" =~ ^(10|[1-9])$ ]]; then
            echo "Error: --ops must be between 1 and 10"
            exit 1
        fi
        shift 2
        ;;
    *)
        usage
        ;;
    esac
done

# Final validation that SCRIPT_ENV is valid
if [[ "$SCRIPT_ENV" != "local" && "$SCRIPT_ENV" != "local-docker" ]]; then
    echo "Error: SCRIPT_ENV must be 'local' or 'local-docker'"
    exit 1
fi

# change to project root (parent of tests directory)
cd "$(dirname "$0")/.."

# Load NUM_OPERATORS: --ops flag > .env.local > default (4)
if [[ -n "$OPS_FROM_FLAG" ]]; then
    NUM_OPERATORS="$OPS_FROM_FLAG"
else
    NUM_OPERATORS=4
    _env_local="docker/operator/.env.local"
    if [[ -f "$_env_local" ]]; then
        _num=$(grep -E '^\s*NUM_OPERATORS=' "$_env_local" | tail -1 | cut -d= -f2 | tr -d ' "'\''')
        if [[ -n "$_num" ]] && [[ "$_num" =~ ^[0-9]+$ ]] && [[ "$_num" -ge 1 ]] && [[ "$_num" -le 10 ]]; then
            NUM_OPERATORS="$_num"
        fi
    fi
    unset _env_local _num
fi

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

# derive x-only public key (32 bytes) from USER_BITCOIN_WIF
# returns the key with 0x prefix (66 chars total)
user_xonly_pubkey_from_wif() {
    if [[ -z "${USER_BITCOIN_WIF:-}" ]]; then
        echo "Error: USER_BITCOIN_WIF not set" >&2
        return 1
    fi
    local desc pubkey xonly
    desc=$(bitcoin-cli -regtest -rpcuser=foo -rpcpassword=rpcpassword getdescriptorinfo "wpkh(${USER_BITCOIN_WIF})" 2>/dev/null | jq -r '.descriptor')
    if [[ -z "$desc" || "$desc" == "null" ]]; then
        echo "Error: Failed to derive descriptor from USER_BITCOIN_WIF" >&2
        return 1
    fi
    # Extract pubkey from wpkh(PUBKEY)#checksum format
    pubkey=$(echo "$desc" | sed -E 's/^wpkh\(([0-9a-fA-F]+)\)#.*/\1/')
    if [[ ${#pubkey} -ne 66 ]]; then
        echo "Error: Unexpected pubkey length: ${#pubkey}" >&2
        return 1
    fi
    # Remove first 2 chars (02/03 prefix) to get x-only key
    xonly="${pubkey:2}"
    echo "0x${xonly}"
}

# derive compressed public key (33 bytes) from USER_BITCOIN_WIF
# returns the key with 0x prefix (68 chars total)
user_compressed_pubkey_from_wif() {
    if [[ -z "${USER_BITCOIN_WIF:-}" ]]; then
        echo "Error: USER_BITCOIN_WIF not set" >&2
        return 1
    fi
    local desc pubkey
    desc=$(bitcoin-cli -regtest -rpcuser=foo -rpcpassword=rpcpassword getdescriptorinfo "wpkh(${USER_BITCOIN_WIF})" 2>/dev/null | jq -r '.descriptor')
    if [[ -z "$desc" || "$desc" == "null" ]]; then
        echo "Error: Failed to derive descriptor from USER_BITCOIN_WIF" >&2
        return 1
    fi
    # Extract pubkey from wpkh(PUBKEY)#checksum format
    pubkey=$(echo "$desc" | sed -E 's/^wpkh\(([0-9a-fA-F]+)\)#.*/\1/')
    if [[ ${#pubkey} -ne 66 ]]; then
        echo "Error: Unexpected pubkey length: ${#pubkey}" >&2
        return 1
    fi
    echo "0x${pubkey}"
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

# Find recent log match in docker compose logs (all operators)
# Checks all logs (no time restriction) since we're already polling in a loop
# Output format: "source:line" or empty if not found
find_recent_docker_log_match() {
    local pattern="$1"

    for op_id in $(seq 1 "$NUM_OPERATORS"); do
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
                warn "Check Docker logs manually: docker compose -p op_{1..$NUM_OPERATORS} logs coordinator"
            else
                warn "Check logs/coordinator-*.log manually"
            fi
            return 1
        fi

        sleep 1
    done
}

cleanup() {
    rm -f /tmp/apply-operators-$$ /tmp/pegout-$$
}
trap cleanup EXIT

clear
log "Configuration: stream=$STREAM_ID, rsk=$RSK_ADDRESS, amount=$VALUE, env=$SCRIPT_ENV"
log "Prerequisite: Start mining in another terminal with: ./cli-infra.sh --start-mine"
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

# Derive x-only public key for pegin (32 bytes with 0x prefix)
USER_XONLY_PUBKEY=$(user_xonly_pubkey_from_wif)
if [[ -z "$USER_XONLY_PUBKEY" ]]; then
    warn "Failed to derive user x-only public key from WIF"
    exit 1
fi

log "RSK Address: $RSK_ADDRESS"
log "Amount: $VALUE sats"
log "Packet: $PACKET_NUMBER"
log "BTC Pub Key: $USER_XONLY_PUBKEY"
log "Command: bash cli-operations.sh user pegin -a $RSK_ADDRESS -v $VALUE -p $PACKET_NUMBER -k $USER_XONLY_PUBKEY --env $SCRIPT_ENV --execute"
echo ""
if ! bash cli-operations.sh user pegin -a $RSK_ADDRESS -v $VALUE -p $PACKET_NUMBER -k "$USER_XONLY_PUBKEY" --env "$SCRIPT_ENV" --execute; then
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

step "Step 4: Request Pegout"

# Derive compressed public key for pegout (33 bytes with 0x prefix)
USER_COMPRESSED_PUBKEY=$(user_compressed_pubkey_from_wif)
if [[ -z "$USER_COMPRESSED_PUBKEY" ]]; then
    warn "Failed to derive user compressed public key from WIF"
    exit 1
fi

log "Command: bash cli-operations.sh user pegout -v $VALUE -k $USER_COMPRESSED_PUBKEY --env $SCRIPT_ENV"
log "Amount: $VALUE sats"
log "USR Pub Key: $USER_COMPRESSED_PUBKEY"
echo ""

if ! bash cli-operations.sh user pegout -v $VALUE -k "$USER_COMPRESSED_PUBKEY" --env "$SCRIPT_ENV" > /tmp/pegout-$$ 2>&1; then
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

if [ "$SUCCESS" = true ]; then
    success "E2E test completed successfully!"
else
    warn "E2E test completed with warnings - PegoutFlow completion not verified"
    exit 1
fi
