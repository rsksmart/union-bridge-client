#!/usr/bin/env bash

# wrapper script for operations to perform operator and user operations
# usage: ./cli-operations.sh setup create-rootstock-wallets
#        ./cli-operations.sh operator fund --env local-docker
#        ./cli-operations.sh operator apply-stream -s 1 --env alphanet -o 1 -r prover
#        ./cli-operations.sh user pegin -a 0x1234...cdef -v 100000 -p 7
#        ./cli-operations.sh user pegout -v 100000
#        ./cli-operations.sh --help

set -euo pipefail

# change to script directory to ensure relative paths work
cd "$(dirname "$0")"

# Build release binary (fast if already built, ~1-2s check)
cargo build --release --manifest-path cli/operations/Cargo.toml --quiet

# forward all arguments to operations (using release binary directly)
RUST_BACKTRACE=0 exec ./target/release/operations "$@"

