#!/usr/bin/env bash
# tools/migrate-v031/test-e2e.sh
#
# Reproducible local E2E for the v0.3.1 → v0.4.x DB migration.
#
# Mirrors the sequence the testnet rollout will use, adapted for a single
# operator running locally in cargo mode. Each step prints what it is
# doing so the same flow can be performed by hand on a testnet host.
#
# Usage:
#   ./tools/migrate-v031/test-e2e.sh --check        verify prereqs and exit
#   ./tools/migrate-v031/test-e2e.sh --run          full E2E (interactive)
#   ./tools/migrate-v031/test-e2e.sh --cleanup      remove the v0.3.1 worktree and sandboxes
#
# Required environment variables for --run:
#   BASE_STORAGE_PATH    absolute path under which .union_bridge/ is created
#   BITCOIND_URL         e.g. http://foo:rpcpassword@host.docker.internal:18443
#   KEY_STORE_PASSWORD   keystore password for member/user keys
#   USER_BITCOIN_WIF     regtest WIF for user transactions
#
# Optional:
#   E2E_OP_ID            operator id to test against (1..4, default 1)
#   E2E_V031_REF         git ref of the v0.3.1 baseline (default v0.3.1)
#   E2E_V031_WORKTREE    path for the v0.3.1 worktree (default /tmp/ubc-v031)
#   E2E_AUTO_CONFIRM     set to 1 to skip destructive-action prompts
#
# ----------------------------------------------------------------------------
# Mapping local ↔ testnet (read-only constraint on testnet — do not modify it)
# ----------------------------------------------------------------------------
#
#   Local step                                    Testnet equivalent
#   -----------------------------------------     --------------------------------------------------
#   git worktree add ... v0.3.1                   no-op (operator-NN already runs v0.3.1)
#   ./cli-infra.sh --start --fresh                no-op (testnet infra is permanent)
#   ./cli-setup-operators.sh --ops 1 -y           no-op (operator-NN already provisioned)
#   ./cli-run.sh --id 1                           coordinator container running on operator-NN
#   tests/run-flows.sh ...                        organic flow activity in testnet
#   ./cli-run.sh --kill                           docker compose down on operator-NN
#   tar czf snapshot.tar.gz                       same, but on operator-NN
#   migrate-v031 <db-path> --config <toml>        SCP binary in, run on operator-NN
#   ./cli-run.sh --id 1 (v0.4.x)                  docker compose up -d (v0.4.x image) on op-NN
#
# Cross-compile the binary for testnet hosts (linux x86_64) from a macOS arm64
# dev box:
#
#   rustup target add x86_64-unknown-linux-gnu
#   cargo build --release --target x86_64-unknown-linux-gnu -p migrate-v031
#   scp target/x86_64-unknown-linux-gnu/release/migrate-v031 \
#       ubuntu@operator-NN.testnet.ub.iovlabs.net:~/
#
# ----------------------------------------------------------------------------

set -euo pipefail

# Resolve repo root (this script lives at <repo>/tools/migrate-v031/).
REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "${REPO_ROOT}"

E2E_OP_ID="${E2E_OP_ID:-1}"
E2E_V031_REF="${E2E_V031_REF:-v0.3.1}"
E2E_V031_WORKTREE="${E2E_V031_WORKTREE:-/tmp/ubc-v031}"
E2E_AUTO_CONFIRM="${E2E_AUTO_CONFIRM:-0}"

# ---- helpers ---------------------------------------------------------------

log()  { printf "\033[1;36m==>\033[0m %s\n" "$*"; }
warn() { printf "\033[1;33m!!\033[0m %s\n" "$*" >&2; }
fail() { printf "\033[1;31mxx\033[0m %s\n" "$*" >&2; exit 1; }

confirm() {
    local prompt="$1"
    if [[ "${E2E_AUTO_CONFIRM}" == "1" ]]; then
        log "auto-confirm: ${prompt}"
        return 0
    fi
    read -r -p "${prompt} [yes/no]: " ans
    [[ "${ans}" == "yes" ]] || fail "aborted by operator"
}

