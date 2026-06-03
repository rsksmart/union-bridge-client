#!/bin/bash
# Run clippy across every Cargo workspace in the repo.
# Always operates in check mode (clippy doesn't auto-fix).

set -e

WORKSPACES=("." "cli" "crates/check-fork/zkp/guest")

echo "🦀 Checking clippy lints..."
for ws in "${WORKSPACES[@]}"; do
  RISC0_SKIP_BUILD=1 cargo clippy \
    --manifest-path "$ws/Cargo.toml" \
    --workspace --all-targets --all-features --locked \
    -- -D warnings
done
