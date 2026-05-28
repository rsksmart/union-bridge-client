#!/usr/bin/env bash

# wrapper script for operations to perform operator and user operations
# usage: ./cli-operations.sh operator fund --env docker-anvil
#        ./cli-operations.sh operator apply-stream -s 1 --env alphanet -o 1 -r prover
#        ./cli-operations.sh user fund --env local-anvil
#        ./cli-operations.sh user pegin -a 0x1234...cdef -v 100000
#        ./cli-operations.sh user pegout -v 100000
#        ./cli-operations.sh --help

set -euo pipefail

# change to script directory to ensure relative paths work
cd "$(dirname "$0")"

OPERATIONS_BIN="./target/release/operations"

# In GitHub Actions (e.g. e2e framework): use existing binary if present (cache hit). Locally: always build so we never run stale code.
if ! { [ -x "$OPERATIONS_BIN" ] && [ "${GITHUB_ACTIONS:-}" = "true" ]; }; then
  cargo build --release --manifest-path cli/operations/Cargo.toml --quiet
fi

# forward all arguments to operations (using release binary directly)
RUST_BACKTRACE=0 exec "$OPERATIONS_BIN" "$@"
