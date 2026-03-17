#!/usr/bin/env bash
#
# Restores environment-specific config files that were removed from git tracking
# as part of the repository cleanup (open-source readiness).
#
# These files (keys, certs, operator YAMLs, .env files) are now gitignored but
# still required on each machine. This script recovers them from git history.
#
# When run on a branch that has the cleanup commit, the restored content is the
# version from the commit *before* the file was untracked — so you get the
# current branch's structure (e.g. 10 operators, updated op_*.yaml format).
# We prefer versions from the current branch (HEAD) so that "new" content added
# on this branch is what gets restored.
#
# Safe to run multiple times — existing files are never overwritten.
#
# Usage:
#   bash scripts/restore-untracked-configs.sh                  # restore all
#   bash scripts/restore-untracked-configs.sh testnet         # restore only testnet
#   bash scripts/restore-untracked-configs.sh regtest local   # restore regtest + local
#
set -euo pipefail

REPO_ROOT="$(git -C "$(dirname "$0")" rev-parse --show-toplevel)"
cd "$REPO_ROOT"

# Default: restore testnet, alphanet, regtest, and local (subset under config/local)
ENVS_TO_RESTORE=("${@}")
if [[ $# -eq 0 ]]; then
  ENVS_TO_RESTORE=(testnet alphanet regtest local)
fi

RESTORED=0
SKIPPED=0
FAILED=0

# Placeholder pattern: angle-bracket tokens like <PLACEHOLDER> or <your-value>
# We skip versions that contain these so we don't restore sanitized templates.
PLACEHOLDER_PATTERN='<[A-Za-z0-9_]{2,}>'

restore_file() {
  local filepath="$1"

  if [[ -f "$filepath" ]]; then
    SKIPPED=$((SKIPPED + 1))
    return
  fi

  # Prefer current branch (HEAD) so that "new" content on this branch is restored.
  # Then fall back to full history if the file was never on this branch.
  local rev_list=""
  if git log -1 --format= -- "$filepath" &>/dev/null; then
    rev_list=$(git log HEAD --format=%H -- "$filepath")
  fi
  if [[ -z "$rev_list" ]]; then
    rev_list=$(git log --all --format=%H -- "$filepath")
  fi

  local found=""
  while IFS= read -r rev; do
    [[ -z "$rev" ]] && continue
    local content
    content=$(git show "${rev}:${filepath}" 2>/dev/null) || continue

    if ! echo "$content" | grep -qE "$PLACEHOLDER_PATTERN"; then
      mkdir -p "$(dirname "$filepath")"
      echo "$content" > "$filepath"
      found=1
      break
    fi
  done <<< "$rev_list"

  if [[ -n "$found" ]]; then
    echo "  restored: $filepath"
    RESTORED=$((RESTORED + 1))
  else
    echo "  WARN: no clean version found for $filepath"
    FAILED=$((FAILED + 1))
  fi
}

echo "Restoring untracked config files from git history..."
echo "Preferring current branch (HEAD) so restored content matches this branch."
echo ""

for env in "${ENVS_TO_RESTORE[@]}"; do
  echo "[$env]"

  if [[ "$env" == "local" ]]; then
    # Local: only restore files that were deleted under config/local
    # (keys, broker config, wallet_*.yaml — op_*.yaml stay tracked)
    while IFS= read -r filepath; do
      [[ -z "$filepath" ]] && continue
      restore_file "$filepath"
    done < <(git log --all --diff-filter=D --name-only --format="" -- \
      "docker/bitvmx-client/config/local/" | sort -u)
    restore_file "docker/operator/.env.local"
  else
    # testnet / alphanet / regtest: restore entire env tree + operator .env
    while IFS= read -r filepath; do
      [[ -z "$filepath" ]] && continue
      restore_file "$filepath"
    done < <(git log --all --diff-filter=D --name-only --format="" -- \
      "docker/bitvmx-client/config/${env}/" | sort -u)
    restore_file "docker/operator/.env.${env}"
  fi

  echo ""
done

# Docker-level .env files (if any were ever deleted)
echo "[docker]"
while IFS= read -r env_file; do
  [[ -z "$env_file" ]] && continue
  restore_file "$env_file"
done < <(git log --all --diff-filter=D --name-only --format="" -- 'docker/.env.*' 2>/dev/null | sort -u)
echo ""

echo "Done: ${RESTORED} restored, ${SKIPPED} already existed (skipped), ${FAILED} failed."
