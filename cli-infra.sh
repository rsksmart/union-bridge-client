#!/usr/bin/env bash

# wrapper script for local infrastructure management
# usage: ./cli-infra.sh --start [--fresh] [--contracts-tag TAG] [--pull-contracts]
#        ./cli-infra.sh --stop
#        ./cli-infra.sh --start-blockchains [--fresh] [--contracts-tag TAG] [--pull-contracts]
#        ./cli-infra.sh --stop-blockchains
#        ./cli-infra.sh --stop-mining
#        ./cli-infra.sh --start-bitvmx [--fresh]
#        ./cli-infra.sh --stop-bitvmx
set -euo pipefail

cd "$(dirname "$0")"

ENVIRONMENT="local"
LOCAL_REGTEST_TAG_ARGS=()
FILTERED_ARGS=()
while [[ $# -gt 0 ]]; do
    case "$1" in
        --env)
            if [[ $# -lt 2 || -z "${2:-}" ]]; then
                echo "Error: --env requires a non-empty value (local or local-regtest)" >&2
                exit 1
            fi
            ENVIRONMENT="$2"
            shift 2
            ;;
        --rskj-tag|--powpeg-tag)
            if [[ $# -lt 2 || -z "${2:-}" ]]; then
                echo "Error: $1 requires a non-empty Docker tag" >&2
                exit 1
            fi
            LOCAL_REGTEST_TAG_ARGS+=("$1" "$2")
            shift 2
            ;;
        *)
            FILTERED_ARGS+=("$1")
            shift
            ;;
    esac
done
if (( ${#FILTERED_ARGS[@]} > 0 )); then
    set -- "${FILTERED_ARGS[@]}"
else
    set --
fi

case "$ENVIRONMENT" in
    local|local-regtest)
        ;;
    *)
        echo "Error: --env must be 'local' or 'local-regtest'" >&2
        exit 1
        ;;
esac

MINE_PID_FILE="/tmp/union-bridge-${ENVIRONMENT}-mining.pids"
BITCOIN_WALLET="mainwallet"
BITCOIN_RPC_ARGS=(-regtest -rpcuser=foo -rpcpassword=rpcpassword -rpcwallet="${BITCOIN_WALLET}")

GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m'

log() { echo -e "${GREEN}[✓]${NC} $1"; }
warn() { echo -e "${YELLOW}[!]${NC} $1"; }

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

on_exit() {
    local exit_code=$?
    if [[ $exit_code -eq 0 ]]; then
        notify_completion "CLI Infra" "Completed"
    else
        notify_completion "CLI Infra" "Failed"
    fi
    return "$exit_code"
}
trap on_exit EXIT

pid_is_running() {
    [[ "${1:-}" =~ ^[0-9]+$ ]] && kill -0 "$1" 2>/dev/null
}

rootstock_rpc_url() {
    if [[ "$ENVIRONMENT" == "local-regtest" ]]; then
        echo "http://localhost:8545"
    else
        echo "http://localhost:8545"
    fi
}

rootstock_label() {
    if [[ "$ENVIRONMENT" == "local-regtest" ]]; then
        echo "Rootstock regtest"
    else
        echo "Anvil"
    fi
}

mining_is_running() {
    if [ ! -f "$MINE_PID_FILE" ]; then
        return 1
    fi

    local anvil_pid="" bitcoin_pid=""
    read -r anvil_pid bitcoin_pid < "$MINE_PID_FILE" 2>/dev/null || true

    pid_is_running "$anvil_pid" || pid_is_running "$bitcoin_pid"
}

cleanup_stale_mining_pid_file() {
    if [ -f "$MINE_PID_FILE" ] && ! mining_is_running; then
        rm -f "$MINE_PID_FILE"
    fi
}

mine_anvil() {
    local rpc_url="$1"
    if ! cast rpc anvil_mine 1 --rpc-url "$rpc_url" &>/dev/null; then
        echo -e "${YELLOW}[WARN]${NC} Anvil mining failed - is Anvil running?" >&2
        return 1
    fi

    while true; do
        cast rpc anvil_mine 1 --rpc-url "$rpc_url" &>/dev/null || true
        sleep 1
    done
}

mine_bitcoin() {
    local mine_address="$1"

    while true; do
        bitcoin-cli "${BITCOIN_RPC_ARGS[@]}" generatetoaddress 1 "$mine_address" &>/dev/null || true
        sleep 5
    done
}

bitcoin_trusted_balance_btc() {
    bitcoin-cli "${BITCOIN_RPC_ARGS[@]}" getbalances 2>/dev/null \
        | tr -d '\n ' \
        | sed -n 's/.*"trusted":\([0-9.]*\).*/\1/p'
}

bootstrap_bitcoin_wallet() {
    local trusted_balance_btc
    trusted_balance_btc=$(bitcoin_trusted_balance_btc)

    if [ -z "$trusted_balance_btc" ]; then
        warn "Failed to read trusted Bitcoin balance from ${BITCOIN_WALLET}"
        exit 1
    fi

    if [[ "$trusted_balance_btc" != "0.00000000" && "$trusted_balance_btc" != "0" ]]; then
        log "Bitcoin wallet '${BITCOIN_WALLET}' already has ${trusted_balance_btc} BTC of mature funds"
        return 0
    fi

    local bootstrap_address
    bootstrap_address=$(bitcoin-cli "${BITCOIN_RPC_ARGS[@]}" getnewaddress bootstrap bech32 2>/dev/null)
    if [ -z "$bootstrap_address" ]; then
        warn "Failed to get Bitcoin bootstrap address from ${BITCOIN_WALLET}"
        exit 1
    fi

    log "Bootstrapping Bitcoin wallet '${BITCOIN_WALLET}' with 101 regtest blocks"
    if ! bitcoin-cli "${BITCOIN_RPC_ARGS[@]}" generatetoaddress 101 "$bootstrap_address" &>/dev/null; then
        warn "Failed to bootstrap Bitcoin wallet '${BITCOIN_WALLET}'"
        exit 1
    fi
}

start_mining() {
    cleanup_stale_mining_pid_file

    if [ -f "$MINE_PID_FILE" ]; then
        local anvil_pid bitcoin_pid
        read -r anvil_pid bitcoin_pid < "$MINE_PID_FILE" 2>/dev/null || true
        if pid_is_running "$anvil_pid" || pid_is_running "$bitcoin_pid"; then
            warn "Mining already running (PIDs: $anvil_pid, $bitcoin_pid)"
            warn "Use ./cli-infra.sh --stop-mining if mining is stuck, or ./cli-infra.sh --stop-blockchains to stop everything first"
            exit 1
        fi
    fi

    if ! command -v cast &> /dev/null; then
        warn "cast command not found (install Foundry)"
        exit 1
    fi
    if ! command -v bitcoin-cli &> /dev/null; then
        warn "bitcoin-cli command not found"
        exit 1
    fi

    if ! cast rpc eth_chainId --rpc-url "$(rootstock_rpc_url)" &> /dev/null; then
        warn "Blockchains not started - $(rootstock_label) not running on $(rootstock_rpc_url)"
        warn "Start blockchains first with: ./cli-infra.sh --start-blockchains"
        exit 1
    fi
    if ! bitcoin-cli "${BITCOIN_RPC_ARGS[@]}" getblockcount &> /dev/null; then
        warn "Blockchains not started - Bitcoin regtest node not accessible"
        warn "Start blockchains first with: ./cli-infra.sh --start-blockchains"
        exit 1
    fi

    if [[ "$ENVIRONMENT" == "local" ]]; then
        log "Testing Anvil mining..."
        if ! cast rpc anvil_mine 1 --rpc-url "$(rootstock_rpc_url)" &>/dev/null; then
            warn "Anvil mining failed - check Anvil is running correctly"
            exit 1
        fi
    else
        log "Rootstock regtest mining is handled by RSKj; only Bitcoin background mining will run"
    fi

    log "Testing Bitcoin mining..."
    local test_address
    test_address=$(bitcoin-cli "${BITCOIN_RPC_ARGS[@]}" getnewaddress 2>/dev/null)
    if [ -z "$test_address" ]; then
        warn "Failed to get Bitcoin address for mining - check wallet is loaded"
        exit 1
    fi
    if ! bitcoin-cli "${BITCOIN_RPC_ARGS[@]}" generatetoaddress 1 "$test_address" &>/dev/null; then
        warn "Bitcoin mining failed - check bitcoind is running correctly"
        exit 1
    fi

    log "Starting background mining..."
    log "  Bitcoin: every 5s"

    local anvil_pid="-"
    if [[ "$ENVIRONMENT" == "local" ]]; then
        log "  Anvil: every 1s"
        nohup bash -c "$(declare -f mine_anvil); mine_anvil \"\$1\"" _ "$(rootstock_rpc_url)" >/tmp/union-bridge-${ENVIRONMENT}-anvil-mining.log 2>&1 &
        anvil_pid=$!
    fi

    nohup bash -c '
mine_address="$1"
while true; do
    bitcoin-cli -regtest -rpcuser=foo -rpcpassword=rpcpassword -rpcwallet=mainwallet generatetoaddress 1 "$mine_address" &>/dev/null || true
    sleep 5
done
' _ "$test_address" >/tmp/union-bridge-${ENVIRONMENT}-bitcoin-mining.log 2>&1 &
    local bitcoin_pid=$!

    echo "$anvil_pid $bitcoin_pid" > "$MINE_PID_FILE"

    log "Mining started (PIDs: $anvil_pid, $bitcoin_pid)"
}

stop_mining() {
    if [ ! -f "$MINE_PID_FILE" ]; then
        warn "No mining processes found (PID file missing)"
        return 0
    fi

    local anvil_pid bitcoin_pid
    read -r anvil_pid bitcoin_pid < "$MINE_PID_FILE" 2>/dev/null || true

    local stopped=false

    if [ -n "$anvil_pid" ] && kill -0 "$anvil_pid" 2>/dev/null; then
        kill -TERM "$anvil_pid" 2>/dev/null || true
        sleep 0.2
        kill -9 "$anvil_pid" 2>/dev/null || true
        log "Stopped Anvil mining (PID: $anvil_pid)"
        stopped=true
    fi

    if [ -n "$bitcoin_pid" ] && kill -0 "$bitcoin_pid" 2>/dev/null; then
        kill -TERM "$bitcoin_pid" 2>/dev/null || true
        sleep 0.2
        kill -9 "$bitcoin_pid" 2>/dev/null || true
        log "Stopped Bitcoin mining (PID: $bitcoin_pid)"
        stopped=true
    fi

    rm -f "$MINE_PID_FILE"

    if [ "$stopped" = false ]; then
        warn "Mining processes were not running"
    else
        log "Mining stopped"
    fi
}

start_blockchains() {
    shift
    local args=("$@")
    local i
    for (( i = 0; i < ${#args[@]}; i++ )); do
        if [[ "${args[i]}" == "--contracts-tag" ]]; then
            if [[ $(( i + 1 )) -ge ${#args[@]} || -z "${args[i+1]:-}" ]]; then
                echo "Error: --contracts-tag requires a non-empty value (e.g. local-build or v0.2.0-alpha.1)" >&2
                exit 1
            fi
        fi
    done

    cleanup_stale_mining_pid_file
    if mining_is_running; then
        log "Stopping existing background mining before restarting blockchains"
        stop_mining
    fi

    local blockchains_cmd=(--env "$ENVIRONMENT")
    if (( ${#LOCAL_REGTEST_TAG_ARGS[@]} > 0 )); then
        blockchains_cmd+=("${LOCAL_REGTEST_TAG_ARGS[@]}")
    fi
    blockchains_cmd+=("$@" up -d)
    docker/local-infra/start-blockchains.sh "${blockchains_cmd[@]}"
    bootstrap_bitcoin_wallet
    start_mining
}

stop_blockchains() {
    stop_mining
    docker/local-infra/start-blockchains.sh --env "$ENVIRONMENT" down
}

start_bitvmx() {
    shift
    docker/local-infra/start-bitvmx.sh "$@" up -d
}

stop_bitvmx() {
    docker/local-infra/start-bitvmx.sh down
}

start_all() {
    shift
    local -a BLOCKCHAINS_OPTS=()
    local -a BITVMX_OPTS=()
    while [[ $# -gt 0 ]]; do
        case "$1" in
            --fresh)
                BLOCKCHAINS_OPTS+=(--fresh)
                BITVMX_OPTS+=(--fresh)
                shift
                ;;
            --contracts-tag)
                if [[ $# -lt 2 || -z "${2:-}" ]]; then
                    echo "Error: --contracts-tag requires a non-empty value (e.g. local-build or v0.2.0-alpha.1)" >&2
                    exit 1
                fi
                BLOCKCHAINS_OPTS+=(--contracts-tag "$2")
                shift 2
                ;;
            --pull-contracts)
                BLOCKCHAINS_OPTS+=(--pull-contracts)
                shift
                ;;
            --rskj-tag|--powpeg-tag)
                if [[ $# -lt 2 || -z "${2:-}" ]]; then
                    echo "Error: $1 requires a non-empty Docker tag" >&2
                    exit 1
                fi
                BLOCKCHAINS_OPTS+=("$1" "$2")
                shift 2
                ;;
            *)
                BLOCKCHAINS_OPTS+=("$1")
                BITVMX_OPTS+=("$1")
                shift
                ;;
        esac
    done
    local blockchains_cmd=(--start-blockchains)
    if (( ${#BLOCKCHAINS_OPTS[@]} > 0 )); then
        blockchains_cmd+=("${BLOCKCHAINS_OPTS[@]}")
    fi
    start_blockchains "${blockchains_cmd[@]}"

    if (( ${#BITVMX_OPTS[@]} > 0 )); then
        docker/local-infra/start-bitvmx.sh "${BITVMX_OPTS[@]}" up -d
    else
        docker/local-infra/start-bitvmx.sh up -d
    fi
}

stop_all() {
    log "Stopping docker infrastructure..."
    docker/local-infra/start-bitvmx.sh down
    stop_blockchains
}

case "${1:-}" in
    --start)
        start_all "$@"
        ;;
    --stop)
        stop_all
        ;;
    --start-blockchains)
        start_blockchains "$@"
        ;;
    --stop-blockchains)
        stop_blockchains
        ;;
    --stop-mining)
        stop_mining
        ;;
    --start-bitvmx)
        start_bitvmx "$@"
        ;;
    --stop-bitvmx)
        stop_bitvmx
        ;;
    *)
        echo "Usage: $0 [--env local|local-regtest] {--start|--stop|--start-blockchains|--stop-blockchains|--stop-mining|--start-bitvmx|--stop-bitvmx}"
        echo ""
        echo "Local Docker Infrastructure:"
        echo "  --start [--fresh] [--contracts-tag TAG] [--pull-contracts] [--rskj-tag TAG] [--powpeg-tag TAG]"
        echo "                                                       Start all blockchains + bitvmx + mining"
        echo "  --stop                                               Stop mining + bitvmx + blockchains"
        echo "  --start-blockchains [--fresh] [--contracts-tag TAG] [--pull-contracts] [--rskj-tag TAG] [--powpeg-tag TAG]"
        echo "                                                       Start blockchains + mining"
        echo "  --stop-blockchains                                   Stop mining + blockchains"
        echo "  --stop-mining                                        Stop background mining only; run this if mining gets stuck"
        echo "  --start-bitvmx [--fresh]                             Start bitvmx only"
        echo "  --stop-bitvmx                                        Stop bitvmx only"
        echo ""
        echo "Options:"
        echo "  --env local|local-regtest                            Select blockchain environment (default: local)"
        echo "  --fresh                                              Clean/reset volumes before starting"
        echo "  --contracts-tag TAG                                  Predeployed Anvil/contracts tag (only for blockchains; e.g. local-build or v0.2.0-alpha.1)"
        echo "  --pull-contracts                                     Pull predeployed Anvil image from registry even if it exists locally"
        echo "  --rskj-tag TAG                                       Official rsksmart/rskj tag for --env local-regtest"
        echo "  --powpeg-tag TAG                                     Official rsksmart/powpeg-node tag for --env local-regtest"
        exit 1
        ;;
esac
