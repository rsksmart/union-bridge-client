#!/usr/bin/env bash

# wrapper script for operations to perform operator and user operations
# usage: ./operations.sh setup create-rootstock-wallets
#        ./operations.sh operator fund --env local-docker
#        ./operations.sh operator apply-stream -s 1 --env alphanet -o 1 -r prover
#        ./operations.sh user pegin -a 0x1234...cdef -s 2000000 -p 7
#        ./operations.sh user pegout -a 1000000
#        ./operations.sh --help

set -euo pipefail

# change to script directory to ensure relative paths work
cd "$(dirname "$0")"

# forward all arguments to operations
RUST_BACKTRACE=0 exec cargo run --manifest-path cli/operations/Cargo.toml -- "$@"

