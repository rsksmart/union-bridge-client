#!/usr/bin/env bash

# Bitcoin Wallet Launcher
# Supports both interactive mode (no command) and command mode (with command)
#
# Interactive mode: ./cli-bitcoin-wallet.sh [network] <user|member>
# Command mode:     ./cli-bitcoin-wallet.sh [network] <user|member> <command> [args...]
#
# Network defaults to regtest if not specified.

set -e

# Check if at least mode argument is provided
if [ $# -eq 0 ]; then
    echo "Usage: $0 [regtest|testnet] <user|member> [command] [args...]"
    echo ""
    echo "Networks (default: regtest):"
    echo "  regtest - Local regtest network (for development)"
    echo "  testnet - Bitcoin testnet network"
    echo ""
    echo "Modes:"
    echo "  user   - Launch wallet in user mode for peg-in/peg-out operations"
    echo "  member - Launch wallet in member mode for BitVMX operations"
    echo ""
    echo "Interactive mode (no command):"
    echo "  $0 <user|member>                    # uses regtest"
    echo "  $0 <network> <user|member>          # explicit network"
    echo "  Opens an interactive prompt where you can type commands"
    echo ""
    echo "Command mode (with command - REGTEST ONLY):"
    echo "  $0 <user|member> <command> [args...]        # uses regtest"
    echo "  $0 regtest <user|member> <command> [args...]"
    echo "  Executes a single command and exits"
    echo "  NOTE: Command mode is restricted to regtest for safety"
    echo ""
    echo "Command mode examples:"
    echo "  $0 user mine_block"
    echo "  $0 user mine_utxo 50000000"
    echo "  $0 user send_to_address bcrt1q... 10000"
    echo "  $0 user create_pegin_tx 50000000 1 bcrt1p... 0x1234..."
    echo "  $0 user list_funds"
    echo ""
    echo "Interactive mode examples:"
    echo "  $0 user"
    echo "  $0 testnet user"
    echo "  $0 testnet member"
    echo ""
    echo "Available commands:"
    echo "  mine_block                                  - Mine a single block (regtest only)"
    echo "  mine_utxo [sats]                           - Mine and fund active address"
    echo "  send_to_address <addr_csv> <sats> [count]  - Send to addresses"
    echo "  create_pegin_tx <value> <packet> <addr> <rsk> - Create RSK pegin transaction"
    echo "  list_funds [all]                           - List available UTXOs"
    echo "  tx_status <txid>                           - Check transaction status"
    echo "  block_height                               - Get current blockchain height"
    echo "  ... and all other wallet commands"
    echo ""
    echo "Required environment variables:"
    echo "  For user mode:   USER_BITCOIN_WIF"
    echo "  For member mode: MEMBER_BITCOIN_WIF"
    exit 1
fi

# Determine if first argument is a network or mode
# If first arg is user/member, default network to regtest
case "$1" in
    "regtest"|"testnet")
        NETWORK="$1"
        shift
        if [ $# -eq 0 ]; then
            echo "Error: Missing mode argument"
            echo "Usage: $0 [regtest|testnet] <user|member> [command] [args...]"
            exit 1
        fi
        MODE="$1"
        shift
        ;;
    "user"|"member")
        NETWORK="regtest"
        MODE="$1"
        shift
        ;;
    *)
        echo "Error: Invalid argument '$1'"
        echo "Expected network (regtest|testnet) or mode (user|member)"
        exit 1
        ;;
esac

# Validate mode and check corresponding environment variable
case "$MODE" in
    "user")
        if [ -z "${USER_BITCOIN_WIF:-}" ]; then
            echo "Error: USER_BITCOIN_WIF environment variable is not set"
            echo "Please set USER_BITCOIN_WIF to your user Bitcoin WIF private key"
            exit 1
        fi
        if [ $# -eq 0 ]; then
            echo "Starting bitcoin-wallet in USER mode on $NETWORK (interactive)..."
        else
            # check if another user wallet is already running before executing command
            if pgrep -f "ub-wallet.*--mode user.*--env $NETWORK" > /dev/null 2>&1; then
                echo "Error: Another USER wallet instance is already running on $NETWORK"
                echo ""
                echo "You cannot run commands while an interactive session is open."
                echo ""
                echo "Please close the interactive wallet session first (type 'exit' or press Ctrl+D)"
                echo "Or check for running processes: ps aux | grep 'ub-wallet.*--mode user.*--env $NETWORK'"
                exit 1
            fi
            echo "Executing command in USER mode on $NETWORK: $*"
        fi
        ;;
    "member")
        if [ -z "${MEMBER_BITCOIN_WIF:-}" ]; then
            echo "Error: MEMBER_BITCOIN_WIF environment variable is not set"
            echo "Please set MEMBER_BITCOIN_WIF to your member Bitcoin WIF private key"
            exit 1
        fi
        if [ $# -eq 0 ]; then
            echo "Starting bitcoin-wallet in MEMBER mode on $NETWORK (interactive)..."
        else
            # check if another member wallet is already running before executing command
            if pgrep -f "ub-wallet.*--mode member.*--env $NETWORK" > /dev/null 2>&1; then
                echo "Error: Another MEMBER wallet instance is already running on $NETWORK"
                echo ""
                echo "You cannot run commands while an interactive session is open."
                echo ""
                echo "Please close the interactive wallet session first (type 'exit' or press Ctrl+D)"
                echo "Or check for running processes: ps aux | grep 'ub-wallet.*--mode member.*--env $NETWORK'"
                exit 1
            fi
            echo "Executing command in MEMBER mode on $NETWORK: $*"
        fi
        ;;
    *)
        echo "Error: Invalid mode '$MODE'"
        echo "Valid modes are: user, member"
        exit 1
        ;;
esac

# Launch bitcoin-wallet with the specified network and mode
# If no additional arguments, opens interactive mode
# If arguments provided, executes command and exits

# In CI (e.g. GitHub Actions), workflow pre-builds the binary; skip build to avoid redundant compile
# if [ "${CI:-}" = "true" ] && [ -x "./target/release/ub-wallet" ]; then
  echo "[wallet-cli] CI: using pre-built binary (skip cargo build)" >&2
# else
#   echo "[wallet-cli] Starting cargo build --release..." >&2
#   cargo build --release --manifest-path ./cli/bitcoin-wallet/Cargo.toml --quiet
#   echo "[wallet-cli] Cargo build finished, exec ub-wallet..." >&2
# fi
exec ./target/release/ub-wallet --env "$NETWORK" --mode "$MODE" "$@"
