#!/usr/bin/env bash

# happy path e2e test - fully automated local flow (no manual intervention)
#
# prerequisites:
#   - union bridge clients running (via: cargo run -- run)
#   - anvil running on localhost:8545
#   - bitcoin regtest node running with RPC enabled
#   - USER_BITCOIN_WIF environment variable set (for bitcoin-wallet operations)
#   - MEMBER_BITCOIN_WIF environment variable set (for member operations)
#
# usage: bash tests/run-flows.sh [--env <local|docker>] [--ops <1-10>] [--stream <0-4>] [--happy|--setup|--committee|--pegin|--pegout|--operator-take]

set -euo pipefail

MODE="happy"
SCRIPT_ENV=""
OPS_FROM_FLAG=""
STREAM_ID=0
NUM_OPERATORS=""
STREAM_DENOMINATION=""
VALUE=""
USER_UTXO_VALUE=""
MEMBER_UTXO_VALUE=""
RSK_ADDRESS=""
COMMITTEE_REGISTRY_ADDRESS="0x0DCd1Bf9A1b36cE34237eEaFef220932846BCD82"
FORCE_ADVANCE_FILE="/tmp/FORCE_ADVANCE"
FORCE_ADVANCE_TARGET_OPERATOR_ID=""
USER_API_HOST="localhost"
BASE_USER_API_PORT=40001
INTERRUPTED_SIGNAL=""
USER_BALANCE_FEE_MARGIN_SATS=10000
COMPLETION_MARKER_DIR_NAME="union-bridge-flow-completion-markers"
COMPLETION_MARKER_DOCKER_DIR="/app/db/coordinator/${COMPLETION_MARKER_DIR_NAME}"

# Committee setup can take longer to emit completion markers in all operators.
# Committee setup can take longer in mixed (local client + docker BitVMX) and full-docker flows.
COMMITTEE_SETUP_MAX_BLOCKS=120
OPERATOR_TAKE_MAX_BLOCKS=900

# colors
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
PURPLE='\033[0;35m'
NC='\033[0m'

log() { echo -e "${BLUE}[INFO]${NC} $1"; }
success() { echo -e "${GREEN}[✓]${NC} $1"; }
warn() { echo -e "${YELLOW}[!]${NC} $1"; }
step() {
    echo ""
    echo -e "${GREEN}========== $1 ==========${NC}"
    echo ""
}

usage() {
    local script_name
    script_name=$(basename "${BASH_SOURCE[0]}")
    cat <<EOF
Usage: ${script_name} [--env <local|docker>] [--ops <1-10>] [--stream <0-4>] [--happy|--setup|--committee|--pegin|--pegout|--operator-take]

Modes:
  default (no mode flags)
             happy: run --setup, --committee, --pegin, and --pegout in sequence.
  --happy    Full happy path: runs --setup and --committee first, then pegin and pegout.
  --setup    Prep-only: member wallet prep, operator funding, and whitelist.
  --committee
             Committee-only: apply operators to the stream and wait for setup completion.
  --pegin    Pegin-only: prepares the user wallet and requests pegin.
  --pegout   Pegout-only: requests pegout.
  --operator-take
             Operator-take-only: requests pegout with FORCE_ADVANCE enabled.

  --env      Environment: local or docker (default: from UC_ENV or local)
  --ops      Number of operators (1-10, default: 4 for local, docker-deploy.env or 4 for docker)
  --stream   Stream identifier (0-4). Defaults to 0.
  --help     Show this help text.

Guides:
  1. Start infra and operators outside this script first.
  2. Most convenient: run with no mode flags for the full happy path.
  3. Use --setup and --committee only when you want to split preparation from user flows.
  4. Run --pegin, --pegout, or --operator-take against that prepared state.

Examples:
  bash tests/run-flows.sh
  bash tests/run-flows.sh --env docker --setup
  bash tests/run-flows.sh --env docker --committee
  bash tests/run-flows.sh --pegin
  bash tests/run-flows.sh --operator-take

Notes:
  - This script does not start cli-infra.sh, cli-run.sh, or docker/operator/start-operators.sh.
  - --happy is the only mode that runs both --setup and --committee automatically.
  - --setup does not create a committee; use --committee for apply-stream + committee completion.
  - --pegin, --pegout, and --operator-take assume setup/committee state already exists.
  - --pegout and --operator-take also assume there is already a matching pegin in place for the user and amount being tested.
EOF
}

# ===== Script configuration and argument parsing =====

load_envrc_if_needed() {
    if [[ -n "${DIRENV_DIR:-}" ]]; then
        return 0
    fi

    local repo_root
    repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)

    if [[ -f "${repo_root}/.envrc" ]]; then
        # shellcheck source=/dev/null
        source "${repo_root}/.envrc"
    fi
}

initialize_script_env_default() {
    if [[ -n "${UC_ENV:-}" ]]; then
        if [[ "$UC_ENV" == "local" || "$UC_ENV" == "docker" ]]; then
            SCRIPT_ENV="$UC_ENV"
        else
            SCRIPT_ENV="local"
        fi
    else
        SCRIPT_ENV="local"
    fi
}

parse_args() {
    local happy_only=false
    local setup_only=false
    local committee_only=false
    local pegin_only=false
    local pegout_only=false
    local operator_take_only=false
    local mode_flag_provided=false

    while [[ $# -gt 0 ]]; do
        case "$1" in
        --env)
            SCRIPT_ENV="${2:-}"
            if [[ -z "$SCRIPT_ENV" ]]; then
                usage
                return 1
            fi
            if [[ "$SCRIPT_ENV" != "local" && "$SCRIPT_ENV" != "docker" ]]; then
                echo "Error: --env must be 'local' or 'docker'" >&2
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
        --help|-h)
            usage
            exit 0
            ;;
        --happy)
            happy_only=true
            mode_flag_provided=true
            shift
            ;;
        --setup)
            setup_only=true
            mode_flag_provided=true
            shift
            ;;
        --committee)
            committee_only=true
            mode_flag_provided=true
            shift
            ;;
        --pegin)
            pegin_only=true
            mode_flag_provided=true
            shift
            ;;
        --pegout)
            pegout_only=true
            mode_flag_provided=true
            shift
            ;;
        --operator-take)
            operator_take_only=true
            mode_flag_provided=true
            shift
            ;;
        *)
            usage
            return 1
            ;;
        esac
    done

    local selected_modes=0
    [[ "$happy_only" == true ]] && selected_modes=$((selected_modes + 1))
    [[ "$setup_only" == true ]] && selected_modes=$((selected_modes + 1))
    [[ "$committee_only" == true ]] && selected_modes=$((selected_modes + 1))
    [[ "$pegin_only" == true ]] && selected_modes=$((selected_modes + 1))
    [[ "$pegout_only" == true ]] && selected_modes=$((selected_modes + 1))
    [[ "$operator_take_only" == true ]] && selected_modes=$((selected_modes + 1))

    if [[ "$selected_modes" -gt 1 ]]; then
        echo "Error: --happy, --setup, --committee, --pegin, --pegout, and --operator-take are mutually exclusive" >&2
        usage
        return 1
    fi

    if [[ "$happy_only" == true ]]; then
        MODE="happy"
    elif [[ "$setup_only" == true ]]; then
        MODE="setup"
    elif [[ "$committee_only" == true ]]; then
        MODE="committee"
    elif [[ "$pegin_only" == true ]]; then
        MODE="pegin"
    elif [[ "$pegout_only" == true ]]; then
        MODE="pegout"
    elif [[ "$operator_take_only" == true ]]; then
        MODE="operator-take"
    else
        MODE="happy"
        log "No mode flag provided; defaulting to --happy."
    fi
}

