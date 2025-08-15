#!/bin/bash

set -euo pipefail

# Validate CLIENT_ID is provided and is one of [1, 2, 3, 4]
CLIENT_ID=${1:-}
if [[ -z "${CLIENT_ID:-}" || ! "$CLIENT_ID" =~ ^[1-4]$ ]]; then
  echo "Error: CLIENT_ID argument is required and must be one of [1, 2, 3, 4]."
  echo "Usage: $0 <CLIENT_ID> [FEATURES]"
  exit 1
fi

# Validate FEATURES if provided
FEATURES=${2:-}
if [[ -n "$FEATURES" && ! "$FEATURES" =~ ^[a-zA-Z0-9_-]+$ ]]; then
  echo "Error: FEATURES must be a single word containing only letters, numbers, underscores, or hyphens."
  echo "Usage: $0 <CLIENT_ID> [FEATURES]"
  exit 1
fi

# Run the client with the provided CLIENT_ID and optional FEATURES
if [[ -n "$FEATURES" ]]; then
  ./run-client.sh --features "$FEATURES" --config ./config/multi-client/$CLIENT_ID --logger log4rs.stdout.yaml
else
  ./run-client.sh --config ./config/multi-client/$CLIENT_ID --logger log4rs.stdout.yaml
fi
