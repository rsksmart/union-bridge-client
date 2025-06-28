#!/bin/bash

# Script to send test reports to Testomat.io
# Usage: ./send-to-testomat.sh [report-file]

set -e

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

# Function to print colored output
print_status() {
    echo -e "${GREEN}[INFO]${NC} $1"
}

print_warning() {
    echo -e "${YELLOW}[WARNING]${NC} $1"
}

print_error() {
    echo -e "${RED}[ERROR]${NC} $1"
}

# Check if environment variables are set
if [ -z "$TESTOMAT_API_KEY" ]; then
    print_error "TESTOMAT_API_KEY environment variable is not set"
    print_warning "Please set your Testomat API key:"
    print_warning "export TESTOMAT_API_KEY='your_api_key_here'"
    print_warning "Or copy .envrc.testomat.sample to .envrc.testomat and source it"
    exit 1
fi

if [ -z "$TESTOMAT_PROJECT_ID" ]; then
    print_error "TESTOMAT_PROJECT_ID environment variable is not set"
    print_warning "Please set your Testomat project ID:"
    print_warning "export TESTOMAT_PROJECT_ID='your_project_id_here'"
    print_warning "Or copy .envrc.testomat.sample to .envrc.testomat and source it"
    exit 1
fi

# Default report file (relative to qa-tools/testomat/)
DEFAULT_REPORT="../reports/tx_dispatcher.xml"
REPORT_FILE="${1:-$DEFAULT_REPORT}"

# Check if report file exists
if [ ! -f "$REPORT_FILE" ]; then
    print_error "Report file not found: $REPORT_FILE"
    print_warning "Available reports in ../reports/:"
    ls -la ../reports/ 2>/dev/null || print_warning "No reports directory found"
    exit 1
fi

print_status "Sending report to Testomat.io..."
print_status "Report file: $REPORT_FILE"
print_status "Project ID: $TESTOMAT_PROJECT_ID"
print_status "Configuration: testomat.yml"

# Testomat API endpoint
API_URL="https://app.testomat.io/api/v1/projects/$TESTOMAT_PROJECT_ID/testruns"

# Create temporary files for response
TEMP_RESPONSE=$(mktemp)
TEMP_HEADERS=$(mktemp)

# Send the report using curl and capture both response and headers
curl -s -w "%{http_code}" \
    -X POST \
    -H "Authorization: Bearer $TESTOMAT_API_KEY" \
    -H "Content-Type: application/xml" \
    --data-binary "@$REPORT_FILE" \
    -D "$TEMP_HEADERS" \
    "$API_URL" > "$TEMP_RESPONSE"

# Extract HTTP status code (last 3 bytes)
HTTP_CODE=$(tail -c 3 "$TEMP_RESPONSE")
# Extract response body (all but last 3 bytes)
RESPONSE_BODY=$(dd if="$TEMP_RESPONSE" bs=1 count=$(($(wc -c < "$TEMP_RESPONSE")-3)) 2>/dev/null)

# Clean up temp files
rm -f "$TEMP_RESPONSE" "$TEMP_HEADERS"

if [ "$HTTP_CODE" -eq 200 ] || [ "$HTTP_CODE" -eq 201 ]; then
    print_status "Report sent successfully to Testomat.io!"
    print_status "HTTP Status: $HTTP_CODE"
    if [ ! -z "$RESPONSE_BODY" ]; then
        print_status "Response: $RESPONSE_BODY"
    fi
else
    print_error "Failed to send report to Testomat.io"
    print_error "HTTP Status: $HTTP_CODE"
    print_error "Response: $RESPONSE_BODY"
    exit 1
fi 