validate_script_env() {
    if [[ "$SCRIPT_ENV" != "local" && "$SCRIPT_ENV" != "docker" ]]; then
        echo "Error: SCRIPT_ENV must be 'local' or 'docker'" >&2
        return 1
    fi
}

load_num_operators() {
    if [[ -n "$OPS_FROM_FLAG" ]]; then
        NUM_OPERATORS="$OPS_FROM_FLAG"
        return 0
    fi

    NUM_OPERATORS=4
    if [[ "$SCRIPT_ENV" != "docker" ]]; then
        return 0
    fi

    local env_file="docker/operator/docker-deploy.env"
    if [[ -f "$env_file" ]]; then
        local configured_ops=""
        configured_ops=$(grep -E '^\s*NUM_OPERATORS=' "$env_file" | tail -1 | cut -d= -f2 | tr -d ' "'\''')
        if [[ -n "$configured_ops" ]] && [[ "$configured_ops" =~ ^[0-9]+$ ]] && [[ "$configured_ops" -ge 1 ]] && [[ "$configured_ops" -le 10 ]]; then
            NUM_OPERATORS="$configured_ops"
        fi
    fi
}

stream_denomination() {
    case "$1" in
        0) echo 100000 ;;
        1) echo 1000000 ;;
        2) echo 10000000 ;;
        3) echo 100000000 ;;
        4) echo 1000000000 ;;
        *)
            echo "Error: unsupported stream id '$1'" >&2
            return 1
            ;;
    esac
}

derived_wallet_utxo_value() {
    local denomination="$1"
    local pegin_enabler_amount=1080
    local wallet_fee_buffer=10000

    echo $((denomination + pegin_enabler_amount + wallet_fee_buffer))
}

derived_member_wallet_utxo_value() {
    local stream_id="$1"
    local operator_count="$2"
    local wallet_fee_buffer=10000
    local per_operator_funding
    per_operator_funding=$(bash cli-operations.sh operator funding-amount --env local --stream "$stream_id")

    echo $((per_operator_funding * operator_count + wallet_fee_buffer))
}

initialize_mode_config() {
    STREAM_DENOMINATION=$(stream_denomination "$STREAM_ID")
    VALUE="$STREAM_DENOMINATION"

    if [[ "$MODE" == "happy" || "$MODE" == "pegin" ]]; then
        USER_UTXO_VALUE=$(derived_wallet_utxo_value "$STREAM_DENOMINATION")
    fi

    if [[ "$MODE" == "happy" || "$MODE" == "setup" ]]; then
        MEMBER_UTXO_VALUE=$(derived_member_wallet_utxo_value "$STREAM_ID" "$NUM_OPERATORS")
    fi
}

# ===== Shared utilities and prerequisite checks =====

format_duration() {
    local total_seconds=$1
    local hours=$((total_seconds / 3600))
    local minutes=$(((total_seconds % 3600) / 60))
    local seconds=$((total_seconds % 60))

    if [ "$hours" -gt 0 ]; then
        printf "%02dh:%02dm:%02ds" "$hours" "$minutes" "$seconds"
    else
        printf "%02dm:%02ds" "$minutes" "$seconds"
    fi
}

notify_terminal_bell() {
    printf '\a'
}

notify_os_notification() {
    local title="$1"
    local message="$2"

    if [[ "$(uname -s)" != "Darwin" ]] || ! command -v osascript &> /dev/null; then
        return 1
    fi

    osascript - "$title" "$message" <<'APPLESCRIPT' &> /dev/null
on run argv
    set notificationTitle to item 1 of argv
    set notificationMessage to item 2 of argv
    display notification notificationMessage with title notificationTitle
end run
APPLESCRIPT
}

notify_completion() {
    local title="$1"
    local message="$2"

    notify_os_notification "$title" "$message" || notify_terminal_bell
}

log_startup_configuration() {
    case "$MODE" in
    happy)
        log "Configuration: mode=$MODE, stream=$STREAM_ID, denomination=$STREAM_DENOMINATION, rsk=$RSK_ADDRESS, amount=$VALUE, env=$SCRIPT_ENV, user_wallet_utxo=$USER_UTXO_VALUE, member_wallet_utxo=$MEMBER_UTXO_VALUE"
        ;;
    setup)
        log "Configuration: mode=$MODE, stream=$STREAM_ID, denomination=$STREAM_DENOMINATION, rsk=$RSK_ADDRESS, amount=$VALUE, env=$SCRIPT_ENV, member_wallet_utxo=$MEMBER_UTXO_VALUE"
        ;;
    pegin)
        log "Configuration: mode=$MODE, stream=$STREAM_ID, denomination=$STREAM_DENOMINATION, rsk=$RSK_ADDRESS, amount=$VALUE, env=$SCRIPT_ENV, user_wallet_utxo=$USER_UTXO_VALUE"
        ;;
    pegout)
        log "Configuration: mode=$MODE, stream=$STREAM_ID, denomination=$STREAM_DENOMINATION, amount=$VALUE, env=$SCRIPT_ENV"
        ;;
    operator-take)
        log "Configuration: mode=$MODE, stream=$STREAM_ID, denomination=$STREAM_DENOMINATION, amount=$VALUE, env=$SCRIPT_ENV"
        ;;
    esac
    log "Prerequisite: keep mining disabled until infra is ready; only start it when the flow needs block progress"
    echo ""
}

# get current bitcoin block height
get_current_bitcoin_height() {
    local height
    height=$(bitcoin-cli -regtest -rpcuser=foo -rpcpassword=rpcpassword getblockcount 2>/dev/null || echo "0")
    height=${height:-0}
    echo "$height"
}

# check if bitcoin node is accessible
check_bitcoin_connectivity() {
    bitcoin-cli -regtest -rpcuser=foo -rpcpassword=rpcpassword getblockcount &> /dev/null
}

check_bitcoin_wallet() {
    bitcoin-cli -regtest -rpcuser=foo -rpcpassword=rpcpassword -rpcwallet=mainwallet getwalletinfo &> /dev/null
}

wait_for_condition() {
    local description="$1"
    local timeout_secs="$2"
    shift 2
    local elapsed=0

    log "Waiting for ${description}..."
    while [[ "${elapsed}" -lt "${timeout_secs}" ]]; do
        if "$@" &> /dev/null; then
            success "${description} ready"
            return 0
        fi
        sleep 1
        elapsed=$((elapsed + 1))
    done

    warn "${description} not ready after ${timeout_secs}s"
    return 1
}

docker_bitvmx_container_healthy() {
    local container_name="$1"
    local status

    if ! status=$(docker inspect --format '{{if .State.Health}}{{.State.Health.Status}}{{else}}{{.State.Status}}{{end}}' "${container_name}" 2>/dev/null); then
        return 1
    fi

    [[ "${status}" == "healthy" ]]
}

wait_for_docker_bitvmx_health() {
    local op_id

    for op_id in $(seq 1 "$NUM_OPERATORS"); do
        if ! wait_for_condition "bitvmx-client-${op_id} health" 90 docker_bitvmx_container_healthy "bitvmx-client-${op_id}"; then
            return 1
        fi
    done
}

all_user_api_endpoints_ready() {
    local op_id

    for op_id in $(seq 1 "$NUM_OPERATORS"); do
        local endpoint=""
        endpoint=$(user_api_endpoint_for_operator "$op_id")
        if ! fetch_member_rsk_address_from_user_api "$endpoint" > /dev/null; then
            return 1
        fi
    done
}

report_unready_user_api_endpoints() {
    local missing=()
    local op_id

    for op_id in $(seq 1 "$NUM_OPERATORS"); do
        local endpoint=""
        endpoint=$(user_api_endpoint_for_operator "$op_id")
        if ! fetch_member_rsk_address_from_user_api "$endpoint" > /dev/null; then
            missing+=("${op_id} (${endpoint})")
        fi
    done

    if [[ "${#missing[@]}" -eq 0 ]]; then
        return 0
    fi

    printf 'Error: user-api not ready for operator(s): %s\n' "${missing[*]}" >&2
    return 1
}

wait_for_test_prereqs() {
    if ! wait_for_condition "Anvil RPC" 30 cast rpc eth_chainId --rpc-url http://localhost:8545; then
        echo "Error: Anvil not ready on localhost:8545" >&2
        return 1
    fi

    if ! wait_for_condition "Bitcoin RPC" 30 check_bitcoin_connectivity; then
        echo "Error: Bitcoin regtest node not accessible" >&2
        echo "Please ensure Bitcoin Core is running with:" >&2
        echo "  bitcoind -regtest -rpcuser=foo -rpcpassword=rpcpassword" >&2
        return 1
    fi

    if ! wait_for_condition "Bitcoin wallet 'mainwallet'" 30 check_bitcoin_wallet; then
        echo "Error: Bitcoin wallet 'mainwallet' is not loaded" >&2
        return 1
    fi

    if ! wait_for_condition "all ${NUM_OPERATORS} user-api endpoints" 60 all_user_api_endpoints_ready; then
        if ! report_unready_user_api_endpoints; then
            :
        fi
        return 1
    fi

    if [[ "$SCRIPT_ENV" == "docker" ]]; then
        if ! wait_for_docker_bitvmx_health; then
            echo "Error: Docker BitVMX clients are not healthy" >&2
            return 1
        fi
    fi
}

# derive x-only public key (32 bytes) from USER_BITCOIN_WIF
# returns the key with 0x prefix (66 chars total)
user_xonly_pubkey_from_wif() {
    if [[ -z "${USER_BITCOIN_WIF:-}" ]]; then
        echo "Error: USER_BITCOIN_WIF not set" >&2
        return 1
    fi
    local desc pubkey xonly
    desc=$(bitcoin-cli -regtest -rpcuser=foo -rpcpassword=rpcpassword getdescriptorinfo "wpkh(${USER_BITCOIN_WIF})" 2>/dev/null | jq -r '.descriptor')
    if [[ -z "$desc" || "$desc" == "null" ]]; then
        echo "Error: Failed to derive descriptor from USER_BITCOIN_WIF" >&2
        return 1
    fi
    # Extract pubkey from wpkh(PUBKEY)#checksum format
    pubkey=$(echo "$desc" | sed -E 's/^wpkh\(([0-9a-fA-F]+)\)#.*/\1/')
    if [[ ${#pubkey} -ne 66 ]]; then
        echo "Error: Unexpected pubkey length: ${#pubkey}" >&2
        return 1
    fi
    # Remove first 2 chars (02/03 prefix) to get x-only key
    xonly="${pubkey:2}"
    echo "0x${xonly}"
}

# derive compressed public key (33 bytes) from USER_BITCOIN_WIF
# returns the key with 0x prefix (68 chars total)
user_compressed_pubkey_from_wif() {
    if [[ -z "${USER_BITCOIN_WIF:-}" ]]; then
        echo "Error: USER_BITCOIN_WIF not set" >&2
        return 1
    fi
    local desc pubkey
    desc=$(bitcoin-cli -regtest -rpcuser=foo -rpcpassword=rpcpassword getdescriptorinfo "wpkh(${USER_BITCOIN_WIF})" 2>/dev/null | jq -r '.descriptor')
    if [[ -z "$desc" || "$desc" == "null" ]]; then
        echo "Error: Failed to derive descriptor from USER_BITCOIN_WIF" >&2
        return 1
    fi
    # Extract pubkey from wpkh(PUBKEY)#checksum format
    pubkey=$(echo "$desc" | sed -E 's/^wpkh\(([0-9a-fA-F]+)\)#.*/\1/')
    if [[ ${#pubkey} -ne 66 ]]; then
        echo "Error: Unexpected pubkey length: ${#pubkey}" >&2
        return 1
    fi
    echo "0x${pubkey}"
}

check_required_commands() {
    if ! command -v cargo &> /dev/null || ! command -v cast &> /dev/null || ! command -v bitcoin-cli &> /dev/null || ! command -v curl &> /dev/null || ! command -v jq &> /dev/null; then
        echo "Error: cargo, cast, bitcoin-cli, curl, and jq required" >&2
        return 1
    fi

    if [[ "$SCRIPT_ENV" == "docker" ]] && ! command -v docker &> /dev/null; then
        echo "Error: docker is required for --env docker" >&2
        return 1
    fi
}

# ===== Completion marker helpers =====

local_coordinator_store_path_for_operator() {
    local op_id="$1"
    echo "${BASE_STORAGE_PATH:-.}/.union_bridge/op_${op_id}/local_database/coordinator"
}

local_completion_marker_dir_for_operator() {
    local op_id="$1"
    echo "$(local_coordinator_store_path_for_operator "$op_id")/${COMPLETION_MARKER_DIR_NAME}"
}

collect_completion_marker_refs_local() {
    local kind="$1"
    local op_id

    for op_id in $(seq 1 "$NUM_OPERATORS"); do
        local marker_dir=""
        marker_dir=$(local_completion_marker_dir_for_operator "$op_id")
        [[ -d "$marker_dir" ]] || continue
        find "$marker_dir" -maxdepth 1 -type f -name "${kind}-*.json" | sort | while IFS= read -r marker_path; do
            [[ -n "$marker_path" ]] || continue
            echo "local|${op_id}|${marker_path}"
        done
    done
}

collect_completion_marker_refs_docker() {
    local kind="$1"
    local op_id

    for op_id in $(seq 1 "$NUM_OPERATORS"); do
        local container_id=""
        container_id=$(docker_coordinator_container_id "$op_id")
        [[ -n "$container_id" ]] || continue
        if ! docker exec "$container_id" test -d "$COMPLETION_MARKER_DOCKER_DIR" 2>/dev/null; then
            continue
        fi

        docker exec "$container_id" sh -lc \
            "find '$COMPLETION_MARKER_DOCKER_DIR' -maxdepth 1 -type f -name '${kind}-*.json' | sort" \
            2>/dev/null | while IFS= read -r marker_path; do
                [[ -n "$marker_path" ]] || continue
                echo "docker|${op_id}|${container_id}|${marker_path}"
            done
    done
}

collect_completion_marker_refs() {
    local kind="$1"
    if [[ "$SCRIPT_ENV" == "docker" ]]; then
        collect_completion_marker_refs_docker "$kind"
        return 0
    fi

    collect_completion_marker_refs_local "$kind"
}

completion_marker_ref_basename() {
    local ref="$1"
    IFS='|' read -r scope _arg1 _arg2 _arg3 <<< "$ref"
    if [[ "$scope" == "docker" ]]; then
        basename "$_arg3"
    else
        basename "$_arg2"
    fi
}

completion_marker_payload_json() {
    local ref="$1"
    IFS='|' read -r scope arg1 arg2 arg3 <<< "$ref"
    if [[ "$scope" == "docker" ]]; then
        docker exec "$arg2" cat "$arg3"
    else
        cat "$arg2"
    fi
}

completion_marker_matches_kind() {
    local kind="$1"
    local ref="$2"

    case "$kind" in
        setup)
            completion_marker_payload_json "$ref" | jq -e \
                --argjson stream_id "$EXPECTED_SETUP_STREAM_ID" \
                --argjson started_at_epoch "$EXPECTED_SETUP_STARTED_AT_EPOCH" \
                '.payload.stream_id == $stream_id and (.completed_at | fromdateiso8601) >= $started_at_epoch' \
                > /dev/null
            ;;
        pegin)
            completion_marker_payload_json "$ref" | jq -e --arg pegin_txid "$EXPECTED_PEGIN_TXID" \
                '.payload.request_pegin_btc_tx_id == $pegin_txid' > /dev/null
            ;;
        pegout|advance-funds)
            completion_marker_payload_json "$ref" | jq -e --arg request_tx_hash "$EXPECTED_REQUEST_PEGOUT_TX_HASH" \
                '.payload.request_pegout_tx_hash == $request_tx_hash' > /dev/null
            ;;
        *)
            echo "Error: unsupported marker kind '$kind'" >&2
            return 1
            ;;
    esac
}

collect_matching_completion_marker_refs() {
    local kind="$1"
    local output_file="$2"
    : > "$output_file"

    while IFS= read -r ref; do
        [[ -n "$ref" ]] || continue
        if completion_marker_matches_kind "$kind" "$ref"; then
            echo "$ref" >> "$output_file"
        fi
    done < <(collect_completion_marker_refs "$kind")
}

first_matching_completion_marker_ref() {
    local kind="$1"
    local refs_file
    refs_file=$(mktemp)

    collect_matching_completion_marker_refs "$kind" "$refs_file"

    local first_ref=""
    first_ref=$(head -n 1 "$refs_file")
    rm -f "$refs_file"

    if [[ -z "$first_ref" ]]; then
        return 1
    fi

    printf '%s\n' "$first_ref"
}

count_completion_marker_refs_in_file() {
    local refs_file="$1"
    if [[ ! -f "$refs_file" ]]; then
        echo 0
        return 0
    fi
    wc -l < "$refs_file" | tr -d ' '
}

print_completion_marker_refs() {
    local refs_file="$1"
    while IFS= read -r ref; do
        [[ -n "$ref" ]] || continue
        IFS='|' read -r scope op_id arg2 _arg3 <<< "$ref"
        if [[ "$scope" == "docker" ]]; then
            log "Completion marker (operator ${op_id}, container ${arg2}): $(completion_marker_ref_basename "$ref")"
        else
            log "Completion marker (operator ${op_id}): $(completion_marker_ref_basename "$ref")"
        fi
    done < "$refs_file"
}

wait_for_correlated_completion_markers() {
    local kind="$1"
    local expected_count="$2"
    local max_blocks="$3"

    local start_height
    start_height=$(get_current_bitcoin_height)
    local target_height=$((start_height + max_blocks))
    local matches_file
    matches_file=$(mktemp)

    log "Waiting for ${expected_count} correlated ${kind} completion marker(s) (max $max_blocks blocks)..."

    while true; do
        local current_height
        current_height=$(get_current_bitcoin_height)
        local blocks_mined=$((current_height - start_height))

        if [ "$blocks_mined" -lt 0 ]; then
            sleep 1
            continue
        fi

        collect_matching_completion_marker_refs "$kind" "$matches_file"
        local current_count=0
        current_count=$(count_completion_marker_refs_in_file "$matches_file")

        echo -ne "\r  Blocks mined: $blocks_mined/$max_blocks | Operators completed: $current_count/$expected_count  "

        if [ "$current_count" -ge "$expected_count" ]; then
            echo ""
            success "${kind} correlated completion marker detected"
            print_completion_marker_refs "$matches_file"
            rm -f "$matches_file"
            return 0
        fi

        if [ "$current_height" -ge "$target_height" ]; then
            echo ""
            warn "${kind} correlated completion marker not found after $max_blocks blocks (height: $start_height -> $current_height)"
            rm -f "$matches_file"
            return 1
        fi

        sleep 1
    done
}

# ===== User wallet and balance helpers =====

user_rsk_balance_wei() {
    local address="$1"
    cast balance "$address" --rpc-url http://localhost:8545
}

user_active_bitcoin_address() {
    bash cli-bitcoin-wallet.sh user list_addresses 2>/dev/null | awk '/\(active\)/ { print $1; exit }'
}

user_btc_balance_sat() {
    local address=""
    address=$(user_active_bitcoin_address)
    if [[ -z "$address" ]]; then
        echo "Error: failed to resolve active user Bitcoin address" >&2
        return 1
    fi

    bitcoin-cli -regtest -rpcuser=foo -rpcpassword=rpcpassword \
        scantxoutset start "[\"addr(${address})\"]" \
        | jq -r '(.total_amount * 100000000 | round) // 0'
}

format_btc_amount_from_sat() {
    local sat="$1"
    awk -v sat="$sat" 'BEGIN { printf "%.8f BTC", sat / 100000000 }'
}

format_rbtc_amount_from_wei() {
    local wei="$1"
    awk -v wei="$wei" 'BEGIN { printf "%.18f RBTC", wei / 1000000000000000000 }'
}

extract_pegin_txid_from_output() {
    local output="$1"
    printf '%s\n' "$output" | awk -F= '/txid=/{print $2}' | tail -1 | tr -d '[:space:]'
}

extract_pegout_request_tx_hash_from_output() {
    local output="$1"
    printf '%s\n' "$output" | sed -nE 's/.*Transaction hash:[[:space:]]*(0x[0-9a-fA-F]{64}).*/\1/p' | tail -1
}

run_user_pegin_and_capture_txid() {
    local rsk_address="$1"
    local value="$2"
    local user_xonly_pubkey="$3"
    local output=""

    if ! output=$(bash cli-operations.sh user pegin -a "$rsk_address" -v "$value" -k "$user_xonly_pubkey" --env "$SCRIPT_ENV" --execute 2>&1); then
        echo -e "${YELLOW}[!]${NC} Command failed!" >&2
        printf '%s\n' "$output" >&2
        return 1
    fi

    local pegin_txid=""
    pegin_txid=$(extract_pegin_txid_from_output "$output")

    if [[ -z "$pegin_txid" ]]; then
        echo -e "${YELLOW}[!]${NC} Failed to extract pegin txid from command output" >&2
        return 1
    fi

    printf '%s\n' "$pegin_txid"
}

run_user_pegout_and_capture_request_tx_hash() {
    local value="$1"
    local user_compressed_pubkey="$2"
    local output=""

    if ! output=$(bash cli-operations.sh user pegout -v "$value" -k "$user_compressed_pubkey" --env "$SCRIPT_ENV" 2>&1); then
        echo -e "${YELLOW}[!]${NC} Command failed! Output:" >&2
        printf '%s\n' "$output" >&2
        return 1
    fi

    local request_tx_hash=""
    request_tx_hash=$(extract_pegout_request_tx_hash_from_output "$output")

    if [[ -z "$request_tx_hash" ]]; then
        echo -e "${YELLOW}[!]${NC} Failed to extract pegout request tx hash from command output" >&2
        return 1
    fi

    printf '%s\n' "$request_tx_hash"
}

assert_user_rsk_balance_increase() {
    local before_wei="$1"
    local after_wei="$2"
    local min_increase_wei="$3"

    local actual_increase_wei=$((after_wei - before_wei))
    if (( actual_increase_wei < min_increase_wei )); then
        warn "User RBTC balance increase too small: got $(format_rbtc_amount_from_wei "$actual_increase_wei"), expected at least $(format_rbtc_amount_from_wei "$min_increase_wei")"
        return 1
    fi

    success "User RBTC balance increased by $(format_rbtc_amount_from_wei "$actual_increase_wei")"
}

assert_user_btc_balance_increase() {
    local before_sat="$1"
    local after_sat="$2"
    local min_increase_sat="$3"

    local actual_increase_sat=$((after_sat - before_sat))
    if (( actual_increase_sat < min_increase_sat )); then
        warn "User BTC balance increase too small: got $(format_btc_amount_from_sat "$actual_increase_sat"), expected at least $(format_btc_amount_from_sat "$min_increase_sat")"
        return 1
    fi

    success "User BTC balance increased by $(format_btc_amount_from_sat "$actual_increase_sat")"
}

assert_user_rsk_balance_decrease() {
    local before_wei="$1"
    local after_wei="$2"
    local min_decrease_wei="$3"

    local actual_decrease_wei=$((before_wei - after_wei))
    if (( actual_decrease_wei < min_decrease_wei )); then
        warn "User RBTC balance decrease too small: got $(format_rbtc_amount_from_wei "$actual_decrease_wei"), expected at least $(format_rbtc_amount_from_wei "$min_decrease_wei")"
        return 1
    fi

    success "User RBTC balance decreased by $(format_rbtc_amount_from_wei "$actual_decrease_wei")"
}

assert_user_btc_balance_decrease() {
    local before_sat="$1"
    local after_sat="$2"
    local min_decrease_sat="$3"

    local actual_decrease_sat=$((before_sat - after_sat))
    if (( actual_decrease_sat < min_decrease_sat )); then
        warn "User BTC balance decrease too small: got $(format_btc_amount_from_sat "$actual_decrease_sat"), expected at least $(format_btc_amount_from_sat "$min_decrease_sat")"
        return 1
    fi

    success "User BTC balance decreased by $(format_btc_amount_from_sat "$actual_decrease_sat")"
}

verify_user_balance_change() {
    local asset="$1"
    local direction="$2"
    local before_value="$3"
    local after_value="$4"
    local expected_min_delta="$5"

    case "$asset" in
        rsk)
            case "$direction" in
                increase)
                    assert_user_rsk_balance_increase \
                        "$before_value" \
                        "$after_value" \
                        "$expected_min_delta" || return 1
                    ;;
                decrease)
                    assert_user_rsk_balance_decrease \
                        "$before_value" \
                        "$after_value" \
                        "$expected_min_delta" || return 1
                    ;;
                *)
                    echo "Error: unsupported balance direction '$direction'" >&2
                    return 1
                    ;;
            esac
            ;;
        btc)
            case "$direction" in
                increase)
                    assert_user_btc_balance_increase \
                        "$before_value" \
                        "$after_value" \
                        "$expected_min_delta" || return 1
                    ;;
                decrease)
                    assert_user_btc_balance_decrease \
                        "$before_value" \
                        "$after_value" \
                        "$expected_min_delta" || return 1
                    ;;
                *)
                    echo "Error: unsupported balance direction '$direction'" >&2
                    return 1
                    ;;
            esac
            ;;
        *)
            echo "Error: unsupported asset '$asset'" >&2
            return 1
            ;;
    esac

    echo ""
}

verify_user_btc_balance_for_value() {
    local direction="$1"
    local before_sat="$2"
    local min_delta_sat="$3"
    local after_sat
    after_sat=$(user_btc_balance_sat)

    verify_user_balance_change btc "$direction" "$before_sat" "$after_sat" "$min_delta_sat"
}

verify_user_rsk_balance_for_value() {
    local direction="$1"
    local before_wei="$2"
    local min_delta_wei="$3"
    local after_wei
    after_wei=$(user_rsk_balance_wei "$RSK_ADDRESS")

    verify_user_balance_change rsk "$direction" "$before_wei" "$after_wei" "$min_delta_wei"
}

pegin_expected_rbtc_amount_wei() {
    local ref=""
    ref=$(first_matching_completion_marker_ref "pegin") || {
        echo "Error: failed to find correlated pegin completion marker for rbtc_amount" >&2
        return 1
    }

    local amount=""
    amount=$(completion_marker_payload_json "$ref" | jq -r '.payload.rbtc_amount // empty' 2>/dev/null) || return 1
    [[ -n "$amount" ]] || {
        echo "Error: correlated pegin completion marker is missing payload.rbtc_amount" >&2
        return 1
    }

    printf '%s\n' "$amount"
}

verify_user_btc_balance_increase_for_value() {
    local before_sat="$1"
    local expected_min_increase_sat=$((VALUE - USER_BALANCE_FEE_MARGIN_SATS))
    if (( expected_min_increase_sat < 0 )); then
        expected_min_increase_sat=0
    fi

    verify_user_btc_balance_for_value increase "$before_sat" "$expected_min_increase_sat"
}

verify_user_btc_balance_decrease_for_value() {
    local before_sat="$1"
    verify_user_btc_balance_for_value decrease "$before_sat" "$VALUE"
}

verify_user_rsk_balance_increase_for_value() {
    local before_wei="$1"
    local expected_min_increase_wei=$((VALUE * 10000000000))
    verify_user_rsk_balance_for_value increase "$before_wei" "$expected_min_increase_wei"
}

verify_user_rsk_balance_increase_for_pegin() {
    local before_wei="$1"
    local expected_min_increase_wei=""
    expected_min_increase_wei=$(pegin_expected_rbtc_amount_wei) || return 1

    verify_user_rsk_balance_for_value increase "$before_wei" "$expected_min_increase_wei"
}

verify_user_rsk_balance_decrease_for_value() {
    local before_wei="$1"
    local expected_min_decrease_wei=$((VALUE * 10000000000))
    verify_user_rsk_balance_for_value decrease "$before_wei" "$expected_min_decrease_wei"
}

# count transactions in blocks from start_height to end_height (excluding coinbase)
count_transactions_in_blocks() {
    local start_height=$1
    local end_height=$2
    local total_txs=0
    local h
    for ((h=start_height + 1; h<=end_height; h++)); do
        local stats
        stats=$(bitcoin-cli -regtest -rpcuser=foo -rpcpassword=rpcpassword getblockstats "$h" 2>/dev/null) || continue
        local total_tx_count
        total_tx_count=$(jq -r '.txs // empty' 2>/dev/null <<<"$stats" || echo "$stats" | tr -d '\n' | sed -E 's/.*"txs"[[:space:]]*:[[:space:]]*([0-9]+).*/\1/')
        if [ -n "$total_tx_count" ] && [ "$total_tx_count" -gt 1 ]; then
            local non_coinbase_count=$((total_tx_count - 1))
            total_txs=$((total_txs + non_coinbase_count))
        fi
    done
    echo "$total_txs"
}

# wait for N bitcoin transactions to be created in mined blocks with confirmations
# expected_count: number of transactions to wait for
# max_blocks: maximum blocks to wait for transactions to appear
# confirmations: number of confirmations required (blocks after transaction is mined)
wait_for_bitcoin_transactions() {
    local expected_count=$1
    local max_blocks=$2
    local confirmations=$3
    local start_height
    start_height=$(get_current_bitcoin_height)

    local target_height=$((start_height + max_blocks + confirmations))
    local first_tx_height=0

    log "Waiting for $expected_count Bitcoin transactions in mined blocks (max $max_blocks blocks to find) with $confirmations confirmations..."
    log "Starting height: $start_height, Max target height: $target_height"

    while true; do
        local current_height
        current_height=$(get_current_bitcoin_height)
        local blocks_mined=$((current_height - start_height))

        if [ "$blocks_mined" -lt 0 ]; then
            sleep 1
            continue
        fi

        local transactions_found=0
        if [ "$blocks_mined" -gt 0 ]; then
            transactions_found=$(count_transactions_in_blocks "$start_height" "$current_height")
        fi

        if [ "$transactions_found" -ge "$expected_count" ] && [ "$first_tx_height" -eq 0 ]; then
            first_tx_height=$current_height
            log "Found $expected_count transactions at height $first_tx_height, waiting for $confirmations confirmations..."
        fi

        local current_confirmations=0
        if [ "$first_tx_height" -gt 0 ]; then
            current_confirmations=$((current_height - first_tx_height))
        fi

        if [ "$first_tx_height" -gt 0 ]; then
            echo -ne "\r  Blocks mined: $blocks_mined | Transactions: $transactions_found/$expected_count | Confirmations: $current_confirmations/$confirmations  "
        else
            echo -ne "\r  Blocks mined: $blocks_mined/$max_blocks | Transactions found: $transactions_found/$expected_count  "
        fi

        if [ "$transactions_found" -ge "$expected_count" ] && [ "$first_tx_height" -gt 0 ] && [ "$current_confirmations" -ge "$confirmations" ]; then
            echo ""
            success "$expected_count Bitcoin transactions found with $confirmations confirmations (height: $start_height -> $current_height, tx at $first_tx_height)"
            return 0
        fi

        if [ "$current_height" -ge "$target_height" ] && [ "$first_tx_height" -eq 0 ]; then
            echo ""
            warn "Only $transactions_found/$expected_count Bitcoin transactions found after $max_blocks blocks (height: $start_height -> $current_height)"
            return 1
        fi

        if [ "$first_tx_height" -gt 0 ] && [ "$current_height" -ge "$target_height" ]; then
            echo ""
            warn "Found $expected_count transactions at height $first_tx_height but only got $current_confirmations/$confirmations confirmations (timeout at height $target_height)"
            return 1
        fi

        sleep 0.5
    done
}

# ===== user-api address resolution =====

user_api_endpoint_for_operator() {
    local op_id="$1"
    local port=$((BASE_USER_API_PORT + op_id - 1))
    echo "${USER_API_HOST}:${port}"
}

fetch_member_rsk_address_from_user_api() {
    local endpoint="$1"
    local response=""
    response=$(curl -fsS --max-time 10 "http://${endpoint}/member/funding-info" 2>/dev/null) || return 1

    local address=""
    address=$(printf '%s' "$response" | jq -r '.rsk_address // empty' 2>/dev/null) || return 1
    [[ -n "$address" ]] || return 1

    echo "$address"
}

fetch_user_rsk_address_from_user_api() {
    local endpoint="$1"
    local response=""
    response=$(curl -fsS --max-time 10 "http://${endpoint}/user/rsk-address" 2>/dev/null) || return 1

    local address=""
    address=$(printf '%s' "$response" | jq -r '.address // empty' 2>/dev/null) || return 1
    [[ -n "$address" ]] || return 1

    echo "$address"
}

resolve_user_rsk_address() {
    local op_id
    for op_id in $(seq 1 "$NUM_OPERATORS"); do
        local endpoint=""
        endpoint=$(user_api_endpoint_for_operator "$op_id")
        local address=""
        if address=$(fetch_user_rsk_address_from_user_api "$endpoint"); then
            echo "$address"
            return 0
        fi
    done

    echo "Error: failed to resolve user RSK address from user-api /user/rsk-address" >&2
    return 1
}

resolve_force_advance_target_address() {
    FORCE_ADVANCE_TARGET_OPERATOR_ID=""
    local op_id
    for op_id in $(seq 1 "$NUM_OPERATORS"); do
        local endpoint=""
        endpoint=$(user_api_endpoint_for_operator "$op_id")
        local address=""
        if address=$(fetch_member_rsk_address_from_user_api "$endpoint"); then
            FORCE_ADVANCE_TARGET_OPERATOR_ID="$op_id"
            echo "$address"
            return 0
        fi
    done

    echo "Error: failed to resolve a member signer address from user-api /member/funding-info for FORCE_ADVANCE" >&2
    return 1
}

# ===== FORCE_ADVANCE / operator-take helpers =====

docker_coordinator_container_id() {
    local op_id="$1"
    local project="op_${op_id}"

    docker ps -q \
        --filter "label=com.docker.compose.project=${project}" \
        --filter "label=com.docker.compose.service=coordinator" \
        | head -n 1
}

find_active_force_advance_local() {
    if [[ -s "$FORCE_ADVANCE_FILE" ]]; then
        local address=""
        address=$(tr -d '\r\n' < "$FORCE_ADVANCE_FILE")
        if [[ -n "$address" ]]; then
            echo "host file $FORCE_ADVANCE_FILE ($address)"
            return 0
        fi
    fi

    if [[ -n "${FORCE_ADVANCE:-}" ]]; then
        echo "host environment FORCE_ADVANCE (${FORCE_ADVANCE})"
        return 0
    fi

    return 1
}

find_active_force_advance_in_docker() {
    local op_id

    for op_id in $(seq 1 "$NUM_OPERATORS"); do
        local container_id=""
        container_id=$(docker_coordinator_container_id "$op_id")
        [[ -n "$container_id" ]] || continue

        local state=""
        if ! state=$(docker exec "$container_id" sh -c '
            if [ -s "$1" ]; then
                value=$(tr -d "\r\n" < "$1")
                if [ -n "$value" ]; then
                    printf "file %s (%s)" "$1" "$value"
                    exit 0
                fi
            fi

            if [ -n "${FORCE_ADVANCE:-}" ]; then
                printf "environment FORCE_ADVANCE (%s)" "$FORCE_ADVANCE"
                exit 0
            fi

            exit 1
        ' sh "$FORCE_ADVANCE_FILE" 2>/dev/null); then
            continue
        fi

        if [[ -n "$state" ]]; then
            echo "coordinator op_${op_id} ${state}"
            return 0
        fi
    done

    return 1
}

ensure_force_advance_inactive_unless_requested() {
    if [[ "$MODE" == "operator-take" ]]; then
        return 0
    fi

    local active_state=""
    if [[ "$SCRIPT_ENV" == "docker" ]]; then
        if active_state=$(find_active_force_advance_in_docker); then
            :
        else
            active_state=""
        fi
    else
        if active_state=$(find_active_force_advance_local); then
            :
        else
            active_state=""
        fi
    fi

    if [[ -n "$active_state" ]]; then
        echo "Error: FORCE_ADVANCE is already active via ${active_state}, but mode is '$MODE'. Clear it or rerun with --operator-take." >&2
        return 1
    fi

    return 0
}

write_force_advance_in_docker() {
    local target_address="$1"
    local op_id="${FORCE_ADVANCE_TARGET_OPERATOR_ID:-}"
    if [[ -z "$op_id" ]]; then
        warn "Target operator id not resolved for docker FORCE_ADVANCE"
        return 1
    fi

    local container_id=""
    container_id=$(docker_coordinator_container_id "$op_id")
    if [[ -z "$container_id" ]]; then
        warn "Coordinator container not found for operator $op_id"
        return 1
    fi

    if ! docker exec "$container_id" sh -c 'printf "%s\n" "$1" > "$2"' sh "$target_address" "$FORCE_ADVANCE_FILE"; then
        warn "Failed to write $FORCE_ADVANCE_FILE in coordinator container for operator $op_id"
        return 1
    fi
}

remove_force_advance_in_docker() {
    local op_id

    for op_id in $(seq 1 "$NUM_OPERATORS"); do
        local container_id=""
        container_id=$(docker_coordinator_container_id "$op_id")
        if [[ -z "$container_id" ]]; then
            continue
        fi

        if ! docker exec "$container_id" sh -c 'rm -f "$1"' sh "$FORCE_ADVANCE_FILE" >/dev/null 2>&1; then
            warn "Failed to remove $FORCE_ADVANCE_FILE from coordinator container op_${op_id}"
        fi
    done
}

enable_force_advance() {
    local target_address="$1"

    if [[ "$SCRIPT_ENV" == "docker" ]]; then
        write_force_advance_in_docker "$target_address" || return 1
        log "FORCE_ADVANCE enabled via $FORCE_ADVANCE_FILE in coordinator container op_${FORCE_ADVANCE_TARGET_OPERATOR_ID} for operator $target_address"
        return 0
    fi

    printf '%s\n' "$target_address" > "$FORCE_ADVANCE_FILE"
    log "FORCE_ADVANCE enabled via $FORCE_ADVANCE_FILE for operator $target_address"
}

restore_force_advance() {
    rm -f "$FORCE_ADVANCE_FILE"

    if [[ "$SCRIPT_ENV" == "docker" ]]; then
        remove_force_advance_in_docker
    fi
}

cleanup() {
    restore_force_advance
    rm -f /tmp/apply-operators-$$ /tmp/pegout-$$
}

on_exit() {
    local exit_code=$?
    local final_exit_code="$exit_code"
    cleanup

    if [[ -n "$INTERRUPTED_SIGNAL" ]]; then
        case "$INTERRUPTED_SIGNAL" in
        SIGINT)
            final_exit_code=130
            ;;
        SIGTERM)
            final_exit_code=143
            ;;
        *)
            final_exit_code="$exit_code"
            ;;
        esac
    fi

    if [[ $final_exit_code -eq 0 ]]; then
        notify_completion "Happy Path script" "Completed"
    elif [[ -n "$INTERRUPTED_SIGNAL" ]]; then
        notify_completion "Happy Path script" "Interrupted (${INTERRUPTED_SIGNAL})"
    else
        notify_completion "Happy Path script" "Failed (exit $final_exit_code)"
    fi

    return "$final_exit_code"
}

on_signal() {
    local signal="$1"
    INTERRUPTED_SIGNAL="$signal"
    case "$signal" in
    SIGINT)
        exit 130
        ;;
    SIGTERM)
        exit 143
        ;;
    *)
        exit 1
        ;;
    esac
}

# ===== Setup, committee, and user-flow phases =====

run_wallet_prep_phase() {
    step "Step 0: Prepare Wallets"
    log "Clearing member wallet database and funding member wallet-specific UTXO..."
    log "Note: First run compiles release binaries for bitcoin-wallet (~1 min), subsequent runs are fast"
    echo ""

    log "Member wallet: clear_db"
    bash cli-bitcoin-wallet.sh member clear_db || warn "Member wallet clear_db failed (may be expected if db is empty)"

    log "Member wallet: mine_utxo $MEMBER_UTXO_VALUE"
    if ! bash cli-bitcoin-wallet.sh member mine_utxo "$MEMBER_UTXO_VALUE"; then
        warn "Member wallet mine_utxo failed!"
        return 1
    fi

    success "Member wallet prepared with funded UTXO"
    echo ""
}

prepare_user_wallet_for_pegin() {
    log "User wallet: clear_db"
    bash cli-bitcoin-wallet.sh user clear_db || warn "User wallet clear_db failed (may be expected if db is empty)"

    log "User wallet: mine_utxo $USER_UTXO_VALUE"
    if ! bash cli-bitcoin-wallet.sh user mine_utxo "$USER_UTXO_VALUE"; then
        warn "User wallet mine_utxo failed!"
        return 1
    fi

    success "User wallet prepared for pegin"
    echo ""
}

run_operator_funding_phase() {
    step "Step 1: Fund Operator Wallets"
    log "Command: bash cli-operations.sh operator fund --env $SCRIPT_ENV --stream $STREAM_ID --execute"
    echo ""
    if ! bash cli-operations.sh operator fund --env "$SCRIPT_ENV" --stream "$STREAM_ID" --execute; then
        warn "Command failed!"
        return 1
    fi
    echo ""
    if ! wait_for_bitcoin_transactions 1 15 5; then
        warn "Failed to detect 1 Bitcoin transaction with 5 confirmations within 15 blocks"
        return 1
    fi
    echo ""
    success "Operator wallets funded (including BitVMX)"
}

run_whitelist_phase() {
    step "Step 2: Whitelist Member Addresses"
    log "Command: bash cli-operations.sh operator whitelist --env $SCRIPT_ENV --contract-address $COMMITTEE_REGISTRY_ADDRESS"
    echo ""
    if ! bash cli-operations.sh operator whitelist --env "$SCRIPT_ENV" --contract-address "$COMMITTEE_REGISTRY_ADDRESS"; then
        warn "Whitelist command failed!"
        return 1
    fi
    success "Member addresses whitelisted"
    echo ""
}

run_committee_phase() {
    step "Step 3: Apply Operators to Stream"
    EXPECTED_SETUP_STREAM_ID="$STREAM_ID"
    EXPECTED_SETUP_STARTED_AT_EPOCH="$(date +%s)"
    log "Command: bash cli-operations.sh operator apply-stream -s $STREAM_ID --env $SCRIPT_ENV"
    echo ""
    if ! bash cli-operations.sh operator apply-stream -s "$STREAM_ID" --env "$SCRIPT_ENV" > /tmp/apply-operators-$$ 2>&1; then
        warn "Command failed! Output:"
        cat /tmp/apply-operators-$$
        rm -f /tmp/apply-operators-$$
        return 1
    fi
    rm -f /tmp/apply-operators-$$
    success "Operators applied to stream $STREAM_ID"
    echo ""
    if ! wait_for_correlated_completion_markers "setup" "$NUM_OPERATORS" "$COMMITTEE_SETUP_MAX_BLOCKS"; then
        warn "Committee setup completion markers not found in all operators within timeout"
        return 1
    fi
    echo ""
}

run_setup_phase() {
    run_wallet_prep_phase
    run_operator_funding_phase
    run_whitelist_phase
}

run_setup_and_committee_phases() {
    local setup_start_time
    setup_start_time=$(date +%s)

    run_setup_phase
    run_committee_phase

    local setup_end_time
    setup_end_time=$(date +%s)
    SETUP_DURATION=$((setup_end_time - setup_start_time))
    success "Setup and committee completed in $(format_duration "$SETUP_DURATION")"
    echo ""
}

run_pegin_phase() {
    if [[ "$MODE" == "happy" ]]; then
        step "Step 4: Request Pegin"
    else
        step "Pegin"
    fi
    local pegin_start_time
    pegin_start_time=$(date +%s)

    log "Preparing user wallet for pegin..."
    if ! prepare_user_wallet_for_pegin; then
        return 1
    fi
    RSK_ADDRESS=$(resolve_user_rsk_address) || return 1

    local user_rsk_balance_before_wei
    user_rsk_balance_before_wei=$(user_rsk_balance_wei "$RSK_ADDRESS")
    local user_btc_balance_before_sat
    user_btc_balance_before_sat=$(user_btc_balance_sat)

    local user_xonly_pubkey
    user_xonly_pubkey=$(user_xonly_pubkey_from_wif)
    if [[ -z "$user_xonly_pubkey" ]]; then
        warn "Failed to derive user x-only public key from WIF"
        return 1
    fi

    log "RSK Address: $RSK_ADDRESS"
    log "Amount: $VALUE sats"
    log "BTC Pub Key: $user_xonly_pubkey"
    log "Command: bash cli-operations.sh user pegin -a $RSK_ADDRESS -v $VALUE -k $user_xonly_pubkey --env $SCRIPT_ENV --execute"
    echo ""
    if ! EXPECTED_PEGIN_TXID=$(run_user_pegin_and_capture_txid "$RSK_ADDRESS" "$VALUE" "$user_xonly_pubkey"); then
        return 1
    fi
    success "Pegin transaction created"
    echo ""
    if ! wait_for_correlated_completion_markers "pegin" "$NUM_OPERATORS" 15; then
        warn "Pegin completion marker not found within timeout"
        return 1
    fi
    echo ""
    if ! verify_user_rsk_balance_increase_for_pegin "$user_rsk_balance_before_wei"; then
        return 1
    fi
    if ! verify_user_btc_balance_decrease_for_value "$user_btc_balance_before_sat"; then
        return 1
    fi

    local pegin_end_time
    pegin_end_time=$(date +%s)
    PEGIN_DURATION=$((pegin_end_time - pegin_start_time))
    success "Pegin completed in $(format_duration "$PEGIN_DURATION")"
    echo ""
}

run_pegout_phase() {
    if [[ "$MODE" == "happy" ]]; then
        step "Step 5: Request Pegout"
    else
        step "Pegout"
    fi
    local pegout_start_time
    pegout_start_time=$(date +%s)
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

    log "Command: bash cli-operations.sh user pegout -v $VALUE -k $user_compressed_pubkey --env $SCRIPT_ENV"
    log "Amount: $VALUE sats"
    log "USR Pub Key: $user_compressed_pubkey"
    echo ""

    if ! EXPECTED_REQUEST_PEGOUT_TX_HASH=$(run_user_pegout_and_capture_request_tx_hash "$VALUE" "$user_compressed_pubkey"); then
        return 1
    fi
    success "Pegout requested"
    echo ""

    if ! wait_for_correlated_completion_markers "pegout" "$NUM_OPERATORS" 15; then
        warn "Pegout completion marker not found within timeout"
        return 1
    fi
    echo ""
    if ! verify_user_btc_balance_increase_for_value "$user_btc_balance_before_sat"; then
        return 1
    fi
    if ! verify_user_rsk_balance_decrease_for_value "$user_rsk_balance_before_wei"; then
        return 1
    fi

    local pegout_end_time
    pegout_end_time=$(date +%s)
    PEGOUT_DURATION=$((pegout_end_time - pegout_start_time))
    success "Pegout completed in $(format_duration "$PEGOUT_DURATION")"
    echo ""
}

run_pegout_verification_phase() {
    if [[ "$MODE" != "happy" ]]; then
        return 0
    fi

    step "Step 6: Verify Pegout Completion"
    # Pegout completion is already enforced by run_pegout_phase; keep this
    # separate step for the full happy-path output expected by older usage.
    success "PegoutFlow completion verified"
    echo ""
}

run_operator_take_phase() {
    step "Operator Take"
    local operator_take_start_time
    operator_take_start_time=$(date +%s)
    RSK_ADDRESS=$(resolve_user_rsk_address) || return 1
    local user_btc_balance_before_sat
    user_btc_balance_before_sat=$(user_btc_balance_sat)
    local user_rsk_balance_before_wei
    user_rsk_balance_before_wei=$(user_rsk_balance_wei "$RSK_ADDRESS")

    local target_address=""
    target_address=$(resolve_force_advance_target_address)

    local user_compressed_pubkey
    user_compressed_pubkey=$(user_compressed_pubkey_from_wif)
    if [[ -z "$user_compressed_pubkey" ]]; then
        warn "Failed to derive user compressed public key from WIF"
        return 1
    fi

    enable_force_advance "$target_address" || return 1

    log "Target operator: $target_address"
    log "Command: bash cli-operations.sh user pegout -v $VALUE -k $user_compressed_pubkey --env $SCRIPT_ENV"
    log "Amount: $VALUE sats"
    log "USR Pub Key: $user_compressed_pubkey"
    echo ""

    if ! EXPECTED_REQUEST_PEGOUT_TX_HASH=$(run_user_pegout_and_capture_request_tx_hash "$VALUE" "$user_compressed_pubkey"); then
        return 1
    fi
    success "Operator-take pegout requested"
    echo ""

    if ! wait_for_correlated_completion_markers "advance-funds" "$NUM_OPERATORS" "$OPERATOR_TAKE_MAX_BLOCKS"; then
        warn "Operator-take completion markers not found in all operators within timeout"
        return 1
    fi
    echo ""
    if ! verify_user_btc_balance_increase_for_value "$user_btc_balance_before_sat"; then
        return 1
    fi
    if ! verify_user_rsk_balance_decrease_for_value "$user_rsk_balance_before_wei"; then
        return 1
    fi

    restore_force_advance

    local operator_take_end_time
    operator_take_end_time=$(date +%s)
    OPERATOR_TAKE_DURATION=$((operator_take_end_time - operator_take_start_time))
    success "Operator take completed in $(format_duration "$OPERATOR_TAKE_DURATION")"
    echo ""
}

print_success_summary() {
    local total_duration
    total_duration=$(( $(date +%s) - SCRIPT_START_TIME ))

    step "Complete"

    case "$MODE" in
    happy)
        success "Happy path completed successfully!"
        log "Timing summary:"
        log "  Setup + Committee: $(format_duration "${SETUP_DURATION:-0}")"
        log "  Pegin:  $(format_duration "${PEGIN_DURATION:-0}")"
        log "  Pegout: $(format_duration "${PEGOUT_DURATION:-0}")"
        log "  Total:  $(format_duration "$total_duration")"
        ;;
    setup)
        success "Setup completed successfully!"
        log "Timing summary:"
        log "  Setup: $(format_duration "${SETUP_DURATION:-0}")"
        log "  Total: $(format_duration "$total_duration")"
        ;;
    committee)
        success "Committee completed successfully!"
        log "Timing summary:"
        log "  Committee: $(format_duration "${SETUP_DURATION:-0}")"
        log "  Total:     $(format_duration "$total_duration")"
        ;;
    pegin)
        success "Pegin happy path completed successfully!"
        log "Timing summary:"
        log "  Pegin: $(format_duration "${PEGIN_DURATION:-0}")"
        log "  Total: $(format_duration "$total_duration")"
        ;;
    pegout)
        success "Pegout happy path completed successfully!"
        log "Timing summary:"
        log "  Pegout: $(format_duration "${PEGOUT_DURATION:-0}")"
        log "  Total:  $(format_duration "$total_duration")"
        ;;
    operator-take)
        success "Operator-take happy path completed successfully!"
        log "Timing summary:"
        log "  Operator take: $(format_duration "${OPERATOR_TAKE_DURATION:-0}")"
        log "  Total:         $(format_duration "$total_duration")"
        ;;
    esac
}

main() {
    load_envrc_if_needed
    initialize_script_env_default
    parse_args "$@" || return 1
    validate_script_env || return 1

    cd "$(dirname "${BASH_SOURCE[0]}")/.."

    load_num_operators
    initialize_mode_config
    check_required_commands || return 1
    wait_for_test_prereqs || return 1
    ensure_force_advance_inactive_unless_requested || return 1

    echo "All prerequisites met!"
    echo ""

    trap on_exit EXIT
    trap 'on_signal SIGINT' INT
    trap 'on_signal SIGTERM' TERM

    log_startup_configuration

    SCRIPT_START_TIME=$(date +%s)

    case "$MODE" in
    happy)
        run_setup_and_committee_phases || return 1
        run_pegin_phase || return 1
        run_pegout_phase || return 1
        run_pegout_verification_phase || return 1
        ;;
    setup)
        run_setup_phase || return 1
        SETUP_DURATION=$(( $(date +%s) - SCRIPT_START_TIME ))
        ;;
    committee)
        run_committee_phase || return 1
        SETUP_DURATION=$(( $(date +%s) - SCRIPT_START_TIME ))
        ;;
    pegin)
        run_pegin_phase || return 1
        ;;
    pegout)
        run_pegout_phase || return 1
        ;;
    operator-take)
        run_operator_take_phase || return 1
        ;;
    *)
        echo "Error: unsupported mode '$MODE'" >&2
        return 1
        ;;
    esac

    print_success_summary
}

if [[ "${BASH_SOURCE[0]}" == "$0" ]]; then
    main "$@"
fi
