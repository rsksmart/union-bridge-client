#!/usr/bin/env bash

set -e

CMD=""
FEATURES=""
SERVICE=""
MOCKING=0
BUILD_BUILDER=0
HELP=0

show_help() {
  cat << EOF

🚀 Union Compose CLI - Docker Compose Management Tool

Usage: 
  $(basename "$0") <command> [options]

Commands:
  build               Build service container(s)
  up                  Start services
  down                Stop services
  build_builder       Build Builder images (standard and risc0 variants)

Options:
  --enable-mocking    On "build": this builds the actors-mocking service.
                      On "up": includes the actors-mocking service into the compose. A "build" is required before "up" for this to work.
                      This service provides anvil, deploys the contracts and starts a BitVMX Client mock (with broker).
  --feature-anvil     For "build" only: this builds the workspace with anvil feature, applying some tweaks in the code to improve Rootstock compatibility. An "up" will therefore run with that feature unless rebuilt without it.
  --service=<name>    For "build" only: this builds a specific service/crate, useful if you want to test a change in a specific service.
  --help, -h          Show this help

Notes on certain options:
  - --feature-anvil: is for "build" only, using it on "up" has no effect. You may need to rebuild to reflect the changes before starting the services.
  - --enable-mocking: is specified at build time, using it on "up" has no effect.

Examples:
  $(basename "$0") build_builder                                    Build the Builder images
  $(basename "$0") build                                            Build all services
  $(basename "$0") build --service=log-indexer                      Build a specific service
  $(basename "$0") build --feature-anvil --enable-mocking           Build with anvil feature and mocking enabled
  $(basename "$0") up                                               Start all services
  $(basename "$0") up --enable-mocking                              Start with mocking
  $(basename "$0") down                                             Stop all services

EOF
  exit 0
}

# Parse command line arguments
if [[ $# -eq 0 ]]; then
  show_help
fi

CMD="$1"
shift

while [[ $# -gt 0 ]]; do
  case "$1" in
    --enable-mocking)
      MOCKING=1
      ;;
    --feature-anvil)
      FEATURES="anvil"
      ;;
    --service=*)
      SERVICE="${1#*=}"
      ;;
    -h|--help)
      HELP=1
      ;;
    *)
      echo "Unknown option: $1"
      show_help
      ;;
  esac
  shift
done

if [[ $HELP -eq 1 ]]; then
  show_help
fi

# Build builder images
build_builder() {
  cmd=(docker build --ssh default -t union-client-builder:rust-1.86-v1 -f Dockerfile_builder .)
  echo "🔨 Building Standard Builder image with command: ${cmd[@]}"
  "${cmd[@]}"

  cmd=(docker build --ssh default --platform linux/amd64 -t union-client-builder:rust-1.86-risc0-v1 -f Dockerfile_builder_risc0 .)
  echo "🔨 Building Risc0 Builder image with command: ${cmd[@]}"
  "${cmd[@]}"

  echo "✅ Builder Base images ready"
}

# Build service containers
build_services() {
  echo "🔨 Building service container(s)..."

  if [[ $MOCKING -eq 1 ]]; then
    cmd=(docker compose -f docker-compose.yml -f docker-compose.mocking.yml build)
  else 
    cmd=(docker compose build)
  fi

  [[ -n $SERVICE ]] && cmd+=("$SERVICE" --build-arg JUST_CRATE="$SERVICE")
  [[ -n $FEATURES ]] && cmd+=(--build-arg FEATURES="$FEATURES")

  echo "Building with command: ${cmd[@]}"
  "${cmd[@]}"
  echo "✅ Build completed"
}

# Start services
start_services() {
  echo "🚀 Starting services..."

  if [[ $MOCKING -eq 1 ]]; then
    cmd=(docker compose -f docker-compose.yml -f docker-compose.mocking.yml up)
  else
    cmd=(docker compose up)
  fi

  echo "Running with command: ${cmd[@]}"
  "${cmd[@]}"
}

# Stop services
stop_services() {
  echo "🛑 Stopping services..."

  if [[ $MOCKING -eq 1 ]]; then
    cmd=(docker compose -f docker-compose.yml -f docker-compose.mocking.yml down)
  else
    cmd=(docker compose down)
  fi

  echo "Stopping with command: ${cmd[@]}"
  "${cmd[@]}"
  echo "✅ Services stopped"
}

# Main execution
case "$CMD" in
  build)
    build_services
    ;;
  up)
    start_services
    ;;
  down)
    stop_services
    ;;
  build_builder)
    build_builder
    ;;
  *)
    echo "Unknown command: $CMD"
    show_help
    ;;
esac
