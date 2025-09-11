#!/usr/bin/env bash

set -e

# Parse command line arguments
PROFILE=""
ROOTSTOCK_URL="ws://host.docker.internal:8545"
USE_LOCAL_ANVIL=false

if [[ "$1" == "--profile" && -n "$2" ]]; then
    PROFILE="--profile $2"
    if [[ "$2" == "anvil" ]]; then
        ROOTSTOCK_URL="ws://host.docker.internal:8545"  # Connect to Docker anvil service via host

        # Build Docker anvil image first
        echo "Building Docker Anvil image..."
        bash d-compose-cli.sh build --features=anvil

        # Start Docker anvil
        echo "Starting Docker Anvil service..."
        docker compose --profile anvil up anvil -d

        # Wait for Docker anvil to be ready
        echo "Waiting for Docker Anvil to be ready..."
        for i in {1..30}; do
            if curl -s -X POST -H "Content-Type: application/json" --data '{"jsonrpc":"2.0","method":"eth_blockNumber","params":[],"id":1}' http://localhost:8545 > /dev/null 2>&1; then
                echo "Docker Anvil is ready!"
                break
            fi
            sleep 1
        done
    fi
    shift 2
elif [[ "$1" == "--local-anvil" ]]; then
    USE_LOCAL_ANVIL=true
    ROOTSTOCK_URL="ws://host.docker.internal:8545"  # Connect to local anvil via host
    
    # Check if local Anvil is running
    echo "Checking for local Anvil instance..."
    if ! curl -s -X POST -H "Content-Type: application/json" --data '{"jsonrpc":"2.0","method":"eth_blockNumber","params":[],"id":1}' http://localhost:8545 > /dev/null 2>&1; then
        echo "❌ Local Anvil not found at localhost:8545"
        echo "Please start Anvil locally first:"
        echo "  anvil --host 0.0.0.0 --port 8545 --auto-mine --block-time 2"
        exit 1
    fi
    echo "✅ Local Anvil detected and ready!"
    shift 1
fi

# Function to cleanup on exit
cleanup() {
    if [[ "$PROFILE" == "--profile anvil" ]]; then
        echo "Stopping Docker Anvil service..."
        docker compose --profile anvil down anvil
    elif [[ "$USE_LOCAL_ANVIL" == "true" ]]; then
        echo "Note: Local Anvil is still running. Stop it manually if needed."
    fi
}

# Set trap to cleanup on script exit
trap cleanup EXIT

# Start all 4 clients with the specified profile
if [[ "$PROFILE" == "--profile anvil" ]]; then
    # Start clients without anvil service (Docker anvil is already running)
    echo "Starting clients with Docker Anvil..."
    PLATFORM=${PLATFORM:-} ROOTSTOCK_URL=$ROOTSTOCK_URL BITVMX_HOST=host.docker.internal BITVMX_PORT=22222 docker compose -p uc-1 -f docker-compose-no-anvil.yml up -d
    PLATFORM=${PLATFORM:-} ROOTSTOCK_URL=$ROOTSTOCK_URL BITVMX_HOST=host.docker.internal BITVMX_PORT=33333 docker compose -p uc-2 -f docker-compose-no-anvil.yml up -d
    PLATFORM=${PLATFORM:-} ROOTSTOCK_URL=$ROOTSTOCK_URL BITVMX_HOST=host.docker.internal BITVMX_PORT=44444 docker compose -p uc-3 -f docker-compose-no-anvil.yml up -d
    PLATFORM=${PLATFORM:-} ROOTSTOCK_URL=$ROOTSTOCK_URL BITVMX_HOST=host.docker.internal BITVMX_PORT=55554 docker compose -p uc-4 -f docker-compose-no-anvil.yml up -d
elif [[ "$USE_LOCAL_ANVIL" == "true" ]]; then
    # Start clients with local anvil (no anvil service in compose)
    echo "Starting clients with local Anvil..."
    PLATFORM=${PLATFORM:-} ROOTSTOCK_URL=$ROOTSTOCK_URL BITVMX_HOST=host.docker.internal BITVMX_PORT=22222 docker compose -p uc-1 -f docker-compose-no-anvil.yml up -d
    PLATFORM=${PLATFORM:-} ROOTSTOCK_URL=$ROOTSTOCK_URL BITVMX_HOST=host.docker.internal BITVMX_PORT=33333 docker compose -p uc-2 -f docker-compose-no-anvil.yml up -d
    PLATFORM=${PLATFORM:-} ROOTSTOCK_URL=$ROOTSTOCK_URL BITVMX_HOST=host.docker.internal BITVMX_PORT=44444 docker compose -p uc-3 -f docker-compose-no-anvil.yml up -d
    PLATFORM=${PLATFORM:-} ROOTSTOCK_URL=$ROOTSTOCK_URL BITVMX_HOST=host.docker.internal BITVMX_PORT=55554 docker compose -p uc-4 -f docker-compose-no-anvil.yml up -d
else
    # Regular startup (no anvil)
    echo "Starting clients without Anvil..."
    PLATFORM=${PLATFORM:-} ROOTSTOCK_URL=$ROOTSTOCK_URL BITVMX_HOST=host.docker.internal BITVMX_PORT=22222 docker compose -p uc-1 -f docker-compose.yml $PROFILE up -d
    PLATFORM=${PLATFORM:-} ROOTSTOCK_URL=$ROOTSTOCK_URL BITVMX_HOST=host.docker.internal BITVMX_PORT=33333 docker compose -p uc-2 -f docker-compose.yml $PROFILE up -d
    PLATFORM=${PLATFORM:-} ROOTSTOCK_URL=$ROOTSTOCK_URL BITVMX_HOST=host.docker.internal BITVMX_PORT=44444 docker compose -p uc-3 -f docker-compose.yml $PROFILE up -d
    PLATFORM=${PLATFORM:-} ROOTSTOCK_URL=$ROOTSTOCK_URL BITVMX_HOST=host.docker.internal BITVMX_PORT=55554 docker compose -p uc-4 -f docker-compose.yml $PROFILE up -d
fi

if [[ "$PROFILE" == "--profile anvil" ]]; then
    echo "All clients started with Docker Anvil. Press Ctrl+C to stop all services and Anvil."
elif [[ "$USE_LOCAL_ANVIL" == "true" ]]; then
    echo "All clients started with local Anvil. Press Ctrl+C to stop all services (Anvil will keep running)."
else
    echo "All clients started. Press Ctrl+C to stop all services."
fi

# Keep the script running until interrupted
while true; do
    sleep 1
done
