#!/usr/bin/env bash
#
# Restores environment-specific config files that were removed from git tracking
# as part of the repository cleanup (UBC-851).
#
# These files (keys, certs, operator yamls, .env files) are now gitignored but
# still required on each machine. This script recovers them from git history.
#
# Safe to run multiple times -- existing files are never overwritten.
#
# Usage:
#   bash scripts/restore-untracked-configs.sh            # restore all environments
#   bash scripts/restore-untracked-configs.sh testnet     # restore only testnet
#   bash scripts/restore-untracked-configs.sh regtest alphanet  # restore specific envs
#
set -euo pipefail

REPO_ROOT="$(git -C "$(dirname "$0")" rev-parse --show-toplevel)"
cd "$REPO_ROOT"

ENVS_TO_RESTORE=("${@:-testnet alphanet regtest}")
if [[ $# -eq 0 ]]; then
  ENVS_TO_RESTORE=(testnet alphanet regtest)
fi

RESTORED=0
SKIPPED=0
FAILED=0

restore_file() {
  local filepath="$1"

  if [[ -f "$filepath" ]]; then
    SKIPPED=$((SKIPPED + 1))
    return
  fi

  # Walk through all commits that touched this file (newest first) and pick the
  # last version that contains real values (no angle-bracket placeholders).
  local found=""
  while IFS= read -r rev; do
    local content
    content=$(git show "${rev}:${filepath}" 2>/dev/null) || continue

    if ! echo "$content" | grep -qE '<[A-Z0-9_]{3,}>'; then
      mkdir -p "$(dirname "$filepath")"
      echo "$content" > "$filepath"
      found=1
      break
    fi
  done < <(git log --all --format=%H -- "$filepath")

  if [[ -n "$found" ]]; then
    echo "  restored: $filepath"
    RESTORED=$((RESTORED + 1))
  else
    echo "  WARN: no clean version found for $filepath"
    FAILED=$((FAILED + 1))
  fi
}

echo "Restoring untracked config files from git history..."
echo ""

for env in "${ENVS_TO_RESTORE[@]}"; do
  echo "[$env]"

  # Discover all files that were ever deleted under this environment's bitvmx config
  while IFS= read -r filepath; do
    [[ -z "$filepath" ]] && continue
    restore_file "$filepath"
  done < <(git log --all --diff-filter=D --name-only --format="" -- \
    "docker/bitvmx-client/config/${env}/" | sort -u)

  # Operator .env file for this environment
  restore_file "docker/operator/.env.${env}"

  echo ""
done

# Docker-level .env files
echo "[docker]"
while IFS= read -r env_file; do
  [[ -z "$env_file" ]] && continue
  restore_file "$env_file"
done < <(git log --all --diff-filter=D --name-only --format="" -- 'docker/.env.*' | sort -u)
echo ""

echo "Done: ${RESTORED} restored, ${SKIPPED} already existed (skipped), ${FAILED} failed."
