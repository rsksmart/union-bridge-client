#!/usr/bin/env bash

# wrapper script for run-cli to avoid typing the full cargo command
# usage: ./run-cli.sh run --num-clients 4
#        ./run-cli.sh setup-wallets create --num-wallets 4

set -euo pipefail

# change to script directory to ensure relative paths work
cd "$(dirname "$0")"

# forward all arguments to run-cli
exec cargo run --manifest-path run-cli/Cargo.toml -- "$@"

