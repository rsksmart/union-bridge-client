#!/usr/bin/env bash

# Test script for wait_for_log_with_block_timeout function
# This tests the function in isolation by mocking dependencies

set -euo pipefail

# Colors
GREEN='\033[0;32m'
RED='\033[0;31m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m'

log() { echo -e "${BLUE}[INFO]${NC} $1"; }
success() { echo -e "${GREEN}[✓]${NC} $1"; }
warn() { echo -e "${YELLOW}[!]${NC} $1"; }
error() { echo -e "${RED}[✗]${NC} $1"; }

# Test counters
TESTS_PASSED=0
TESTS_FAILED=0

# ===== Setup test environment =====
SCRIPT_ENV="local"
TEST_LOG_DIR="/tmp/test-logs-$$"
mkdir -p "$TEST_LOG_DIR"

# Create mock log files with CURRENT timestamps
create_test_logs() {
    local timestamp=$(date "+%Y-%m-%d %H:%M:%S")
    
    # Create coordinator logs with test patterns
    echo "$timestamp [ INFO] [coordinator] Loading configuration" > "$TEST_LOG_DIR/coordinator-1.log"
    echo "$timestamp [ INFO] [coordinator::flows::committee::setup_committee_flow] CommitteeSetupFlow Done: success" >> "$TEST_LOG_DIR/coordinator-1.log"
    echo "$timestamp [ INFO] [coordinator::monitor] Starting Block monitoring" >> "$TEST_LOG_DIR/coordinator-1.log"
    
    echo "$timestamp [ INFO] [coordinator] Loading configuration" > "$TEST_LOG_DIR/coordinator-2.log"
    echo "$timestamp [ INFO] [coordinator::flows::pegin] PeginFlow Done for packet 0" >> "$TEST_LOG_DIR/coordinator-2.log"
    
    # Log with old timestamp (should NOT match in find_recent_file_log_match)
    echo "2023-01-01 00:00:00 [ INFO] [coordinator] OldPattern that should not match" > "$TEST_LOG_DIR/coordinator-3.log"
    
    echo "$timestamp [ INFO] [coordinator] Loading configuration" > "$TEST_LOG_DIR/coordinator-4.log"
}

cleanup() {
    rm -rf "$TEST_LOG_DIR"
    log "Cleaned up test logs"
}
trap cleanup EXIT

# ===== Mock functions =====

# Mock bitcoin height - simulates block mining
MOCK_HEIGHT=100
get_current_bitcoin_height() {
    echo "$MOCK_HEIGHT"
}

# Increment mock height (simulate mining)
mine_mock_block() {
    MOCK_HEIGHT=$((MOCK_HEIGHT + 1))
}

# ===== Functions under test (copied from run-happy-path.sh) =====

