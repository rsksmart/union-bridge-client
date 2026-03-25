#!/usr/bin/env bash

# wrapper script for infrastructure management
# usage: ./cli-infra.sh --start [--fresh] [--contracts-tag TAG]  # start all docker infra (blockchains + bitvmx) + mining
#        ./cli-infra.sh --stop                         # stop mining + all docker infra
#        ./cli-infra.sh --start-blockchains [--fresh] [--contracts-tag TAG]  # start blockchains docker containers only
#        ./cli-infra.sh --stop-blockchains             # stop blockchains docker containers only
#        ./cli-infra.sh --start-bitvmx [--fresh]       # start bitvmx docker containers only
#        ./cli-infra.sh --stop-bitvmx                  # stop bitvmx docker containers only
#        ./cli-infra.sh --start-mine                   # start background mining (anvil + bitcoin)
#        ./cli-infra.sh --stop-mine                    # stop background mining
#        ./cli-infra.sh --start-regtest                # start regtest operators via SSH (fast path)
#        ./cli-infra.sh --start-regtest --fresh        # run full fresh orchestration on regtest instance (remote-only)
#        ./cli-infra.sh --stop-regtest                 # stop regtest operators via SSH

set -euo pipefail

# change to script directory to ensure relative paths work
cd "$(dirname "$0")"

MINE_PID_FILE="/tmp/union-bridge-mining.pids"

# regtest remote config
REGTEST_HOST="union-bridge-use2-1.regtest.rskcomputing.net"
REGTEST_USER="ubuntu"
REGTEST_ROOT="union-bridge-client"
REGTEST_FRESH_REMOTE_SCRIPT="${REGTEST_FRESH_REMOTE_SCRIPT:-/home/${REGTEST_USER}/regtest-fresh/regtest_fresh.sh}"
REGTEST_FRESH_MODE="${REGTEST_FRESH_MODE:-remote}"

# colors
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m'

log() { echo -e "${GREEN}[✓]${NC} $1"; }
warn() { echo -e "${YELLOW}[!]${NC} $1"; }

# mining functions
mine_anvil() {
    # test first call
    if ! cast rpc anvil_mine 1 --rpc-url http://localhost:8545 &>/dev/null; then
        echo -e "${YELLOW}[WARN]${NC} Anvil mining failed - is Anvil running?" >&2
        return 1
    fi

    # mine indefinitely
    while true; do
        cast rpc anvil_mine 1 --rpc-url http://localhost:8545 &>/dev/null || true
        sleep 1
    done
}

mine_bitcoin() {
    # get address to mine to (once)
    local mine_address
    mine_address=$(bitcoin-cli -regtest -rpcuser=foo -rpcpassword=rpcpassword getnewaddress 2>/dev/null)
    if [ -z "$mine_address" ]; then
        echo -e "${YELLOW}[WARN]${NC} Failed to get Bitcoin address for mining" >&2
        return 1
    fi

    # mine indefinitely
    while true; do
        bitcoin-cli -regtest -rpcuser=foo -rpcpassword=rpcpassword generatetoaddress 1 "$mine_address" &>/dev/null || true
        sleep 5
    done
}

