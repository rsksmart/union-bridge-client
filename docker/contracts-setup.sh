#!/bin/bash
set -euo pipefail

echo "[contracts-setup] Checking if UNION_CONTRACTS_TAG is set..."

if [ -z "${UNION_CONTRACTS_TAG:-}" ]; then
  echo "[contracts-setup] Skipping: UNION_CONTRACTS_TAG not set"
  exit 0
fi

echo "[contracts-setup] Installing Node.js..."
apt-get update && apt-get install -y curl git gnupg ca-certificates
curl -fsSL https://deb.nodesource.com/setup_18.x | bash -
apt-get install -y nodejs
node --version && npm --version

echo "[contracts-setup] Installing Foundry..."
curl -L https://foundry.paradigm.xyz | bash
~/.foundry/bin/foundryup --install "$FOUNDRY_VERSION"
mkdir -p /opt/foundry && cp -r ~/.foundry/* /opt/foundry
rm -rf ~/.foundry

echo "[contracts-setup] Setting up SSH access to GitHub..."
mkdir -p -m 0700 ~/.ssh
ssh-keyscan github.com >> ~/.ssh/known_hosts

echo "[contracts-setup] Cloning contracts @ ${UNION_CONTRACTS_TAG}..."
git clone --depth=1 --branch "$UNION_CONTRACTS_TAG" \
  ssh://git@github.com/FairgateLabs/bitvmx-union-bridge-contracts.git /app/contracts

echo "[contracts-setup] Installing and building contracts..."
cd /app/contracts
/opt/foundry/bin/forge install
/opt/foundry/bin/forge build

echo "[contracts-setup] Done."