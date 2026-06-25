#!/usr/bin/env bash

# Local Anvil E2E for operator-take retrigger handling.
#
# This intentionally reuses scripts/test-flows.sh helpers instead of duplicating
# wallet, marker, and FORCE_ADVANCE plumbing.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_ROOT"

# shellcheck source=scripts/test-flows.sh
source "${REPO_ROOT}/scripts/test-flows.sh"

readonly OPERATOR_TAKE_TRIGGERED_TOPIC="0x3ab74991326b5f5d68212942cb51a1bea6850b7f0bdc3a3dc6ab01ce30f55e19"
readonly PEGOUT_REQUESTED_TOPIC="0x12c37783fbba03764e845b87b13db2607d9ba6b88305a716bde5d1e6112065f9"
readonly LOCAL_ANVIL_SENDER="0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266"

E2E_ENV="local-anvil"
E2E_OPS="4"
E2E_STREAM="0"
E2E_REPORT_DIR="target/e2e-reports"
E2E_SMOKE_ONLY=false
E2E_OPERATOR_TIMEOUT_PADDING_SECS=2
E2E_EVENT_POLL_SECS=0.5
E2E_FIRST_TRIGGER_MAX_BLOCKS=300
E2E_SECOND_TRIGGER_MAX_BLOCKS=60
E2E_USER_FLOW_MAX_BLOCKS=120
E2E_PEGOUT_TXID_OVERRIDE=""

E2E_STARTED_AT="$(date -u +"%Y-%m-%dT%H:%M:%SZ")"
E2E_RUN_ID="$(date -u +"%Y%m%dT%H%M%SZ")"
E2E_REPORT_PATH=""
E2E_RUN_LOG=""
E2E_STATUS="FAIL"
E2E_FAILURE_REASON=""
E2E_PEGOUT_MANAGER_ADDRESS=""
E2E_SIGNATURE_MANAGER_ADDRESS=""
E2E_REQUEST_PEGOUT_TX_HASH=""
E2E_ACCEPT_PEGIN_TXID=""
E2E_PEGOUT_TXID=""
E2E_SECOND_TRIGGER_TX_HASH=""
E2E_LOG_DIR=""
E2E_LOG_OFFSET_FILE=""

usage() {
    cat <<EOF
Usage: $(basename "$0") [options]

Runs a local-anvil E2E that verifies operator-take retrigger handling:
  setup + committee -> pegin -> operator-take request -> second OperatorTakeTriggered -> completion markers.

Options:
  --env local-anvil        Environment to use. Only local-anvil is supported by this retrigger script.
  --ops N                 Number of operators. Defaults to 4.
  --stream N              Stream id. Defaults to 0.
  --smoke-only            Run the existing operator-take path without forcing a second trigger.
  --pegout-txid 0x...     Use this pegout txid instead of reading it from the first OperatorTakeTriggered event.
  --report-dir DIR        Where to write the markdown report. Defaults to target/e2e-reports.
  --help                  Show this help.

Prerequisites:
  ./scripts/setup-operators.sh --ops 4 -y
  ./scripts/run-infra.sh --env local-anvil --start-all --fresh
  ./scripts/run-clients.sh --fresh
EOF
}

parse_e2e_args() {
    while [[ $# -gt 0 ]]; do
        case "$1" in
        --env)
            E2E_ENV="${2:-}"
            shift 2
            ;;
        --ops)
            E2E_OPS="${2:-}"
            shift 2
            ;;
        --stream)
            E2E_STREAM="${2:-}"
            shift 2
            ;;
        --smoke-only)
            E2E_SMOKE_ONLY=true
            shift
            ;;
        --pegout-txid)
            E2E_PEGOUT_TXID_OVERRIDE="${2:-}"
            shift 2
            ;;
        --report-dir)
            E2E_REPORT_DIR="${2:-}"
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

    if [[ "$E2E_ENV" != "local-anvil" ]]; then
        echo "Error: this retrigger E2E only supports --env local-anvil" >&2
        return 1
    fi
    if [[ -z "$E2E_OPS" ]] || ! [[ "$E2E_OPS" =~ ^(10|[1-9])$ ]]; then
        echo "Error: --ops must be between 1 and 10" >&2
        return 1
    fi
    if [[ -z "$E2E_STREAM" ]] || ! [[ "$E2E_STREAM" =~ ^[0-4]$ ]]; then
        echo "Error: --stream must be between 0 and 4" >&2
        return 1
    fi
    if [[ -n "$E2E_PEGOUT_TXID_OVERRIDE" ]] && ! [[ "$E2E_PEGOUT_TXID_OVERRIDE" =~ ^0x[0-9a-fA-F]{64}$ ]]; then
        echo "Error: --pegout-txid must be a 32-byte hex value with 0x prefix" >&2
        return 1
    fi
}