start_mining() {
    # check if already running
    if [ -f "$MINE_PID_FILE" ]; then
        local anvil_pid bitcoin_pid
        read -r anvil_pid bitcoin_pid < "$MINE_PID_FILE" 2>/dev/null || true
        if [ -n "$anvil_pid" ] && kill -0 "$anvil_pid" 2>/dev/null; then
            warn "Mining already running (PIDs: $anvil_pid, $bitcoin_pid)"
            warn "Use --stop-mine to stop first"
            exit 1
        fi
    fi

    # check prerequisites
    if ! command -v cast &> /dev/null; then
        warn "cast command not found (install Foundry)"
        exit 1
    fi
    if ! command -v bitcoin-cli &> /dev/null; then
        warn "bitcoin-cli command not found"
        exit 1
    fi

    # check blockchains are running
    if ! cast rpc eth_chainId --rpc-url http://localhost:8545 &> /dev/null; then
        warn "Blockchains not started - Anvil not running on localhost:8545"
        warn "Start blockchains first with: ./cli-infra.sh --start-blockchains"
        exit 1
    fi
    if ! bitcoin-cli -regtest -rpcuser=foo -rpcpassword=rpcpassword getblockcount &> /dev/null; then
        warn "Blockchains not started - Bitcoin regtest node not accessible"
        warn "Start blockchains first with: ./cli-infra.sh --start-blockchains"
        exit 1
    fi

    # test mining capability before starting background processes
    log "Testing Anvil mining..."
    if ! cast rpc anvil_mine 1 --rpc-url http://localhost:8545 &>/dev/null; then
        warn "Anvil mining failed - check Anvil is running correctly"
        exit 1
    fi

    log "Testing Bitcoin mining..."
    local test_address
    test_address=$(bitcoin-cli -regtest -rpcuser=foo -rpcpassword=rpcpassword getnewaddress 2>/dev/null)
    if [ -z "$test_address" ]; then
        warn "Failed to get Bitcoin address for mining - check wallet is loaded"
        exit 1
    fi
    if ! bitcoin-cli -regtest -rpcuser=foo -rpcpassword=rpcpassword generatetoaddress 1 "$test_address" &>/dev/null; then
        warn "Bitcoin mining failed - check bitcoind is running correctly"
        exit 1
    fi

    log "Starting background mining..."
    log "  Anvil: every 1s"
    log "  Bitcoin: every 5s"

    # start mining in background (already tested, so just loop)
    mine_anvil &
    local anvil_pid=$!

    mine_bitcoin &
    local bitcoin_pid=$!

    # save PIDs
    echo "$anvil_pid $bitcoin_pid" > "$MINE_PID_FILE"

    log "Mining started (PIDs: $anvil_pid, $bitcoin_pid)"
    log "Use --stop-mine to stop"
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
    shift # remove --start-blockchains from args
    # Validate --contracts-tag has a non-empty value (avoid passing "up" as value)
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
    docker/local-infra/start_blockchains.sh "$@" up -d
}

stop_blockchains() {
    docker/local-infra/start_blockchains.sh down
}

start_bitvmx() {
    shift # remove --start-bitvmx from args
    docker/local-infra/start_bitvmx.sh "$@" up -d
}

stop_bitvmx() {
    docker/local-infra/start_bitvmx.sh down
}

start_all() {
    shift # remove --start from args
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
            *)
                # Pass through other args (e.g. --force-recreate) to both scripts
                BLOCKCHAINS_OPTS+=("$1")
                BITVMX_OPTS+=("$1")
                shift
                ;;
        esac
    done
    docker/local-infra/start_blockchains.sh "${BLOCKCHAINS_OPTS[@]}" up -d
    docker/local-infra/start_bitvmx.sh "${BITVMX_OPTS[@]}" up -d

    log "Starting mining..."
    start_mining
}

stop_all() {
    log "Stopping mining..."
    stop_mining

    log "Stopping docker infrastructure..."
    docker/local-infra/start_bitvmx.sh down
    docker/local-infra/start_blockchains.sh down
}

