#!/bin/bash
set -euo pipefail

if [ -z "${UNION_CONTRACTS_TAG:-}" ]; then
  echo "[actors-mocking-setup] No need to install contracts"
  exit 0
fi

if [ -z "${FOUNDRY_VERSION:-}" ]; then
  echo "[actors-mocking-setup] Foundry version not provided, exiting."
  exit 1
fi

# only required for contracts, therefore not in Dockerfile
apt-get update -y && DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends \
  curl \
  git \
  gnupg ca-certificates \
  openssh-client \
  && apt-get clean \
  && rm -rf /var/lib/apt/lists/*

echo "[contracts-setup] Installing Node.js..."
curl -fsSL https://deb.nodesource.com/setup_18.x | bash -
apt-get install -y nodejs
node --version && npm --version

echo "[actors-mocking-setup] Installing Foundry..."
curl -L https://foundry.paradigm.xyz | bash
~/.foundry/bin/foundryup --install "$FOUNDRY_VERSION"
mkdir -p /opt/foundry && cp -r ~/.foundry/* /opt/foundry
rm -rf ~/.foundry

echo "[actors-mocking-setup] Setting up SSH access to GitHub..."
mkdir -p -m 0700 ~/.ssh
ssh-keyscan github.com >> ~/.ssh/known_hosts

echo "[actors-mocking-setup] Cloning contracts @ ${UNION_CONTRACTS_TAG}..."
git clone --depth=1 --branch "$UNION_CONTRACTS_TAG" \
  ssh://git@github.com/temp-rsk/bitvmx-union-bridge-contracts.git /app/contracts

echo "[actors-mocking-setup] Installing and building contracts..."
cd /app/contracts
/opt/foundry/bin/forge install
/opt/foundry/bin/forge build

echo "[actors-mocking-setup] Done."