#!/usr/bin/env bash
#
# Helper to inspect the Prometheus /metrics endpoints exposed by the four
# union-bridge-client services running locally. See docs/MONITORING.md for the
# full metric catalogue.
#
# Usage:
#   scripts/metrics.sh                       # curated summary of all services
#   scripts/metrics.sh raw                   # raw /metrics for all services
#   scripts/metrics.sh raw coordinator       # raw /metrics for one service
#   scripts/metrics.sh watch                 # live summary, refreshed every 2s
#   scripts/metrics.sh filter <pattern>      # grep the raw output of all services
#   scripts/metrics.sh check                 # smoke-test endpoints (exit 0 if all up)
#
# Override per-service host/port with env vars when you've rebound them:
#   METRICS_HOST=10.0.0.5 scripts/metrics.sh
#   COORDINATOR_PORT=19101 scripts/metrics.sh

set -uo pipefail

HOST="${METRICS_HOST:-localhost}"
COORDINATOR_PORT="${COORDINATOR_PORT:-9101}"
USER_API_PORT="${USER_API_METRICS_PORT:-9102}"
BLOCK_INDEXER_PORT="${BLOCK_INDEXER_PORT:-9103}"
LOG_INDEXER_PORT="${LOG_INDEXER_PORT:-9104}"

SERVICES=(
    "coordinator:${COORDINATOR_PORT}"
    "user-api:${USER_API_PORT}"
    "block-indexer:${BLOCK_INDEXER_PORT}"
    "log-indexer:${LOG_INDEXER_PORT}"
)

# Colours only when stdout is a terminal so piping to grep/less stays clean.
if [[ -t 1 ]]; then
    BOLD=$'\033[1m'
    DIM=$'\033[2m'
    GREEN=$'\033[32m'
    YELLOW=$'\033[33m'
    RED=$'\033[31m'
    CYAN=$'\033[36m'
    RESET=$'\033[0m'
else
    BOLD=""; DIM=""; GREEN=""; YELLOW=""; RED=""; CYAN=""; RESET=""
fi

usage() {
    sed -n '2,18p' "$0" | sed 's/^# \{0,1\}//'
}

# Fetch /metrics for a service. Echoes the body on success, sets exit code 0;
# echoes nothing and exits non-zero if the endpoint is unreachable.
fetch() {
    local port="$1"
    curl -sf --max-time 2 "http://${HOST}:${port}/metrics"
}

# Print a single metric value (or "—" when absent). Args:
#   $1: metrics body (raw text)
#   $2: metric name + optional label match (passed to grep -E)
#   $3: optional sprintf format applied to the numeric value (defaults to "%s")
extract() {
    local body="$1" pattern="$2" fmt="${3:-%s}"
    local line value
    line=$(printf '%s\n' "$body" | grep -E "^${pattern}( |\\{)" | head -1)
    if [[ -z "$line" ]]; then
        printf '%s' "—"
        return
    fi
    value=$(printf '%s' "$line" | awk '{print $NF}')
    # shellcheck disable=SC2059  # fmt is intentionally user-provided.
    printf "$fmt" "$value"
}

# Sum every sample of a counter (across label combinations).
sum_counter() {
    local body="$1" name="$2"
    printf '%s\n' "$body" | awk -v m="$name" '
        $0 ~ "^" m "(\\{| )" { sum += $NF }
        END { printf "%.0f", sum + 0 }
    '
}

print_coordinator_summary() {
    local body="$1"
    local liveness pings timeouts last_msg_ts events_total flows_pegin flows_pegout flows_advance flows_committee
    liveness=$(extract "$body" "union_bitvmx_liveness")
    pings=$(extract "$body" "union_bitvmx_pings_sent_total")
    timeouts=$(extract "$body" "union_bitvmx_ping_timeouts_total")
    last_msg_ts=$(extract "$body" "union_bitvmx_last_message_timestamp_seconds")
    events_total=$(sum_counter "$body" "union_coordinator_events_processed_total")
    flows_pegin=$(extract "$body" 'union_flows_active\{[^}]*type="pegin"[^}]*\}')
    flows_pegout=$(extract "$body" 'union_flows_active\{[^}]*type="pegout"[^}]*\}')
    flows_advance=$(extract "$body" 'union_flows_active\{[^}]*type="advance-funds"[^}]*\}')
    flows_committee=$(extract "$body" 'union_flows_active\{[^}]*type="committee-setup"[^}]*\}')

    local liveness_label="${liveness}"
    case "$liveness" in
        1) liveness_label="${GREEN}healthy (1)${RESET}" ;;
        0) liveness_label="${YELLOW}unknown (0)${RESET}" ;;
        -1) liveness_label="${RED}not-responding (-1)${RESET}" ;;
    esac

    local age="—"
    if [[ "$last_msg_ts" != "—" ]]; then
        local now elapsed
        now=$(date +%s)
        elapsed=$(awk -v n="$now" -v t="$last_msg_ts" 'BEGIN { printf "%.0f", n - t }')
        age="${elapsed}s ago"
    fi

    printf '  BitVMX liveness         : %s\n' "$liveness_label"
    printf '  BitVMX last message     : %s\n' "$age"
    printf '  BitVMX pings sent       : %s   (timeouts: %s)\n' "$pings" "$timeouts"
    printf '  Events processed total  : %s\n' "$events_total"
    printf '  Active flows            : pegin=%s pegout=%s advance-funds=%s committee-setup=%s\n' \
        "$flows_pegin" "$flows_pegout" "$flows_advance" "$flows_committee"
}

