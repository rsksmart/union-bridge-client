#!/bin/bash
set -euo pipefail

# TODO(jira) https://rsklabs.atlassian.net/browse/ub-176

echo "[bitvmx-setup] Setting up SSH access to GitHub..."
mkdir -p -m 0700 ~/.ssh
ssh-keyscan github.com >> ~/.ssh/known_hosts

echo "[bitvmx-setup] Cloning bitvmx..."
git clone --depth=1 --branch main --recurse-submodules \
  ssh://git@github.com/FairgateLabs/rust-bitvmx-workspace.git /rust-bitvmx-workspace

echo "[bitvmx-setup] Done"