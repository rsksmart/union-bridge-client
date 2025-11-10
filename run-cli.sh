#!/usr/bin/env bash

# wrapper script for run-cli to avoid typing the full cargo command
# usage: ./run-cli.sh run -n 4 [--fresh]
#        ./run-cli.sh setup-wallets both --num-wallets 4
#        ./run-cli.sh setup-wallets fund --env docker-local
#        ./run-cli.sh create-pegin-tx -a 0xabc...123 -s 1000000 -p 0
#        ./run-cli.sh setup-committee -s 1 [--env local]
#        ./run-cli.sh --help

set -euo pipefail

# change to script directory to ensure relative paths work
cd "$(dirname "$0")"

# forward all arguments to run-cli
RUST_BACKTRACE=0 exec cargo run --manifest-path run-cli/Cargo.toml -- "$@"

