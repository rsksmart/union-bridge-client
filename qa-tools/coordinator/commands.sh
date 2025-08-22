#!/bin/bash

# Default RPC port
DEFAULT_PORT=8545
RPC_PORT="${1:-$DEFAULT_PORT}"
RPC_URL="http://127.0.0.1:$RPC_PORT"

# Get current block number
function current_block() {
  local block_hex=$(curl -s -X POST -H "Content-Type: application/json" \
    -d '{"jsonrpc":"2.0","method":"eth_blockNumber","params":[],"id":1}' \
    $RPC_URL | jq -r '.result')
  local block_dec=$((block_hex))
  echo "Current block number: $block_dec"
}

# Load evm snapshot
function load() {
  echo "[evm_revert]"
  curl -s -X POST -H "Content-Type: application/json" \
    -d '{"jsonrpc":"2.0","method":"evm_revert","params":["0x0"],"id":1}' \
    $RPC_URL | jq
  current_block
}

# Save evm snapshot
function save() {
  echo "[evm_snapshot]"
  curl -s -X POST -H "Content-Type: application/json" \
    -d '{"jsonrpc":"2.0","method":"evm_snapshot","params":[],"id":1}' \
    $RPC_URL | jq
  current_block
}

# Mine a block
function mine() {
  echo "[evm_mine]"
  curl -s -X POST -H "Content-Type: application/json" \
    -d '{"jsonrpc":"2.0","method":"evm_mine","params":[],"id":1}' \
    $RPC_URL | jq
  current_block
}

# Fund an account
function fund() {
  echo "[cast send]"
  cast send \
    --rpc-url $RPC_URL \
    --from 0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266 \
    0x5bdd03ceaf59cad075cb29c67696581d857b9031 \
    --value 1ether \
    --unlocked
  current_block
}

# Export functions into current shell
export -f save load mine fund current_block
echo "Commands loaded: save, load, mine, fund"