user_flow_completion_max_blocks() {
    echo "$E2E_USER_FLOW_MAX_BLOCKS"
}

setup_report_files() {
    mkdir -p "$E2E_REPORT_DIR"
    E2E_REPORT_PATH="${E2E_REPORT_DIR}/operator-take-retrigger-${E2E_RUN_ID}.md"
    E2E_RUN_LOG="${E2E_REPORT_DIR}/operator-take-retrigger-${E2E_RUN_ID}.log"
    E2E_LOG_OFFSET_FILE="${E2E_REPORT_DIR}/operator-take-retrigger-${E2E_RUN_ID}.log-offsets"
    touch "$E2E_RUN_LOG"
}

runtime_logs_dir() {
    if [[ -n "${UB_LOG_DIR:-}" ]]; then
        printf '%s\n' "$UB_LOG_DIR"
        return
    fi
    if [[ -L "logs/latest" ]]; then
        printf '%s\n' "logs/$(readlink logs/latest)"
        return
    fi
    printf '%s\n' "logs"
}

write_report() {
    local exit_code="$1"
    local finished_at
    finished_at="$(date -u +"%Y-%m-%dT%H:%M:%SZ")"

    {
        echo "# Operator Take Retrigger E2E"
        echo
        echo "- status: ${E2E_STATUS}"
        echo "- exit_code: ${exit_code}"
        echo "- reason: ${E2E_FAILURE_REASON:-n/a}"
        echo "- started_at: ${E2E_STARTED_AT}"
        echo "- finished_at: ${finished_at}"
        echo "- commit: $(git rev-parse --short HEAD 2>/dev/null || echo unknown)"
        echo "- env: ${E2E_ENV}"
        echo "- ops: ${E2E_OPS}"
        echo "- stream: ${E2E_STREAM}"
        echo "- pegout_manager: ${E2E_PEGOUT_MANAGER_ADDRESS:-unknown}"
        echo "- signature_manager: ${E2E_SIGNATURE_MANAGER_ADDRESS:-unknown}"
        echo "- request_pegout_tx_hash: ${E2E_REQUEST_PEGOUT_TX_HASH:-unknown}"
        echo "- accept_pegin_txid: ${E2E_ACCEPT_PEGIN_TXID:-unknown}"
        echo "- pegout_txid: ${E2E_PEGOUT_TXID:-unknown}"
        echo "- second_trigger_tx_hash: ${E2E_SECOND_TRIGGER_TX_HASH:-unknown}"
        echo "- runtime_logs_dir: ${E2E_LOG_DIR:-unknown}"
        echo "- raw_log: ${E2E_RUN_LOG}"
        echo
        echo "## Commands"
        echo
        echo '```bash'
        echo "bash scripts/test-operator-take-retrigger.sh --env ${E2E_ENV} --ops ${E2E_OPS} --stream ${E2E_STREAM}"
        echo '```'
        echo
        echo "## Evidence"
        echo
        if [[ -f "$E2E_RUN_LOG" ]]; then
            grep -E "Operator-take pegout requested|First OperatorTakeTriggered|Second OperatorTakeTriggered|advance-funds correlated completion marker detected|Superseding existing advance funds flow|Invalid state transition|Unauthorized fingerprint|BitVMX is not responding" "$E2E_RUN_LOG" || true
        else
            echo "Raw log was not written."
        fi
    } > "$E2E_REPORT_PATH"
}

finalize() {
    local exit_code=$?
    restore_force_advance >/dev/null 2>&1 || true

    if [[ "$exit_code" -ne 0 && -z "$E2E_FAILURE_REASON" ]]; then
        E2E_FAILURE_REASON="script exited before completing all assertions"
    fi

    write_report "$exit_code" || true
    echo "E2E report: ${E2E_REPORT_PATH}"
}

fail_e2e() {
    E2E_FAILURE_REASON="$1"
    echo "Error: $1" >&2
    return 1
}

