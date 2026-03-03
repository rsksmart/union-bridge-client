#!/usr/bin/env bash

set -euo pipefail

cd cli

use_anvil_feature=true
for arg in "$@"; do
  if [[ "$arg" == "--no-deploy" ]]; then
    use_anvil_feature=false
    break
  fi
done

if [[ "${use_anvil_feature}" == "true" ]]; then
  cargo run --bin mocks --features "anvil" -- "$@"
else
  cargo run --bin mocks -- "$@"
fi
