#!/usr/bin/env bash

# wrapper script for run to run local operators
# usage: ./cli-run.sh --id 1 --fresh
#        ./cli-run.sh --features anvil
#        ./cli-run.sh --help

set -euo pipefail

# change to script directory to ensure relative paths work
cd "$(dirname "$0")"

# forward all arguments to run
RUST_BACKTRACE=1 exec cargo run --manifest-path cli/run/Cargo.toml -- "$@"
