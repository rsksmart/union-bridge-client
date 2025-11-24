#!/usr/bin/env bash

set -euo pipefail

cd cli
cargo run --bin mocks --features "anvil"