start_regtest() {
    local fresh=false
    shift # remove --start-regtest

    while [[ $# -gt 0 ]]; do
        case "$1" in
            --fresh)
                fresh=true
                ;;
            *)
                warn "Unknown option for --start-regtest: $1"
                exit 1
                ;;
        esac
        shift
    done

    log "Connecting to regtest: ${REGTEST_HOST}"
    if [[ "$fresh" == true ]]; then
        if [[ "${REGTEST_FRESH_MODE}" == "local" ]]; then
            echo "Error: REGTEST_FRESH_MODE=local is no longer supported." >&2
            echo "Use remote mode (default): ./cli-infra.sh --start-regtest --fresh" >&2
            echo "Optional override: REGTEST_FRESH_MODE=remote ./cli-infra.sh --start-regtest --fresh" >&2
            exit 1
        fi

        if [[ "${REGTEST_FRESH_MODE}" != "remote" ]]; then
            echo "Error: unknown REGTEST_FRESH_MODE='${REGTEST_FRESH_MODE}'. Expected 'remote'." >&2
            exit 1
        fi

        warn "Fresh mode (remote-only): running script ${REGTEST_FRESH_REMOTE_SCRIPT} on ${REGTEST_HOST}"
        local remote_cmd="set -euo pipefail; if [[ ! -x '${REGTEST_FRESH_REMOTE_SCRIPT}' ]]; then echo 'Error: missing executable fresh script at ${REGTEST_FRESH_REMOTE_SCRIPT}' >&2; exit 1; fi; bash '${REGTEST_FRESH_REMOTE_SCRIPT}'"
        ssh -A "${REGTEST_USER}@${REGTEST_HOST}" "${remote_cmd}"
        log "Remote regtest fresh orchestration completed"
        return 0
    fi

    local remote_cmd="cd ~/${REGTEST_ROOT} && bash docker/operator/start_operators.sh --env regtest up -d"

    log "Starting regtest operators..."
    ssh -A "${REGTEST_USER}@${REGTEST_HOST}" "${remote_cmd}"

    log "Regtest operators started"
}

stop_regtest() {
    log "Connecting to regtest: ${REGTEST_HOST}"

    local remote_cmd="cd ~/${REGTEST_ROOT} && bash docker/operator/start_operators.sh --env regtest down"

    log "Stopping regtest operators..."
    ssh -A "${REGTEST_USER}@${REGTEST_HOST}" "${remote_cmd}"

    log "Regtest operators stopped"
}

# main command handling
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
    --start-bitvmx)
        start_bitvmx "$@"
        ;;
    --stop-bitvmx)
        stop_bitvmx
        ;;
    --start-mine)
        start_mining
        ;;
    --stop-mine)
        stop_mining
        ;;
    --start-regtest)
        start_regtest "$@"
        ;;
    --stop-regtest)
        stop_regtest
        ;;
    *)
        echo "Usage: $0 {--start|--stop|--start-blockchains|--stop-blockchains|--start-bitvmx|--stop-bitvmx|--start-mine|--stop-mine|--start-regtest|--stop-regtest}"
        echo ""
        echo "Local Docker Infrastructure:"
        echo "  --start [--fresh] [--contracts-tag TAG]              Start all blockchains + bitvmx + mining"
        echo "  --stop                         Stop mining + bitvmx + blockchains"
        echo "  --start-blockchains [--fresh] [--contracts-tag TAG]  Start blockchains only (anvil + bitcoin)"
        echo "  --stop-blockchains             Stop blockchains only"
        echo "  --start-bitvmx [--fresh]       Start bitvmx only"
        echo "  --stop-bitvmx                  Stop bitvmx only"
        echo ""
        echo "Mining (requires blockchains running):"
        echo "  --start-mine                   Start background mining (anvil + bitcoin)"
        echo "  --stop-mine                    Stop background mining"
        echo ""
        echo "Remote Regtest Infrastructure:"
        echo "  --start-regtest                Start regtest operators via SSH (fast path)"
        echo "  --start-regtest --fresh        Run full fresh orchestration on regtest instance (remote-only)"
        echo "  --stop-regtest                 Stop regtest operators via SSH"
        echo ""
        echo "Regtest Fresh Modes:"
        echo "  REGTEST_FRESH_MODE             remote (default). local is unsupported."
        echo "  REGTEST_FRESH_REMOTE_SCRIPT    Remote fresh script path (default: ${REGTEST_FRESH_REMOTE_SCRIPT})"
        echo ""
        echo "Options:"
        echo "  --fresh                        Clean/reset volumes before starting"
        echo "  --contracts-tag TAG             Contracts image tag (only for blockchains; e.g. local-build or v0.2.0-alpha.1)"
        exit 1
        ;;
esac