initialize_test_flows_context() {
    SCRIPT_ENV="$E2E_ENV"
    OPS_FROM_FLAG="$E2E_OPS"
    STREAM_ID="$E2E_STREAM"
    MODE="operator-take"

    load_envrc_if_needed
    # Mirror test-flows.sh main(), which does not run when sourced.
    # shellcheck source=/dev/null
    source "${REPO_ROOT}/docker/local-infra/bitcoind-rpc-env.sh"
    BITCOIN_WALLET="${BITCOIN_WALLET:-mainwallet}"
    load_contract_addresses || return 1
    load_num_operators
    initialize_mode_config
    USER_UTXO_VALUE=$(derived_wallet_utxo_value "$STREAM_DENOMINATION")
    MEMBER_UTXO_VALUE=$(derived_member_wallet_utxo_value "$STREAM_ID" "$COMMITTEE_MEMBER_COUNT")
    check_required_commands || return 1
    wait_for_test_prereqs || return 1
    ensure_force_advance_inactive_unless_requested || return 1

    local config_file
    config_file=$(contract_config_file)
    E2E_PEGOUT_MANAGER_ADDRESS=$(contract_address_from_config "PegoutManager" "$config_file")
    if [[ -z "$E2E_PEGOUT_MANAGER_ADDRESS" ]]; then
        fail_e2e "failed to resolve PegoutManager address from ${config_file}"
        return 1
    fi
    E2E_SIGNATURE_MANAGER_ADDRESS=$(contract_address_from_config "SignatureManager" "$config_file")
    if [[ -z "$E2E_SIGNATURE_MANAGER_ADDRESS" ]]; then
        fail_e2e "failed to resolve SignatureManager address from ${config_file}"
        return 1
    fi

    E2E_LOG_DIR=$(runtime_logs_dir)
    snapshot_runtime_log_offsets
}

snapshot_runtime_log_offsets() {
    : > "$E2E_LOG_OFFSET_FILE"

    if [[ ! -d "$E2E_LOG_DIR" ]]; then
        return 0
    fi

    local file lines
    for file in "$E2E_LOG_DIR"/coordinator-*.log; do
        [[ -f "$file" ]] || continue
        lines=$(wc -l < "$file" | tr -d ' ')
        printf '%s\t%s\n' "$file" "$lines" >> "$E2E_LOG_OFFSET_FILE"
    done
}

operator_take_logs_json() {
    local from_block="$1"

    cast logs \
        --json \
        --rpc-url "$ROOTSTOCK_RPC_URL" \
        --from-block "$from_block" \
        --to-block latest \
        --address "$E2E_PEGOUT_MANAGER_ADDRESS" \
        "$OPERATOR_TAKE_TRIGGERED_TOPIC"
}

extract_first_operator_take_txid() {
    jq -sr '
        map(if type == "array" then .[] else . end)
        | .[0].topics[1] // empty
    ' | head -1
}

