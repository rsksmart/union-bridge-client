#!/bin/bash
set -euo pipefail

if [ -z "${UNION_CONTRACTS_TAG:-}" ]; then
  echo "[contracts-setup] No need to install contracts"
  exit 0
fi

echo "[contracts-setup] Setting up SSH access to GitHub..."
mkdir -p -m 0700 ~/.ssh
ssh-keyscan github.com >> ~/.ssh/known_hosts

echo "[contracts-setup] Cloning contracts @ ${UNION_CONTRACTS_TAG}..."
git clone --depth=1 --branch "$UNION_CONTRACTS_TAG" \
  ssh://git@github.com/temp-rsk/bitvmx-union-bridge-contracts.git /app/contracts

echo "[contracts-setup] Installing and building contracts..."
cd /app/contracts
/opt/foundry/bin/forge install
/opt/foundry/bin/forge build

echo "[contracts-setup] Done."