print_user_api_summary() {
    local body="$1"
    local requests_total duration_count
    requests_total=$(sum_counter "$body" "http_requests_total")
    duration_count=$(sum_counter "$body" "http_request_duration_seconds_count")

    printf '  HTTP requests total     : %s\n' "$requests_total"
    printf '  HTTP samples observed   : %s\n' "$duration_count"

    # Show the top label combinations so the operator sees traffic shape.
    local breakdown
    breakdown=$(printf '%s\n' "$body" \
        | grep -E '^http_requests_total\{' \
        | awk '{count[$1]+=$NF} END { for (k in count) printf "    %s %s\n", count[k], k }' \
        | sort -rn | head -5)
    if [[ -n "$breakdown" ]]; then
        printf '  Top endpoints:\n%s\n' "$breakdown"
    fi
}

print_indexer_summary() {
    local body="$1" indexer="$2"
    local height blocks_or_logs reorgs sub_errors
    height=$(extract "$body" "union_indexer_height\\{[^}]*indexer=\"${indexer}\"[^}]*\\}")
    if [[ "$indexer" == "block" ]]; then
        blocks_or_logs=$(sum_counter "$body" "union_indexer_blocks_indexed_total")
        reorgs=$(sum_counter "$body" "union_indexer_reorgs_total")
        printf '  Tip height (rsk)        : %s\n' "$height"
        printf '  Blocks indexed total    : %s\n' "$blocks_or_logs"
        printf '  Reorgs detected         : %s\n' "$reorgs"
    else
        blocks_or_logs=$(sum_counter "$body" "union_log_indexer_logs_indexed_total")
        local unmanaged
        unmanaged=$(sum_counter "$body" "union_log_indexer_unmanaged_contract_logs_total")
        printf '  Last log block (rsk)    : %s\n' "$height"
        printf '  Logs indexed total      : %s\n' "$blocks_or_logs"
        printf '  Unmanaged contract logs : %s\n' "$unmanaged"
    fi
    sub_errors=$(sum_counter "$body" "union_indexer_subscription_errors_total")
    printf '  Subscription errors     : %s\n' "$sub_errors"
}

print_summary() {
    printf '%sunion-bridge-client metrics @ %s%s\n' "$BOLD" "$(date '+%Y-%m-%d %H:%M:%S')" "$RESET"
    printf '%shost=%s%s\n\n' "$DIM" "$HOST" "$RESET"

    for entry in "${SERVICES[@]}"; do
        local svc="${entry%%:*}"
        local port="${entry##*:}"
        local body
        body=$(fetch "$port")
        local rc=$?

        printf '%s%s%s %s(:%s)%s\n' "$CYAN" "$svc" "$RESET" "$DIM" "$port" "$RESET"
        if [[ $rc -ne 0 || -z "$body" ]]; then
            printf '  %sdown or unreachable%s\n\n' "$RED" "$RESET"
            continue
        fi

        case "$svc" in
            coordinator)    print_coordinator_summary "$body" ;;
            user-api)       print_user_api_summary "$body" ;;
            block-indexer)  print_indexer_summary "$body" "block" ;;
            log-indexer)    print_indexer_summary "$body" "log" ;;
        esac
        printf '\n'
    done
}

print_raw() {
    local only="${1:-}"
    for entry in "${SERVICES[@]}"; do
        local svc="${entry%%:*}"
        local port="${entry##*:}"
        if [[ -n "$only" && "$only" != "$svc" ]]; then
            continue
        fi
        printf '%s===== %s (:%s) =====%s\n' "$BOLD" "$svc" "$port" "$RESET"
        local body
        body=$(fetch "$port") || {
            printf '  %sdown or unreachable%s\n' "$RED" "$RESET"
            continue
        }
        printf '%s\n' "$body"
        printf '\n'
    done
}

print_filter() {
    local pattern="$1"
    if [[ -z "$pattern" ]]; then
        echo "filter requires a pattern argument" >&2
        exit 2
    fi
    for entry in "${SERVICES[@]}"; do
        local svc="${entry%%:*}"
        local port="${entry##*:}"
        local body
        body=$(fetch "$port") || continue
        local hits
        hits=$(printf '%s\n' "$body" | grep -E "$pattern" || true)
        if [[ -n "$hits" ]]; then
            printf '%s===== %s (:%s) =====%s\n' "$BOLD" "$svc" "$port" "$RESET"
            printf '%s\n\n' "$hits"
        fi
    done
}

check_all() {
    local failed=0
    for entry in "${SERVICES[@]}"; do
        local svc="${entry%%:*}"
        local port="${entry##*:}"
        if fetch "$port" > /dev/null; then
            printf '%s[OK]%s   %s :%s\n' "$GREEN" "$RESET" "$svc" "$port"
        else
            printf '%s[DOWN]%s %s :%s\n' "$RED" "$RESET" "$svc" "$port"
            failed=1
        fi
    done
    exit "$failed"
}

main() {
    local cmd="${1:-summary}"
    case "$cmd" in
        ""|summary)
            print_summary
            ;;
        raw)
            print_raw "${2:-}"
            ;;
        filter)
            print_filter "${2:-}"
            ;;
        check)
            check_all
            ;;
        watch)
            if ! command -v watch > /dev/null; then
                echo "'watch' not found; fall back to: while true; do scripts/metrics.sh; sleep 2; clear; done" >&2
                exit 2
            fi
            # Re-exec ourselves under watch with no extra args (summary mode).
            exec watch -n 2 -c "$0"
            ;;
        -h|--help|help)
            usage
            ;;
        *)
            echo "Unknown command: $cmd" >&2
            usage >&2
            exit 2
            ;;
    esac
}

main "$@"
