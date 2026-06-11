#!/usr/bin/env bash

# Pegout recovery e2e.
#
# Starts from the same prerequisites as scripts/test-flows.sh: infra and clients
# must already be running. By default this script prepares setup/committee state,
# completes a normal pegin to fund the user on Rootstock, requests a pegout,
# restarts coordinators without wiping storage, and then waits for the
# correlated pegout completion marker and balance changes.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"

# shellcheck disable=SC1091
source "${SCRIPT_DIR}/test-flows.sh"

RUN_PREREQS=true
RECOVERY_SETTLE_SECONDS=3

usage() {
    local script_name
    script_name=$(basename "${BASH_SOURCE[0]}")
    cat <<EOF
Usage: ${script_name} [--env <local-anvil|docker-anvil|local-rskj|docker-rskj>] [--ops <1-10>] [--stream <0-4>] [--skip-prereqs] [--settle-seconds <N>]

Runs a pegout recovery e2e:
  1. Optionally runs setup + committee + a normal pegin prerequisite.
  2. Requests a pegout.
  3. Waits briefly so processors can observe and persist in-flight state.
  4. Restarts coordinators without wiping storage.
  5. Waits for the correlated pegout completion marker and balance checks.

Options:
  --env              Environment: local-anvil, docker-anvil, local-rskj, or docker-rskj.
  --ops              Number of operators (1-10). Same semantics as scripts/test-flows.sh.
  --stream           Stream identifier (0-4). Defaults to 0.
  --skip-prereqs     Do not run setup, committee, or pegin; assume pegout prerequisites exist.
  --settle-seconds   Seconds to wait after request before restart. Defaults to ${RECOVERY_SETTLE_SECONDS}.
  --help, -h         Show this help text.

Examples:
  bash scripts/test-pegout-recovery.sh --env docker-anvil
  bash scripts/test-pegout-recovery.sh --env local-anvil --skip-prereqs --settle-seconds 8
EOF
}

parse_recovery_args() {
    while [[ $# -gt 0 ]]; do
        case "$1" in
        --env)
            SCRIPT_ENV="${2:-}"
            if [[ -z "$SCRIPT_ENV" ]]; then
                usage
                return 1
            fi
            shift 2
            ;;
        --ops)
            OPS_FROM_FLAG="${2:-}"
            if [[ -z "$OPS_FROM_FLAG" ]] || ! [[ "$OPS_FROM_FLAG" =~ ^(10|[1-9])$ ]]; then
                echo "Error: --ops must be between 1 and 10" >&2
                return 1
            fi
            shift 2
            ;;
        --stream)
            STREAM_ID="${2:-}"
            if [[ -z "$STREAM_ID" ]] || ! [[ "$STREAM_ID" =~ ^[0-4]$ ]]; then
                echo "Error: --stream must be between 0 and 4" >&2
                return 1
            fi
            shift 2
            ;;
        --skip-prereqs)
            RUN_PREREQS=false
            shift
            ;;
        --settle-seconds)
            RECOVERY_SETTLE_SECONDS="${2:-}"
            if [[ -z "$RECOVERY_SETTLE_SECONDS" ]] || ! [[ "$RECOVERY_SETTLE_SECONDS" =~ ^[0-9]+$ ]]; then
                echo "Error: --settle-seconds must be a non-negative integer" >&2
                return 1
            fi
            shift 2
            ;;
        --help|-h)
            usage
            exit 0
            ;;
        *)
            usage
            return 1
            ;;
        esac
    done
}

correlated_completion_marker_count() {
    local kind="$1"
    local matches_file
    matches_file=$(mktemp)
    collect_matching_completion_marker_refs "$kind" "$matches_file"
    count_completion_marker_refs_in_file "$matches_file"
    rm -f "$matches_file"
}

assert_no_correlated_completion_marker() {
    local kind="$1"
    local count
    count=$(correlated_completion_marker_count "$kind")
    if (( count > 0 )); then
        warn "Found ${count} correlated ${kind} completion marker(s) before restart"
        warn "The flow completed before the recovery cut; rerun with a smaller --settle-seconds value"
        return 1
    fi
}

docker_operator_env_file_abs() {
    echo "${REPO_ROOT}/$(docker_operator_env_file)"
}

restart_docker_coordinators_for_recovery() {
    local env_file
    env_file=$(docker_operator_env_file_abs)

    log "Restarting Docker coordinator services with ${env_file}"
    bash "${REPO_ROOT}/docker/operator/start-operators.sh" \
        --env-file "$env_file" \
        --ops "$NUM_OPERATORS" \
        restart coordinator

    wait_for_docker_coordinator_health
}

start_local_clients_for_recovery() {
    local run_log_dir
    run_log_dir="${REPO_ROOT}/logs/recovery-$(date +%y%m%d-%H%M%S)"
    mkdir -p "$run_log_dir"

    local -a run_args=(--services coordinator)
    if is_rskj_backend_env; then
        run_args+=(--rskj)
    fi

    log "Starting local coordinators in background without --fresh"
    log "Local recovery logs: ${run_log_dir}/run-clients.log"
    env UB_LOG_DIR="$run_log_dir" \
        nohup bash "${SCRIPT_DIR}/run-clients.sh" "${run_args[@]}" \
        > "${run_log_dir}/run-clients.log" 2>&1 &

    local run_pid=$!
    printf '%s\n' "$run_pid" > "${run_log_dir}/run-clients.pid"
    log "Local clients launcher PID: ${run_pid}"
}

