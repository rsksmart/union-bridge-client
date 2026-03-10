#!/usr/bin/env bash

# Bitcoin Wallet Launcher
# Supports both interactive mode (no command) and command mode (with command)
#
# Interactive mode: ./cli-bitcoin-wallet.sh [network] <user|member>
# Command mode:     ./cli-bitcoin-wallet.sh [network] <user|member> <command> [args...]
#
# Network defaults to regtest if not specified.

set -e

# If no arguments, default to default mode on regtest
if [ $# -eq 0 ]; then
    NETWORK="regtest"
    MODE="default"
fi

# Determine if first argument is a network or mode (skip parsing if already set by no-args default)
if [ -z "${MODE:-}" ]; then
case "$1" in
    "regtest"|"testnet")
        NETWORK="$1"
        shift
        if [ $# -eq 0 ]; then
            MODE="default"
        else
            MODE="$1"
            shift
        fi
        ;;
    "default"|"user"|"member")
        NETWORK="regtest"
        MODE="$1"
        shift
        ;;
    *)
        echo "Error: Invalid argument '$1'"
        echo "Expected network (regtest|testnet) or mode (default|user|member)"
        exit 1
        ;;
esac
fi

# Validate mode and check corresponding environment variable
case "$MODE" in
    "default")
        if [ $# -eq 0 ]; then
            echo "Starting bitcoin-wallet in DEFAULT mode on $NETWORK (interactive)..."
        else
            echo "Executing command in DEFAULT mode on $NETWORK: $*"
        fi
        ;;
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
        echo "Valid modes are: default, user, member"
        exit 1
        ;;
esac

# Launch bitcoin-wallet with the specified network and mode
# If no additional arguments, opens interactive mode
# If arguments provided, executes command and exits

WALLET_BIN="./target/release/ub-wallet"

# In GitHub Actions (e.g. e2e framework): use existing binary if present (cache hit). Locally: always build so we never run stale code.
if ! { [ -x "$WALLET_BIN" ] && [ "${GITHUB_ACTIONS:-}" = "true" ]; }; then
  cargo build --release --manifest-path ./cli/bitcoin-wallet/Cargo.toml --quiet
fi

exec "$WALLET_BIN" --env "$NETWORK" --mode "$MODE" "$@"
