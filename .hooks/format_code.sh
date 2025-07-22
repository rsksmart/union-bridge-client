#!/bin/bash

set -e

echo "🦀 Formatting code..."
cargo +nightly fmt

echo "🦀 Sorting Cargo.toml..."
cargo sort -w > /dev/null

echo "Formatting complete!"