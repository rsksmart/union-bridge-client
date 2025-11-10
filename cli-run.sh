#!/usr/bin/env bash

# wrapper script for run-local to run local operators
# usage: ./run-local.sh --id 1 --fresh
#        ./run-local.sh --features anvil
#        ./run-local.sh --help

set -euo pipefail

# change to script directory to ensure relative paths work
cd "$(dirname "$0")"

# forward all arguments to run-local
RUST_BACKTRACE=0 exec cargo run --manifest-path cli/run-local/Cargo.toml -- "$@"
