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
  local commit="$2"

  if [[ -f "$filepath" ]]; then
    SKIPPED=$((SKIPPED + 1))
    return
  fi

  if ! git show "${commit}^:${filepath}" > /dev/null 2>&1; then
    # Try the commit itself (file may have been added and removed in the same commit)
    if ! git show "${commit}:${filepath}" > /dev/null 2>&1; then
      echo "  WARN: could not extract $filepath"
      FAILED=$((FAILED + 1))
      return
    fi
    mkdir -p "$(dirname "$filepath")"
    git show "${commit}:${filepath}" > "$filepath"
  else
    mkdir -p "$(dirname "$filepath")"
    git show "${commit}^:${filepath}" > "$filepath"
  fi

  echo "  restored: $filepath"
  RESTORED=$((RESTORED + 1))
}

echo "Restoring untracked config files from git history..."
echo ""

for env in "${ENVS_TO_RESTORE[@]}"; do
  echo "[$env]"

  # Discover all files that were ever deleted under this environment's bitvmx config
  while IFS= read -r line; do
    [[ -z "$line" ]] && continue
    commit=$(echo "$line" | cut -d' ' -f1)
    filepath=$(echo "$line" | cut -d' ' -f2-)
    restore_file "$filepath" "$commit"
  done < <(git log --all --diff-filter=D --format="%H" --name-only -- \
    "docker/bitvmx-client/config/${env}/" | \
    awk '/^[0-9a-f]{40}$/{commit=$0; next} NF{print commit, $0}')

  # Operator .env file for this environment
  env_commit=$(git log --all -1 --diff-filter=D --format=%H -- "docker/operator/.env.${env}" 2>/dev/null || true)
  if [[ -n "$env_commit" ]]; then
    restore_file "docker/operator/.env.${env}" "$env_commit"
  fi

  echo ""
done

# Docker-level .env files
echo "[docker]"
for env_file in docker/.env.testnet docker/.env.alphanet docker/.env.regtest; do
  env_commit=$(git log --all -1 --diff-filter=D --format=%H -- "$env_file" 2>/dev/null || true)
  if [[ -n "$env_commit" ]]; then
    restore_file "$env_file" "$env_commit"
  fi
done
echo ""

echo "Done: ${RESTORED} restored, ${SKIPPED} already existed (skipped), ${FAILED} failed."
