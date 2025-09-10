#!/bin/bash

set -euo pipefail

# Source common multi-client environment setup
source multi-client-env.sh

# Validate CLIENT_ID is provided and is one of [1-10]
CLIENT_ID=${1:-}
if [[ -z "${CLIENT_ID:-}" || ! "$CLIENT_ID" =~ ^([1-9]|10)$ ]]; then
  echo "Error: CLIENT_ID argument is required and must be one of [1-10]."
  echo "Usage: $0 <CLIENT_ID> [FEATURES]"
  exit 1
fi

# Validate BASE_STORAGE_PATH environment variable is set
if [[ -z "${BASE_STORAGE_PATH:-}" ]]; then
  echo "Error: BASE_STORAGE_PATH environment variable is required but not set."
  echo ""
  echo "Please set the BASE_STORAGE_PATH environment variable before running this script:"
  echo "  export BASE_STORAGE_PATH=/Users/username"
  echo "  $0 $CLIENT_ID [FEATURES]"
  echo ""
  echo "Example:"
  echo "  export BASE_STORAGE_PATH=/Users/username"
  echo "  $0 1 my-feature"
  exit 1
fi

# Validate FEATURES if provided
FEATURES=${2:-}
if [[ -n "$FEATURES" && ! "$FEATURES" =~ ^[a-zA-Z0-9_-]+$ ]]; then
  echo "Error: FEATURES must be a single word containing only letters, numbers, underscores, or hyphens."
  echo "Usage: export BASE_STORAGE_PATH=/path && $0 <CLIENT_ID> [FEATURES]"
  exit 1
fi

# Set environment variables for this client
set_multi_client_env "$CLIENT_ID" "$BASE_STORAGE_PATH"

# Run the client with the provided CLIENT_ID and optional FEATURES
if [[ -n "$FEATURES" ]]; then
  ./run-client.sh --features "$FEATURES" --config ./config/multi-client-template --logger log4rs.stdout.yaml
else
  ./run-client.sh --config ./config/multi-client-template --logger log4rs.stdout.yaml
fi