count_operator_take_events_for_txid() {
    local pegout_txid="$1"

    jq -sr --arg txid "$pegout_txid" '
        map(if type == "array" then .[] else . end)
        | map(select((.topics[1] // "" | ascii_downcase) == ($txid | ascii_downcase)))
        | length
    '
}

pegout_txid_from_request_tx_hash() {
    local tx_hash="$1"
    local manager_address
    manager_address=$(printf '%s\n' "$E2E_PEGOUT_MANAGER_ADDRESS" | tr '[:upper:]' '[:lower:]')

    local log_data
    log_data=$(cast receipt \
        --json \
        --rpc-url "$ROOTSTOCK_RPC_URL" \
        "$tx_hash" \
        | jq -r \
            --arg manager_address "$manager_address" \
            --arg topic "$PEGOUT_REQUESTED_TOPIC" '
                .logs[]
                | select((.address | ascii_downcase) == $manager_address)
                | select(.topics[0] == $topic)
                | .data
            ' \
        | head -1)

    if [[ -z "$log_data" || "$log_data" == "null" ]]; then
        return 1
    fi

    cast decode-abi \
        --json \
        'f()(bytes,((uint32,(bytes32,uint32,uint32,bytes)[],(uint64,bytes)[],uint32),bytes32,bytes32,bytes),uint64,uint64,uint64,uint64)' \
        "$log_data" \
        | jq -r '.[1] | capture("\\], [0-9]+\\), (?<txid>0x[0-9a-fA-F]{64}),").txid'
}

accept_pegin_txid_from_request_tx_hash() {
    local tx_hash="$1"
    local manager_address
    manager_address=$(printf '%s\n' "$E2E_PEGOUT_MANAGER_ADDRESS" | tr '[:upper:]' '[:lower:]')

    cast receipt \
        --json \
        --rpc-url "$ROOTSTOCK_RPC_URL" \
        "$tx_hash" \
        | jq -r \
            --arg manager_address "$manager_address" \
            --arg topic "$PEGOUT_REQUESTED_TOPIC" '
                .logs[]
                | select((.address | ascii_downcase) == $manager_address)
                | select(.topics[0] == $topic)
                | .topics[1]
            ' \
        | head -1
}

wait_for_operator_take_candidate() {
    local pegout_txid="$1"
    local start_height
    start_height=$(cast block-number --rpc-url "$ROOTSTOCK_RPC_URL")
    local target_height=$((start_height + 60))

    log "Waiting for at least one operator signature nonce before triggering operator take..."

    while true; do
        local call_json nonce_count current_height
        call_json=$(cast call \
            --json \
            --rpc-url "$ROOTSTOCK_RPC_URL" \
            "$E2E_SIGNATURE_MANAGER_ADDRESS" \
            "getPartialSignatures(bytes32)((bytes32,bytes)[],uint8,uint128)" \
            "$pegout_txid" 2>/dev/null || true)
        nonce_count=$(printf '%s\n' "$call_json" \
            | jq -r '.[0] // ""' 2>/dev/null \
            | grep -Eo ', 0x[0-9a-fA-F]{2,}' \
            | wc -l \
            | tr -d ' ')

        if [[ "$nonce_count" =~ ^[0-9]+$ && "$nonce_count" -gt 0 ]]; then
            success "Operator-take candidate signature data observed (${nonce_count} nonce(s))"
            return 0
        fi

        current_height=$(cast block-number --rpc-url "$ROOTSTOCK_RPC_URL")
        if (( current_height >= target_height )); then
            fail_e2e "operator-take candidate signature data not observed before block ${target_height}"
            return 1
        fi

        sleep "$E2E_EVENT_POLL_SECS"
    done
}

wait_for_first_operator_take_triggered() {
    local from_block="$1"
    local start_height
    start_height=$(cast block-number --rpc-url "$ROOTSTOCK_RPC_URL")
    local target_height=$((start_height + E2E_FIRST_TRIGGER_MAX_BLOCKS))

    log "Waiting for first OperatorTakeTriggered event (max ${E2E_FIRST_TRIGGER_MAX_BLOCKS} blocks)..." >&2

    while true; do
        local logs_json pegout_txid current_height
        logs_json=$(operator_take_logs_json "$from_block")
        pegout_txid=$(printf '%s\n' "$logs_json" | extract_first_operator_take_txid)

        if [[ -n "$pegout_txid" && "$pegout_txid" =~ ^0x[0-9a-fA-F]{64}$ ]]; then
            printf '%s\n' "$pegout_txid"
            return 0
        fi

        current_height=$(cast block-number --rpc-url "$ROOTSTOCK_RPC_URL")
        if (( current_height >= target_height )); then
            fail_e2e "first OperatorTakeTriggered event not found before block ${target_height}"
            return 1
        fi

        sleep "$E2E_EVENT_POLL_SECS"
    done
}

wait_for_second_operator_take_triggered() {
    local from_block="$1"
    local pegout_txid="$2"
    local start_height
    start_height=$(cast block-number --rpc-url "$ROOTSTOCK_RPC_URL")
    local target_height=$((start_height + E2E_SECOND_TRIGGER_MAX_BLOCKS))

    log "Waiting for second OperatorTakeTriggered event for ${pegout_txid}..."

    while true; do
        local logs_json count current_height
        logs_json=$(operator_take_logs_json "$from_block")
        count=$(printf '%s\n' "$logs_json" | count_operator_take_events_for_txid "$pegout_txid")

        if [[ "$count" -ge 2 ]]; then
            success "Second OperatorTakeTriggered observed for ${pegout_txid}"
            return 0
        fi

        current_height=$(cast block-number --rpc-url "$ROOTSTOCK_RPC_URL")
        if (( current_height >= target_height )); then
            fail_e2e "second OperatorTakeTriggered event not found before block ${target_height}"
            return 1
        fi

        sleep "$E2E_EVENT_POLL_SECS"
    done
}

operator_take_timeout_secs() {
    local raw timeout
    raw=$(cast call \
        --rpc-url "$ROOTSTOCK_RPC_URL" \
        "$E2E_PEGOUT_MANAGER_ADDRESS" \
        "operatorTakeTimeout()(uint256)" 2>/dev/null || true)

    if [[ -n "$raw" ]]; then
        timeout=$(cast to-dec "$raw" 2>/dev/null || true)
        if [[ "$timeout" =~ ^[0-9]+$ && "$timeout" -gt 0 ]]; then
            printf '%s\n' "$timeout"
            return 0
        fi
    fi

    printf '%s\n' "120"
}

user_take_timeout_secs() {
    local raw timeout
    raw=$(cast call \
        --rpc-url "$ROOTSTOCK_RPC_URL" \
        "$E2E_PEGOUT_MANAGER_ADDRESS" \
        "userTakeTimeout()(uint256)" 2>/dev/null || true)

    if [[ -n "$raw" ]]; then
        timeout=$(cast to-dec "$raw" 2>/dev/null || true)
        if [[ "$timeout" =~ ^[0-9]+$ && "$timeout" -gt 0 ]]; then
            printf '%s\n' "$timeout"
            return 0
        fi
    fi

    printf '%s\n' "600"
}

advance_user_take_timeout() {
    local timeout_secs
    timeout_secs=$(user_take_timeout_secs)
    local increase_secs=$((timeout_secs + E2E_OPERATOR_TIMEOUT_PADDING_SECS))

    log "Advancing Anvil time by ${increase_secs}s to expire the user take..."
    cast rpc evm_increaseTime "$increase_secs" --rpc-url "$ROOTSTOCK_RPC_URL" >/dev/null
}

advance_operator_take_timeout() {
    local timeout_secs
    timeout_secs=$(operator_take_timeout_secs)
    local increase_secs=$((timeout_secs + E2E_OPERATOR_TIMEOUT_PADDING_SECS))

    log "Advancing Anvil time by ${increase_secs}s to expire the selected operator take..."
    cast rpc evm_increaseTime "$increase_secs" --rpc-url "$ROOTSTOCK_RPC_URL" >/dev/null
}

trigger_operator_take_for_e2e() {
    local accept_pegin_txid="$1"
    local label="$2"
    local output tx_hash

    log "Command: cast send ${E2E_PEGOUT_MANAGER_ADDRESS} triggerOperatorTake(bytes32) ${accept_pegin_txid}"
    output=$(cast send \
        --rpc-url "$ROOTSTOCK_RPC_URL" \
        --from "$LOCAL_ANVIL_SENDER" \
        "$E2E_PEGOUT_MANAGER_ADDRESS" \
        "triggerOperatorTake(bytes32)" \
            "$accept_pegin_txid" \
            --unlocked 2>&1) || {
            printf '%s\n' "$output" >&2
            E2E_FAILURE_REASON="${label} trigger cast send failed"
            return 1
        }

    printf '%s\n' "$output"
    tx_hash=$(printf '%s\n' "$output" | grep -Eo '0x[0-9a-fA-F]{64}' | head -1 || true)
    if [[ "$label" == "second" ]]; then
        E2E_SECOND_TRIGGER_TX_HASH="$tx_hash"
    fi
}

trigger_operator_take_first() {
    trigger_operator_take_for_e2e "$1" "first"
}

trigger_operator_take_again() {
    trigger_operator_take_for_e2e "$1" "second"
}

wait_for_log_pattern() {
    local pattern="$1"
    local max_seconds="$2"
    local deadline=$((SECONDS + max_seconds))

    while (( SECONDS < deadline )); do
        if grep_runtime_logs_since_start "$pattern" >/dev/null; then
            return 0
        fi
        sleep 2
    done

    return 1
}

grep_runtime_logs_since_start() {
    local pattern="$1"

    if [[ ! -d "$E2E_LOG_DIR" ]]; then
        return 1
    fi

    local file offset found=false
    for file in "$E2E_LOG_DIR"/coordinator-*.log; do
        [[ -f "$file" ]] || continue
        offset=0
        if [[ -f "$E2E_LOG_OFFSET_FILE" ]]; then
            offset=$(awk -v file="$file" '$1 == file { print $2; found = 1 } END { if (!found) print 0 }' "$E2E_LOG_OFFSET_FILE")
        fi
        if tail -n "+$((offset + 1))" "$file" | grep -n "$pattern"; then
            found=true
        fi
    done

    [[ "$found" == true ]]
}

assert_no_bad_log_patterns() {
    local bad_patterns=(
        "Invalid state transition"
        "Invalid state transition from SetVarBitVmxAdvanceFundsRegistered with OperatorTakeSPV"
        "Unauthorized fingerprint"
    )

    if [[ ! -d "$E2E_LOG_DIR" ]]; then
        warn "Runtime log dir not found: ${E2E_LOG_DIR}; skipping negative log checks"
        return 0
    fi

    local pattern
    for pattern in "${bad_patterns[@]}"; do
        if grep_runtime_logs_since_start "$pattern"; then
            fail_e2e "bad log pattern found: ${pattern}"
            return 1
        fi
    done
}

assert_selected_completion_marker() {
    local refs_file selected_count selected_payload
    refs_file=$(mktemp)
    collect_matching_completion_marker_refs "advance-funds" "$refs_file"

    selected_count=$(while IFS= read -r ref; do
        [[ -n "$ref" ]] || continue
        completion_marker_payload_json "$ref" | jq -r 'select(.payload.was_selected_operator == true) | .payload.was_selected_operator'
    done < "$refs_file" | wc -l | tr -d ' ')

    if [[ "$selected_count" != "1" ]]; then
        rm -f "$refs_file"
        fail_e2e "expected exactly one selected-operator completion marker, found ${selected_count}"
        return 1
    fi

    selected_payload=$(while IFS= read -r ref; do
        [[ -n "$ref" ]] || continue
        completion_marker_payload_json "$ref" | jq -c 'select(.payload.was_selected_operator == true) | .payload'
    done < "$refs_file" | head -1)

    rm -f "$refs_file"

    if ! printf '%s\n' "$selected_payload" | jq -e \
        --arg request_tx_hash "$EXPECTED_REQUEST_PEGOUT_TX_HASH" \
        '.request_pegout_tx_hash == $request_tx_hash
         and (.accept_pegin_txid | type == "string" and length > 0)
         and (.advance_funds_txid | type == "string" and length > 0)
         and (.selected_operator_address | type == "string" and length > 0)
         and (.pegout_id | type == "string" and length > 0)' \
        > /dev/null; then
        printf '%s\n' "$selected_payload" >&2
        fail_e2e "selected-operator completion marker is missing required payload fields"
        return 1
    fi

    success "Selected-operator completion marker verified"
}

assert_retrigger_log_observed() {
    if wait_for_log_pattern "Superseding existing advance funds flow for operator-take slot" 60; then
        success "Retrigger flow replacement log observed"
        return 0
    fi

    fail_e2e "retrigger flow replacement log was not observed"
}

run_operator_take_retrigger_phase() {
    step "Operator Take Retrigger"

    local operator_take_start_time
    operator_take_start_time=$(date +%s)
    EXPECTED_PEGOUT_STARTED_AT_EPOCH="$operator_take_start_time"

    RSK_ADDRESS=$(resolve_user_rsk_address) || return 1

    local user_btc_balance_before_sat
    user_btc_balance_before_sat=$(user_btc_balance_sat)
    local user_rsk_balance_before_wei
    user_rsk_balance_before_wei=$(user_rsk_balance_wei "$RSK_ADDRESS")

    local target_address
    target_address=$(resolve_force_advance_target_address)

    local user_compressed_pubkey
    user_compressed_pubkey=$(user_compressed_pubkey_from_wif)
    if [[ -z "$user_compressed_pubkey" ]]; then
        fail_e2e "failed to derive user compressed public key from WIF"
        return 1
    fi

    enable_force_advance "$target_address" || return 1

    local operator_take_from_block
    operator_take_from_block=$(cast block-number --rpc-url "$ROOTSTOCK_RPC_URL")

    log "Target operator forced to miss user-take: $target_address"
    log "Command: bash scripts/operations.sh user pegout -v $VALUE -k $user_compressed_pubkey --env $SCRIPT_ENV"
    log "Amount: $VALUE sats"
    log "USR Pub Key: $user_compressed_pubkey"
    echo

    EXPECTED_REQUEST_PEGOUT_TX_HASH=$(run_user_pegout_and_capture_request_tx_hash "$VALUE" "$user_compressed_pubkey") || return 1
    E2E_REQUEST_PEGOUT_TX_HASH="$EXPECTED_REQUEST_PEGOUT_TX_HASH"
    success "Operator-take pegout requested"

    E2E_ACCEPT_PEGIN_TXID=$(accept_pegin_txid_from_request_tx_hash "$EXPECTED_REQUEST_PEGOUT_TX_HASH")
    if [[ -z "$E2E_ACCEPT_PEGIN_TXID" || ! "$E2E_ACCEPT_PEGIN_TXID" =~ ^0x[0-9a-fA-F]{64}$ ]]; then
        fail_e2e "failed to derive accept pegin txid from request receipt ${EXPECTED_REQUEST_PEGOUT_TX_HASH}"
        return 1
    fi

    if [[ -n "$E2E_PEGOUT_TXID_OVERRIDE" ]]; then
        E2E_PEGOUT_TXID="$E2E_PEGOUT_TXID_OVERRIDE"
        log "Using pegout txid override: ${E2E_PEGOUT_TXID}"
    else
        E2E_PEGOUT_TXID=$(pegout_txid_from_request_tx_hash "$EXPECTED_REQUEST_PEGOUT_TX_HASH")
        if [[ -z "$E2E_PEGOUT_TXID" || ! "$E2E_PEGOUT_TXID" =~ ^0x[0-9a-fA-F]{64}$ ]]; then
            fail_e2e "failed to derive pegout txid from request receipt ${EXPECTED_REQUEST_PEGOUT_TX_HASH}"
            return 1
        fi
    fi

    wait_for_operator_take_candidate "$E2E_PEGOUT_TXID" || return 1
    advance_user_take_timeout
    trigger_operator_take_first "$E2E_PEGOUT_TXID" || return 1
    local observed_pegout_txid
    observed_pegout_txid=$(wait_for_first_operator_take_triggered "$operator_take_from_block") || return 1
    if [[ "${observed_pegout_txid,,}" != "${E2E_PEGOUT_TXID,,}" ]]; then
        fail_e2e "first OperatorTakeTriggered txid ${observed_pegout_txid} did not match expected ${E2E_PEGOUT_TXID}"
        return 1
    fi
    success "First OperatorTakeTriggered observed for ${E2E_PEGOUT_TXID}"

    advance_operator_take_timeout
    trigger_operator_take_again "$E2E_PEGOUT_TXID" || return 1
    wait_for_second_operator_take_triggered "$operator_take_from_block" "$E2E_PEGOUT_TXID" || return 1
    assert_retrigger_log_observed || return 1

    if ! wait_for_correlated_completion_markers "advance-funds" "$NUM_OPERATORS" "$OPERATOR_TAKE_MAX_BLOCKS"; then
        fail_e2e "operator-take completion markers not found in all operators within timeout"
        return 1
    fi
    assert_selected_completion_marker || return 1

    if ! verify_user_btc_balance_increase_for_value "$user_btc_balance_before_sat"; then
        return 1
    fi
    if ! verify_user_rsk_balance_decrease_for_value "$user_rsk_balance_before_wei"; then
        return 1
    fi

    restore_force_advance
    assert_no_bad_log_patterns || return 1
}

main_e2e() {
    parse_e2e_args "$@"
    setup_report_files
    exec > >(tee -a "$E2E_RUN_LOG") 2>&1
    trap finalize EXIT

    initialize_test_flows_context

    echo "All prerequisites met!"
    echo
    log_startup_configuration
    log "PegoutManager: ${E2E_PEGOUT_MANAGER_ADDRESS}"
    log "Report: ${E2E_REPORT_PATH}"
    echo

    SCRIPT_START_TIME=$(date +%s)
    run_setup_and_committee_phases || return 1
    run_pegin_phase || return 1

    if [[ "$E2E_SMOKE_ONLY" == true ]]; then
        run_operator_take_phase || return 1
        assert_selected_completion_marker || return 1
        assert_no_bad_log_patterns || return 1
    else
        run_operator_take_retrigger_phase || return 1
    fi

    E2E_STATUS="PASS"
    E2E_FAILURE_REASON=""
}

main_e2e "$@"
