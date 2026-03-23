#!/usr/bin/env bash
#
# Restores environment-specific config files that were removed from git tracking
# as part of the repository cleanup (open-source readiness).
#
# These files (keys, certs, operator YAMLs, .env files) are now gitignored but
# still required on each machine. This script recovers them from git history.
#
# Baseline: merge of feat/update_to_5.0.1_bitvmx_0.4.0_contracts_merge into main (PR #356).
#
# Broker TLS PEMs (broker/config/*.pem) and client keys (client/config/keys/*) are restored
# ONLY from this baseline — no older-commit fallback — so they stay consistent with the same
# tree as operator YAML pubkey_hash / allow_list / peers committed there.
#
# Other paths (e.g. some YAML, wallet templates) may still fall back through git history if
# missing at the baseline.
#
# Override the baseline commit with RESTORE_BASE_REF='<commit-ish>' (e.g. after a rename or rebase).
#
# Safe to run multiple times — existing files are never overwritten.
#
# Usage:
#   bash scripts/restore-untracked-configs.sh                  # restore all
#   bash scripts/restore-untracked-configs.sh testnet         # restore only testnet
#   bash scripts/restore-untracked-configs.sh regtest local   # restore regtest + local
#   RESTORE_BASE_REF=abc1234 bash scripts/restore-untracked-configs.sh
#
set -euo pipefail

REPO_ROOT="$(git -C "$(dirname "$0")" rev-parse --show-toplevel)"
cd "$REPO_ROOT"

# Merge commit on main: "Update to 5.0.1 BitVMX and Bridge contracts 0.4.0 (#356)"
# = feat/update_to_5.0.1_bitvmx_0.4.0_contracts_merge merged to main.
RESTORE_BASE_REF="${RESTORE_BASE_REF:-7cd5317c}"
if ! git rev-parse -q --verify "${RESTORE_BASE_REF}^{commit}" >/dev/null; then
  echo "Error: RESTORE_BASE_REF='${RESTORE_BASE_REF}' is not a valid commit in this repo." >&2
  echo "  Set it to the main merge of feat/update_to_5.0.1_bitvmx_0.4.0_contracts_merge (PR #356)." >&2
  exit 1
fi
RESTORE_BASE_SHA="$(git rev-parse --short "${RESTORE_BASE_REF}^{commit}")"

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

# PEMs + operator keys must match the baseline snapshot (same commit as pubkey_hash in YAML).
is_identity_material() {
  local filepath="$1"
  case "$filepath" in
    docker/bitvmx-client/config/*/broker/config/*.pem) return 0 ;;
    docker/bitvmx-client/config/*/client/config/keys/*.key) return 0 ;;
    docker/bitvmx-client/config/*/client/config/keys/*.pem) return 0 ;;
    *) return 1 ;;
  esac
}

# Restore only from RESTORE_BASE_REF (no history walk).
restore_file_baseline_only() {
  local filepath="$1"

  if [[ -f "$filepath" ]]; then
    SKIPPED=$((SKIPPED + 1))
    return
  fi

  if ! git cat-file -e "${RESTORE_BASE_REF}:${filepath}" 2>/dev/null; then
    echo "  ERROR: $filepath not in baseline ${RESTORE_BASE_SHA} — cannot fall back to history (must match committed pubkey_hash / broker identity)." >&2
    FAILED=$((FAILED + 1))
    return
  fi

  local tmp
  tmp="$(mktemp)"
  if ! git show "${RESTORE_BASE_REF}:${filepath}" >"$tmp" 2>/dev/null; then
    rm -f "$tmp"
    echo "  ERROR: failed to read $filepath from baseline ${RESTORE_BASE_SHA}" >&2
    FAILED=$((FAILED + 1))
    return
  fi

  if grep -qE "$PLACEHOLDER_PATTERN" "$tmp" 2>/dev/null; then
    rm -f "$tmp"
    echo "  ERROR: $filepath at baseline contains placeholders; refusing." >&2
    FAILED=$((FAILED + 1))
    return
  fi

  mkdir -p "$(dirname "$filepath")"
  mv "$tmp" "$filepath"
  echo "  restored: $filepath (baseline-only identity ${RESTORE_BASE_SHA})"
  RESTORED=$((RESTORED + 1))
}

restore_file() {
  local filepath="$1"

  if [[ -f "$filepath" ]]; then
    SKIPPED=$((SKIPPED + 1))
    return
  fi

  local found=""
  local content=""
  local source_note=""

  # 1) Prefer the BitVMX 5.0.1 / contracts 0.4.0 main merge tree (canonical updated configs).
  if git cat-file -e "${RESTORE_BASE_REF}:${filepath}" 2>/dev/null; then
    content=$(git show "${RESTORE_BASE_REF}:${filepath}" 2>/dev/null) || content=""
    if [[ -n "$content" ]] && ! echo "$content" | grep -qE "$PLACEHOLDER_PATTERN"; then
      mkdir -p "$(dirname "$filepath")"
      echo "$content" > "$filepath"
      echo "  restored: $filepath (from baseline ${RESTORE_BASE_SHA})"
      RESTORED=$((RESTORED + 1))
      return
    fi
  fi

  # 2) Fall back: walk history (HEAD first, then all refs).
  local rev_list=""
  if git log -1 --format= -- "$filepath" &>/dev/null; then
    rev_list=$(git log HEAD --format=%H -- "$filepath")
  fi
  if [[ -z "$rev_list" ]]; then
    rev_list=$(git log --all --format=%H -- "$filepath")
  fi

  while IFS= read -r rev; do
    [[ -z "$rev" ]] && continue
    content=$(git show "${rev}:${filepath}" 2>/dev/null) || continue

    if ! echo "$content" | grep -qE "$PLACEHOLDER_PATTERN"; then
      mkdir -p "$(dirname "$filepath")"
      echo "$content" > "$filepath"
      found=1
      source_note=$(git rev-parse --short "${rev}")
      break
    fi
  done <<< "$rev_list"

  if [[ -n "$found" ]]; then
    echo "  restored: $filepath (from history ${source_note})"
    RESTORED=$((RESTORED + 1))
  else
    echo "  WARN: no clean version found for $filepath"
    FAILED=$((FAILED + 1))
  fi
}

echo "Restoring untracked config files from git history..."
echo "Baseline tree: ${RESTORE_BASE_REF} (${RESTORE_BASE_SHA}) — main merge of BitVMX 5.0.1 + contracts 0.4.0 (#356)."
echo "Unset RESTORE_BASE_REF or override to use a different commit."
echo ""

for env in "${ENVS_TO_RESTORE[@]}"; do
  echo "[$env]"

  if [[ "$env" == "local" ]]; then
    # Local: only restore files that were deleted under config/local
    # (keys, broker config, wallet_*.yaml — op_*.yaml stay tracked)
    while IFS= read -r filepath; do
      [[ -z "$filepath" ]] && continue
      if is_identity_material "$filepath"; then
        restore_file_baseline_only "$filepath"
      else
        restore_file "$filepath"
      fi
    done < <(git log --all --diff-filter=D --name-only --format="" -- \
      "docker/bitvmx-client/config/local/" | sort -u)
    restore_file "docker/operator/.env.local"
    # Blockchains compose env (bitcoind + anvil + deploy-contracts); not under bitvmx-client/
    restore_file "docker/local-infra/.env.local"
  else
    # testnet / alphanet / regtest: restore entire env tree + operator .env
    while IFS= read -r filepath; do
      [[ -z "$filepath" ]] && continue
      if is_identity_material "$filepath"; then
        restore_file_baseline_only "$filepath"
      else
        restore_file "$filepath"
      fi
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
