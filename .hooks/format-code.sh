#!/bin/bash
# Format Rust code and sort Cargo.toml files across every workspace in the repo.
# Usage:
#   .hooks/format-code.sh           # write mode (rewrites files in place)
#   .hooks/format-code.sh --check   # check mode (non-zero exit on drift)

set -e

EXTRA_FMT_ARGS=()
EXTRA_SORT_ARGS=()
if [ "${1:-}" = "--check" ]; then
  EXTRA_FMT_ARGS=("--" "--check")
  EXTRA_SORT_ARGS=("--check")
fi

WORKSPACES=("." "cli" "crates/check-fork/zkp/guest")

echo "🦀 Formatting code..."
for ws in "${WORKSPACES[@]}"; do
  cargo +nightly fmt --all --manifest-path "$ws/Cargo.toml" "${EXTRA_FMT_ARGS[@]}"
done

echo "🦀 Sorting Cargo.toml..."
for ws in "${WORKSPACES[@]}"; do
  cargo sort --workspace "${EXTRA_SORT_ARGS[@]}" "$ws" > /dev/null
done