require_env() {
    local missing=()
    for v in BASE_STORAGE_PATH BITCOIND_URL KEY_STORE_PASSWORD USER_BITCOIN_WIF; do
        eval "val=\${${v}:-}"
        [[ -n "${val}" ]] || missing+=("${v}")
    done
    if (( ${#missing[@]} > 0 )); then
        fail "missing required env vars: ${missing[*]} (see the comment header for details)"
    fi
}

# ---- prereq check ----------------------------------------------------------

cmd_check() {
    log "Checking required commands..."
    for c in cargo docker tar git; do
        command -v "$c" >/dev/null || fail "missing command: $c"
    done

    log "Checking env vars..."
    require_env

    log "Checking sibling repos..."
    [[ -d "${REPO_ROOT}/../rust-bitvmx-client"   ]] || warn "../rust-bitvmx-client not found (some flows may need it)"
    [[ -d "${REPO_ROOT}/../union-bridge-contracts" ]] || warn "../union-bridge-contracts not found (full builds may need it)"

    log "Checking docker daemon..."
    docker info >/dev/null 2>&1 || fail "docker daemon not running"

    log "Checking v0.3.1 ref..."
    git rev-parse --verify "${E2E_V031_REF}^{commit}" >/dev/null 2>&1 \
        || fail "git ref '${E2E_V031_REF}' is not reachable; fetch tags first (git fetch --tags origin)"

    log "All prereqs OK."
}

# ---- worktrees -------------------------------------------------------------

ensure_v031_worktree() {
    if [[ -d "${E2E_V031_WORKTREE}" ]]; then
        log "v0.3.1 worktree already present at ${E2E_V031_WORKTREE}"
        return
    fi
    log "Adding v0.3.1 worktree at ${E2E_V031_WORKTREE}"
    git worktree add "${E2E_V031_WORKTREE}" "${E2E_V031_REF}"
}

# ---- E2E sequence ----------------------------------------------------------

step_start_infra() {
    log "Starting infrastructure (cli-infra.sh --start --fresh) ..."
    confirm "This will tear down any running infra. Proceed?"
    "${REPO_ROOT}/cli-infra.sh" --start --fresh
}

step_setup_operator() {
    log "Provisioning operator op_${E2E_OP_ID} (cli-setup-operators.sh --ops ${E2E_OP_ID})"
    confirm "cli-setup-operators.sh removes existing op_${E2E_OP_ID} state under ${BASE_STORAGE_PATH}/.union_bridge/. Proceed?"
    "${REPO_ROOT}/cli-setup-operators.sh" --ops "${E2E_OP_ID}" --yes
}

step_run_v031_and_populate() {
    log "Building and running v0.3.1 from ${E2E_V031_WORKTREE} ..."
    pushd "${E2E_V031_WORKTREE}" >/dev/null
    cargo build --release -p coordinator
    "${E2E_V031_WORKTREE}/cli-run.sh" --id "${E2E_OP_ID}" &
    local v031_pid=$!
    popd >/dev/null

    log "Waiting 15s for v0.3.1 coordinator to come up ..."
    sleep 15

    log "Populating state via tests/run-flows.sh (setup + committee + pegin + pegout)"
    bash "${E2E_V031_WORKTREE}/tests/run-flows.sh" --env local --ops "${E2E_OP_ID}" --setup
    bash "${E2E_V031_WORKTREE}/tests/run-flows.sh" --env local --ops "${E2E_OP_ID}" --committee
    bash "${E2E_V031_WORKTREE}/tests/run-flows.sh" --env local --ops "${E2E_OP_ID}" --pegin
    bash "${E2E_V031_WORKTREE}/tests/run-flows.sh" --env local --ops "${E2E_OP_ID}" --pegout

    log "Stopping v0.3.1 coordinator ..."
    "${E2E_V031_WORKTREE}/cli-run.sh" --kill || true
    wait "${v031_pid}" 2>/dev/null || true
}

step_snapshot() {
    local snap="${BASE_STORAGE_PATH}/op_${E2E_OP_ID}_pre_v04x_$(date +%s).tar.gz"
    log "Snapshotting operator state to ${snap}"
    tar czf "${snap}" -C "${BASE_STORAGE_PATH}" ".union_bridge/op_${E2E_OP_ID}"
    echo "${snap}" > "${BASE_STORAGE_PATH}/.last_snapshot_op_${E2E_OP_ID}"
}

step_run_migrator() {
    log "Building migrate-v031 ..."
    cargo build --release -p migrate-v031

    local db="${BASE_STORAGE_PATH}/.union_bridge/op_${E2E_OP_ID}/local_database/coordinator"
    [[ -d "${db}" ]] || fail "coordinator DB not found at ${db}"

    log "Running migrate-v031 against ${db}"
    "${REPO_ROOT}/target/release/migrate-v031" "${db}"
}

step_run_v04x_and_verify() {
    log "Running v0.4.x coordinator from ${REPO_ROOT} (current branch) ..."
    "${REPO_ROOT}/cli-run.sh" --id "${E2E_OP_ID}" &
    local v04x_pid=$!

    log "Waiting 30s for v0.4.x to start and restore flows (look for 'Restored M ... flows' in logs) ..."
    sleep 30

    if ! kill -0 "${v04x_pid}" 2>/dev/null; then
        fail "v0.4.x coordinator exited unexpectedly; check ${BASE_STORAGE_PATH}/.union_bridge/op_${E2E_OP_ID} logs"
    fi

    log "Continuing flows under v0.4.x (pegin + pegout)"
    bash "${REPO_ROOT}/tests/run-flows.sh" --env local --ops "${E2E_OP_ID}" --pegin
    bash "${REPO_ROOT}/tests/run-flows.sh" --env local --ops "${E2E_OP_ID}" --pegout

    log "Stopping v0.4.x coordinator ..."
    "${REPO_ROOT}/cli-run.sh" --kill || true
    wait "${v04x_pid}" 2>/dev/null || true
}

cmd_run() {
    cmd_check
    ensure_v031_worktree
    step_start_infra
    step_setup_operator
    step_run_v031_and_populate
    step_snapshot
    step_run_migrator
    step_run_v04x_and_verify
    log "E2E PASSED. Snapshot at $(cat "${BASE_STORAGE_PATH}/.last_snapshot_op_${E2E_OP_ID}")."
}

# ---- cleanup ---------------------------------------------------------------

cmd_cleanup() {
    log "Cleaning up ..."

    if [[ -d "${E2E_V031_WORKTREE}" ]]; then
        confirm "Remove v0.3.1 worktree at ${E2E_V031_WORKTREE}?"
        git worktree remove --force "${E2E_V031_WORKTREE}"
    fi

    if "${REPO_ROOT}/cli-run.sh" --kill 2>/dev/null; then
        log "Killed any local coordinators."
    fi

    if command -v docker >/dev/null && docker info >/dev/null 2>&1; then
        confirm "Stop infra (cli-infra.sh --stop)?"
        "${REPO_ROOT}/cli-infra.sh" --stop || true
    fi

    log "Cleanup done."
}

# ---- entry point -----------------------------------------------------------

usage() {
    sed -n '2,/^$/p' "${BASH_SOURCE[0]}"
    exit 1
}

case "${1:-}" in
    --check)   cmd_check ;;
    --run)     cmd_run ;;
    --cleanup) cmd_cleanup ;;
    *)         usage ;;
esac
