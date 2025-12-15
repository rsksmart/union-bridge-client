#!/usr/bin/env bash

# wrapper script for run to run local operators
# usage: ./cli-run.sh --id 1 --fresh
#        ./cli-run.sh --features anvil
#        ./cli-run.sh --help
#        ./cli-run.sh --logs
#        ./cli-run.sh --start-mine    # start background mining (anvil + bitcoin)
#        ./cli-run.sh --stop-mine     # stop background mining

set -euo pipefail

# change to script directory to ensure relative paths work
cd "$(dirname "$0")"

MINE_PID_FILE="/tmp/union-bridge-mining.pids"

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

    # check connectivity
    if ! cast rpc eth_chainId --rpc-url http://localhost:8545 &> /dev/null; then
        warn "Anvil not running on localhost:8545"
        exit 1
    fi
    if ! bitcoin-cli -regtest -rpcuser=foo -rpcpassword=rpcpassword getblockcount &> /dev/null; then
        warn "Bitcoin regtest node not accessible"
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
        exit 0
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

# handle --start-mine option
if [[ "${1:-}" == "--start-mine" ]]; then
    start_mining
    exit 0
fi

# handle --stop-mine option
if [[ "${1:-}" == "--stop-mine" ]]; then
    stop_mining
    exit 0
fi

# handle --logs option
if [[ "${1:-}" == "--logs" ]]; then
  (
    pids=()

    # kill all children on Ctrl+C (INT) or TERM
    cleanup() {
      kill "${pids[@]}" 2>/dev/null || true
      exit 0
    }
    trap cleanup INT TERM

    for i in {1..4}; do
      tail -n0 -F "logs/coordinator-$i.log" | sed "s/^/[op-$i] /" &
      pids+=($!)
    done

    wait
  )
  exit 0
fi

# forward all arguments to run
RUST_BACKTRACE=1 exec cargo run --manifest-path cli/run/Cargo.toml -- "$@"
