#!/bin/bash
set -e

# 1) Delete everything inside data/
echo "Clearing data/..."
rm -rf data/*

# 2) Delete everything inside logs/
echo "Clearing logs/..."
rm -rf logs/*

# 3) Delete any log4rs_*.yaml (excluding log4rs.yaml)
echo "Removing log4rs_*.yaml (excluding log4rs.yaml)..."
rm -f log4rs_*.yaml 2>/dev/null || true

# 4) Delete any timestamped subfolder (pattern %Y%m%d_%H%M%S) inside config/ or its subfolders.
echo "Removing timestamped subfolders in config/..."

# Explanation:
#   - `find config/ -type d` finds all directories under `config/`.
#   - `grep -E '[0-9]{8}_[0-9]{6}'` filters for names containing 8 digits, underscore, then 6 digits.
#   - `sort -r` ensures child dirs are removed before parents (avoid "No such file or directory" errors).
#   - `xargs rm -rf` removes them.

find config/ -type d | grep -E '[0-9]{8}_[0-9]{6}' | sort -r | xargs rm -rf 2>/dev/null || true

echo "Clear script completed."