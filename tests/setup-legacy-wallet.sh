#!/usr/bin/env bash

# Script to create/load a legacy Bitcoin wallet and export address and WIF
#
# prerequisites:
#   - bitcoin regtest node running with RPC enabled
#
# usage: source tests/setup-legacy-wallet.sh
#   or:   eval "$(bash tests/setup-legacy-wallet.sh)"

set -euo pipefail

# colors
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m'

log() { echo -e "${BLUE}[INFO]${NC} $1" >&2; }
success() { echo -e "${GREEN}[✓]${NC} $1" >&2; }
warn() { echo -e "${YELLOW}[!]${NC} $1" >&2; }

# Check if bitcoin-cli is available
if ! command -v bitcoin-cli &> /dev/null; then
    echo "Error: bitcoin-cli required" >&2
    exit 1
fi

# Check if Bitcoin RPC is accessible
if ! bitcoin-cli -regtest -rpcuser=foo -rpcpassword=rpcpassword getblockcount &> /dev/null; then
    echo "Error: Bitcoin regtest node not accessible" >&2
    echo "Please ensure Bitcoin Core is running with:" >&2
    echo "  bitcoind -regtest -rpcuser=foo -rpcpassword=rpcpassword" >&2
    exit 1
fi

# Create legacy wallet to sign transactions with utxo available
# Note: If bitcoind was started with -wallet=mainwallet, it creates a descriptor wallet by default
# We need a legacy wallet for dumpprivkey to work, so we'll use a separate wallet name
LEGACY_WALLET_NAME="legacy_wallet"
log "Ensuring legacy wallet '$LEGACY_WALLET_NAME' is available"
# Check if legacy wallet exists
if bitcoin-cli -regtest -rpcuser=foo -rpcpassword=rpcpassword listwallets 2>/dev/null | grep -q "$LEGACY_WALLET_NAME"; then
  log "Legacy wallet '$LEGACY_WALLET_NAME' is already loaded"
else
  # Try to create the legacy wallet
  CREATE_OUTPUT=$(bitcoin-cli -regtest -rpcuser=foo -rpcpassword=rpcpassword -named createwallet wallet_name="$LEGACY_WALLET_NAME" descriptors=false 2>&1) || {
    # If creation fails, try loading it (might already exist but not loaded)
    if ! bitcoin-cli -regtest -rpcuser=foo -rpcpassword=rpcpassword loadwallet "$LEGACY_WALLET_NAME" >/dev/null 2>&1; then
      warn "Failed to create or load legacy wallet '$LEGACY_WALLET_NAME'!"
      warn "Error: $CREATE_OUTPUT"
      exit 1
    fi
  }
fi

log "Getting legacy wallet address"
LEGACY_ADDRESS=$(bitcoin-cli -regtest -rpcuser=foo -rpcpassword=rpcpassword -rpcwallet="$LEGACY_WALLET_NAME" getnewaddress)
log "Legacy address: $LEGACY_ADDRESS"
if [ -z "$LEGACY_ADDRESS" ]; then
  warn "Failed to get legacy wallet address!"
  exit 1
fi

log "Getting legacy wallet wif"
LEGACY_ADDRESS_WIF=$(bitcoin-cli -regtest -rpcuser=foo -rpcpassword=rpcpassword -rpcwallet="$LEGACY_WALLET_NAME" dumpprivkey $LEGACY_ADDRESS)
log "Legacy address wif: $LEGACY_ADDRESS_WIF"
if [ -z "$LEGACY_ADDRESS_WIF" ]; then
  warn "Failed to get legacy wallet WIF!"
  exit 1
fi

# Fund the legacy address with at least 15 BTC
# In regtest, we mine blocks directly to the address (simpler than sendtoaddress)
log "Funding legacy address with at least 15 BTC..."
# Each block reward in regtest is 50 BTC, so we need at least 1 block (but mine 101 to mature coinbase)
# Mine 101 blocks to the legacy address to ensure we have mature, spendable coins
bitcoin-cli -regtest -rpcuser=foo -rpcpassword=rpcpassword generatetoaddress 101 "$LEGACY_ADDRESS" >/dev/null 2>&1 || {
  warn "Failed to mine blocks to legacy address"
  exit 1
}
log "Mined 101 blocks to legacy address (50 BTC per block, first 100 blocks mature after mining)"

# Verify the balance
LEGACY_BALANCE=$(bitcoin-cli -regtest -rpcuser=foo -rpcpassword=rpcpassword -rpcwallet="$LEGACY_WALLET_NAME" getbalance 2>/dev/null || echo "0")
log "Legacy wallet balance: $LEGACY_BALANCE BTC"

success "Legacy wallet setup complete"

# Output variable assignments for sourcing
# If script is sourced directly, export the variables
# If script is executed, output variable assignments
if [[ "${BASH_SOURCE[0]}" == "${0}" ]]; then
    # Script is being executed (not sourced)
    echo "export LEGACY_WALLET_NAME=\"$LEGACY_WALLET_NAME\""
    echo "export LEGACY_ADDRESS=\"$LEGACY_ADDRESS\""
    echo "export LEGACY_ADDRESS_WIF=\"$LEGACY_ADDRESS_WIF\""
else
    # Script is being sourced
    export LEGACY_WALLET_NAME="$LEGACY_WALLET_NAME"
    export LEGACY_ADDRESS="$LEGACY_ADDRESS"
    export LEGACY_ADDRESS_WIF="$LEGACY_ADDRESS_WIF"
fi
