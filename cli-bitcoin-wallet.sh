#!/usr/bin/env bash

# Bitcoin Wallet Launcher
# Usage: ./cli-bitcoin-wallet.sh <user|member> [additional_args...]

set -e

# Check if mode argument is provided
if [ $# -eq 0 ]; then
    echo "Usage: $0 <user|member> [additional_args...]"
    echo ""
    echo "Modes:"
    echo "  user   - Launch wallet in user mode for peg-in/peg-out operations"
    echo "  member - Launch wallet in member mode for BitVMX operations"
    echo ""
    echo "Required environment variables:"
    echo "  For user mode:   USER_BITCOIN_WIF"
    echo "  For member mode: MEMBER_BITCOIN_WIF"
    exit 1
fi

MODE="$1"
shift # Remove mode from arguments

# Validate mode and check corresponding environment variable
case "$MODE" in
    "user")
        if [ -z "${USER_BITCOIN_WIF:-}" ]; then
            echo "Error: USER_BITCOIN_WIF environment variable is not set"
            echo "Please set USER_BITCOIN_WIF to your user Bitcoin WIF private key"
            exit 1
        fi
        echo "Starting bitcoin-wallet in USER mode..."
        ;;
    "member")
        if [ -z "${MEMBER_BITCOIN_WIF:-}" ]; then
            echo "Error: MEMBER_BITCOIN_WIF environment variable is not set"
            echo "Please set MEMBER_BITCOIN_WIF to your member Bitcoin WIF private key"
            exit 1
        fi
        echo "Starting bitcoin-wallet in MEMBER mode..."
        ;;
    *)
        echo "Error: Invalid mode '$MODE'"
        echo "Valid modes are: user, member"
        exit 1
        ;;
esac

# Launch bitcoin-wallet with the specified mode
exec cargo run --manifest-path ./bitcoin-wallet/Cargo.toml --bin ub-wallet -- --mode "$MODE" "$@"