stop_local_run_launchers_for_recovery() {
    local launcher_pids
    launcher_pids=$(pgrep -f "target/debug/run" || true)
    if [[ -z "$launcher_pids" ]]; then
        return 0
    fi

    log "Stopping local run launcher process(es) before coordinator restart"
    while IFS= read -r pid; do
        [[ -n "$pid" ]] || continue
        kill "$pid" 2>/dev/null || true
    done <<< "$launcher_pids"

    sleep 1

    while IFS= read -r pid; do
        [[ -n "$pid" ]] || continue
        if kill -0 "$pid" 2>/dev/null; then
            kill -9 "$pid" 2>/dev/null || true
        fi
    done <<< "$launcher_pids"
}

restart_local_clients_for_recovery() {
    log "Stopping local coordinator services without deleting storage"
    stop_local_run_launchers_for_recovery
    bash "${SCRIPT_DIR}/run-clients.sh" --kill --services coordinator
    start_local_clients_for_recovery
    wait_for_test_prereqs
}

restart_coordinators_for_recovery() {
    step "Recovery cut: restart coordinators"
    if is_docker_mode_env; then
        restart_docker_coordinators_for_recovery
    else
        restart_local_clients_for_recovery
    fi
    success "Coordinator runtime restarted"
    echo ""
}

initialize_pegout_recovery() {
    load_envrc_if_needed
    initialize_script_env_default
    parse_recovery_args "$@" || return 1
    validate_script_env || return 1
    ensure_selected_env_matches_running_mode || return 1

    cd "$REPO_ROOT"

    load_contract_addresses || return 1
    load_num_operators

    if [[ "$RUN_PREREQS" == true ]]; then
        # shellcheck disable=SC2034
        MODE="happy"
    else
        # shellcheck disable=SC2034
        MODE="pegout"
    fi
    initialize_mode_config

    check_required_commands || return 1
    wait_for_test_prereqs || return 1
    ensure_force_advance_inactive_unless_requested || return 1

    log "Configuration: recovery=pegout, env=${SCRIPT_ENV}, ops=${NUM_OPERATORS}, stream=${STREAM_ID}, amount=${VALUE}, run_prereqs=${RUN_PREREQS}, settle_seconds=${RECOVERY_SETTLE_SECONDS}"
    echo ""
}

run_pegout_recovery_phase() {
    step "Pegout Recovery"

    local pegout_start_time
    pegout_start_time=$(date +%s)
    # Consumed by completion marker helpers from scripts/test-flows.sh.
    # shellcheck disable=SC2034
    EXPECTED_PEGOUT_STARTED_AT_EPOCH="$pegout_start_time"
    RSK_ADDRESS=$(resolve_user_rsk_address) || return 1

    local user_btc_balance_before_sat
    user_btc_balance_before_sat=$(user_btc_balance_sat)
    local user_rsk_balance_before_wei
    user_rsk_balance_before_wei=$(user_rsk_balance_wei "$RSK_ADDRESS")

    local user_compressed_pubkey
    user_compressed_pubkey=$(user_compressed_pubkey_from_wif)
    if [[ -z "$user_compressed_pubkey" ]]; then
        warn "Failed to derive user compressed public key from WIF"
        return 1
    fi

    log "Command: bash scripts/operations.sh user pegout -v $VALUE -k $user_compressed_pubkey --env $SCRIPT_ENV"
    log "Amount: $VALUE sats"
    log "USR Pub Key: $user_compressed_pubkey"
    echo ""

    EXPECTED_REQUEST_PEGOUT_TX_HASH=$(run_user_pegout_and_capture_request_tx_hash "$VALUE" "$user_compressed_pubkey") || return 1
    success "Pegout requested: ${EXPECTED_REQUEST_PEGOUT_TX_HASH}"
    echo ""

    if (( RECOVERY_SETTLE_SECONDS > 0 )); then
        log "Waiting ${RECOVERY_SETTLE_SECONDS}s before restart so processor state can be persisted"
        sleep "$RECOVERY_SETTLE_SECONDS"
    fi

    assert_no_correlated_completion_marker "pegout" || return 1
    restart_coordinators_for_recovery || return 1

    local max_blocks
    max_blocks=$(user_flow_completion_max_blocks)
    if ! wait_for_correlated_completion_markers "pegout" "$NUM_OPERATORS" "$max_blocks"; then
        warn "Pegout completion marker not found after recovery restart"
        return 1
    fi
    echo ""

    verify_user_btc_balance_increase_for_value "$user_btc_balance_before_sat" || return 1
    verify_user_rsk_balance_decrease_for_value "$user_rsk_balance_before_wei" || return 1

    local pegout_end_time
    pegout_end_time=$(date +%s)
    PEGOUT_DURATION=$((pegout_end_time - pegout_start_time))
    success "Pegout recovery completed in $(format_duration "$PEGOUT_DURATION")"
    echo ""
}

main() {
    initialize_pegout_recovery "$@" || return 1

    if [[ "$RUN_PREREQS" == true ]]; then
        run_setup_and_committee_phases || return 1
        run_pegin_phase || return 1
    fi

    run_pegout_recovery_phase || return 1

    step "Complete"
    success "Pegout recovery e2e completed successfully"
}

main "$@"
