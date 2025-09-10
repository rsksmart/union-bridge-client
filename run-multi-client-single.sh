#!/bin/bash

set -euo pipefail

# Validate ID is provided and is one of [1, 2, 3, 4]
ID=${1:-}
if [[ -z "${ID:-}" || ! "$ID" =~ ^[1-4]$ ]]; then
  echo "Error: ID argument is required and must be one of [1, 2, 3, 4]."
  echo "Usage: $0 <ID> [FEATURES]"
  exit 1
fi

# Validate FEATURES if provided
FEATURES=${2:-}
if [[ -n "$FEATURES" && ! "$FEATURES" =~ ^[a-zA-Z0-9_-]+$ ]]; then
  echo "Error: FEATURES must be a single word containing only letters, numbers, underscores, or hyphens."
  echo "Usage: $0 <ID> [FEATURES]"
  exit 1
fi

# Run the client with the provided ID and optional FEATURES
if [[ -n "$FEATURES" ]]; then
  CLIENT_ID=${ID} ./run-client.sh --features "$FEATURES" --config ./config/multi-client/$ID --logger log4rs.stdout.yaml
else
  CLIENT_ID=${ID} ./run-client.sh --config ./config/multi-client/$ID --logger log4rs.stdout.yaml
fi
