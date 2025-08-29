#!/usr/bin/env bash

set -e

CMD=""
FEATURES=""
SERVICE=""
PLATFORM=""  # Will be set only if explicitly provided
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
  --feature-anvil     For "build" only: this builds the workspace with anvil feature, applying some tweaks in the code to improve Rootstock compatibility. An "up" will therefore run with that feature unless rebuilt without it.
  --service=<name>    For "build" only: this builds a specific service/crate, useful if you want to test a change in a specific service.
  --platform=<arch>   Specify the target platform for Docker builds
  --help, -h          Show this help

Notes on certain options:
  - --feature-anvil: is for "build" only, using it on "up" has no effect. You may need to rebuild to reflect the changes before starting the services.

Examples:
  $(basename "$0") build_builder                                    Build the Builder images
  $(basename "$0") build                                            Build all services
  $(basename "$0") build --service=log-indexer                      Build a specific service
  $(basename "$0") build --platform=linux/arm64                     Build for ARM64 platform
  $(basename "$0") up                                               Start all services
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
    --feature-anvil)
      FEATURES="anvil"
      ;;
    --service=*)
      SERVICE="${1#*=}"
      ;;
    --platform=*)
      PLATFORM="${1#*=}"
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
  cmd=(docker build --ssh default)
  [[ -n $PLATFORM ]] && cmd+=(--platform "$PLATFORM")
  cmd+=(-t union-client-builder:rust-1.86-v1 -f Dockerfile_builder .)
  echo "🔨 Building Standard Builder image with command: ${cmd[@]}"
  "${cmd[@]}"

# TODO when the zkp feature is re-enabled, use this to build coordinator in an image with amd64 support (required by Risc0 in Macs with M chips) via Dockerfile_coordinator
#  cmd=(docker build --ssh default)
#  [[ -n $PLATFORM ]] && cmd+=(--platform "$PLATFORM")
#  cmd+=(-t union-client-builder:rust-1.86-risc0-v1 -f Dockerfile_builder_risc0 .)
#  echo "🔨 Building Risc0 Builder image with command: ${cmd[@]}"
#  "${cmd[@]}"

  echo "✅ Builder Base images ready"
}

# Build service containers
build_services() {
  echo "🔨 Building service container(s)..."

  # Export PLATFORM only if explicitly provided
  [[ -n $PLATFORM ]] && export PLATFORM

  cmd=(docker compose build)

  [[ -n $SERVICE ]] && cmd+=("$SERVICE" --build-arg JUST_CRATE="$SERVICE")
  [[ -n $FEATURES ]] && cmd+=(--build-arg FEATURES="$FEATURES")

  if [[ -n $PLATFORM ]]; then
    echo "Building with command: ${cmd[@]} (PLATFORM=$PLATFORM)"
  else
    echo "Building with command: ${cmd[@]}"
  fi
  "${cmd[@]}"
  echo "✅ Build completed"
}

# Start services
start_services() {
  echo "🚀 Starting services..."

  # Export PLATFORM only if explicitly provided
  [[ -n $PLATFORM ]] && export PLATFORM

  cmd=(docker compose up)

  if [[ -n $PLATFORM ]]; then
    echo "Running with command: ${cmd[@]} (PLATFORM=$PLATFORM)"
  else
    echo "Running with command: ${cmd[@]}"
  fi
  "${cmd[@]}"
}

# Stop services
stop_services() {
  echo "🛑 Stopping services..."

  cmd=(docker compose down)

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
