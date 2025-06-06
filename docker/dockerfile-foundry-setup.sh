#!/bin/bash
set -euo pipefail

if [ -z "${FOUNDRY_VERSION:-}" ]; then
  echo "[contracts-setup] No need to install Foundry"
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