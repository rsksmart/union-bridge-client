#!/bin/bash

set -e

echo "🦀 Formatting code..."
cargo +nightly fmt
cargo +nightly fmt --manifest-path check-fork/zkp/guest/Cargo.toml

echo "🦀 Sorting Cargo.toml..."
cargo sort -w > /dev/null
cargo sort check-fork/zkp/guest > /dev/null