# Find recent log match in local log files (modified to use TEST_LOG_DIR)
find_recent_file_log_match() {
    local pattern="$1"

    # min timestamp as string (1 minute ago)
    local min_ts
    min_ts=$(date -v-1M "+%Y-%m-%d %H:%M:%S" 2>/dev/null || date -d "1 minute ago" "+%Y-%m-%d %H:%M:%S")

    shopt -s nullglob
    for log_file in "$TEST_LOG_DIR"/coordinator-*.log; do
        [[ -f "$log_file" ]] || continue

        local found_line
        found_line=$(awk -v pattern="$pattern" -v min_ts="$min_ts" '
            $0 ~ pattern && substr($0, 1, 19) >= min_ts { print; exit }
        ' "$log_file")

        if [ -n "$found_line" ]; then
            echo "${log_file}:${found_line}"
            return 0
        fi
    done
}

# Wait for a log pattern to appear, with a block-based timeout
wait_for_log_with_block_timeout() {
    local pattern="$1"
    local max_blocks=$2

    local start_height=$(get_current_bitcoin_height)
    local target_height=$((start_height + max_blocks))

    log "Waiting for log pattern: $pattern (max $max_blocks blocks)..."

    while true; do
        local current_height=$(get_current_bitcoin_height)
        local blocks_mined=$((current_height - start_height))

        # safeguard
        if [ $blocks_mined -lt 0 ]; then
            sleep 0.1
            continue
        fi

        echo -ne "\r  Blocks mined: $blocks_mined/$max_blocks | Checking logs...  "

        # Check for log pattern in coordinator logs
        local result=""
        if [[ "$SCRIPT_ENV" == "docker" ]]; then
            # Would use find_recent_docker_log_match in real scenario
            result=""
        else
            result=$(find_recent_file_log_match "$pattern")
        fi

        if [ -n "$result" ]; then
            # parse "source:line" format
            local found_source="${result%%:*}"
            local found_line="${result#*:}"
            echo ""  # newline after the progress display
            success "Log pattern found after $blocks_mined blocks!"
            log "Found in: $found_source"
            echo "$found_line"
            return 0
        fi

        # Check if we've exceeded the block limit
        if [ $current_height -ge $target_height ]; then
            echo ""  # newline after the progress display
            warn "Log pattern not found after $max_blocks blocks (height: $start_height -> $current_height)"
            if [[ "$SCRIPT_ENV" == "docker" ]]; then
                warn "Check Docker logs manually: docker compose -p op_{1..4} logs coordinator"
            else
                warn "Check logs manually in: $TEST_LOG_DIR"
            fi
            return 1
        fi

        # In real script this would sleep 1, but for testing we speed it up
        sleep 0.1
        # Simulate mining for the test
        mine_mock_block
    done
}

# ===== Test Cases =====

echo ""
echo "======================================"
echo "  Testing wait_for_log_with_block_timeout"
echo "======================================"
echo ""

# Test 1: Pattern exists in logs - should find it immediately
test_pattern_found() {
    log "Test 1: Pattern exists and should be found"
    create_test_logs
    MOCK_HEIGHT=100
    
    if wait_for_log_with_block_timeout "CommitteeSetupFlow Done:" 5; then
        success "Test 1 PASSED: Pattern was found as expected"
        TESTS_PASSED=$((TESTS_PASSED + 1))
    else
        error "Test 1 FAILED: Pattern should have been found"
        TESTS_FAILED=$((TESTS_FAILED + 1))
    fi
    echo ""
}

# Test 2: Another pattern exists - PeginFlow Done
test_peginflow_pattern() {
    log "Test 2: PeginFlow Done pattern exists and should be found"
    create_test_logs
    MOCK_HEIGHT=100
    
    if wait_for_log_with_block_timeout "PeginFlow Done" 5; then
        success "Test 2 PASSED: PeginFlow pattern was found as expected"
        TESTS_PASSED=$((TESTS_PASSED + 1))
    else
        error "Test 2 FAILED: PeginFlow pattern should have been found"
        TESTS_FAILED=$((TESTS_FAILED + 1))
    fi
    echo ""
}

# Test 3: Pattern does NOT exist - should timeout
test_pattern_not_found() {
    log "Test 3: Pattern does NOT exist - should timeout after max_blocks"
    create_test_logs
    MOCK_HEIGHT=100
    
    if wait_for_log_with_block_timeout "NonExistentPattern12345" 3; then
        error "Test 3 FAILED: Should have timed out for non-existent pattern"
        TESTS_FAILED=$((TESTS_FAILED + 1))
    else
        success "Test 3 PASSED: Correctly timed out for non-existent pattern"
        TESTS_PASSED=$((TESTS_PASSED + 1))
    fi
    echo ""
}

# Test 4: Old timestamp pattern should NOT match (find_recent_file_log_match filters by time)
test_old_timestamp_not_matched() {
    log "Test 4: Old timestamp pattern should NOT match (only last minute)"
    create_test_logs
    MOCK_HEIGHT=100
    
    if wait_for_log_with_block_timeout "OldPattern" 3; then
        error "Test 4 FAILED: Old pattern should not have been found"
        TESTS_FAILED=$((TESTS_FAILED + 1))
    else
        success "Test 4 PASSED: Old timestamp pattern correctly ignored"
        TESTS_PASSED=$((TESTS_PASSED + 1))
    fi
    echo ""
}

# Test 5: Direct test of find_recent_file_log_match
test_find_recent_file_log_match_direct() {
    log "Test 5: Direct test of find_recent_file_log_match"
    create_test_logs
    
    local result=$(find_recent_file_log_match "CommitteeSetupFlow Done")
    
    if [ -n "$result" ]; then
        success "Test 5 PASSED: find_recent_file_log_match found pattern"
        log "Result: $result"
        TESTS_PASSED=$((TESTS_PASSED + 1))
    else
        error "Test 5 FAILED: find_recent_file_log_match should have found pattern"
        TESTS_FAILED=$((TESTS_FAILED + 1))
    fi
    echo ""
}

# Test 6: Test result parsing (source:line format)
test_result_parsing() {
    log "Test 6: Test result parsing (source:line format)"
    create_test_logs
    
    local result=$(find_recent_file_log_match "CommitteeSetupFlow Done")
    
    if [ -n "$result" ]; then
        local found_source="${result%%:*}"
        local found_line="${result#*:}"
        
        log "Full result: $result"
        log "Parsed source: $found_source"
        log "Parsed line: $found_line"
        
        if [[ "$found_source" == *"coordinator-"* ]] && [[ "$found_line" == *"CommitteeSetupFlow Done"* ]]; then
            success "Test 6 PASSED: Parsing works correctly"
            TESTS_PASSED=$((TESTS_PASSED + 1))
        else
            error "Test 6 FAILED: Parsing did not produce expected results"
            TESTS_FAILED=$((TESTS_FAILED + 1))
        fi
    else
        error "Test 6 FAILED: No result to parse"
        TESTS_FAILED=$((TESTS_FAILED + 1))
    fi
    echo ""
}

# ===== Run all tests =====
test_pattern_found
test_peginflow_pattern
test_pattern_not_found
test_old_timestamp_not_matched
test_find_recent_file_log_match_direct
test_result_parsing

# ===== Summary =====
echo ""
echo "======================================"
echo "  Test Summary"
echo "======================================"
echo ""
echo -e "Tests Passed: ${GREEN}$TESTS_PASSED${NC}"
echo -e "Tests Failed: ${RED}$TESTS_FAILED${NC}"
echo ""

if [ $TESTS_FAILED -eq 0 ]; then
    success "All tests passed!"
    exit 0
else
    error "Some tests failed!"
    exit 1
fi
