#!/usr/bin/env bash

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

UC_TAG=""
DOCKER_COMPOSE_ARGS=()

# Display help message
print_help() {
  echo "Usage: $0 [OPTIONS] [DOCKER_COMPOSE_ARGS...]"
  echo ""
  echo "Options:"
  echo "  --help                   Display this help message"
  echo "  --env <ENV>              Set environment (alphanet or local)"
  echo "  --tag <TAG>              Set tag for Union Client"
  echo "  --fresh                  Tear down operators (and volumes) before running the command"
  echo ""
  echo "Common Docker Compose Arguments can be used. Examples:"
  echo "  up                       Create and start containers"
  echo "  down [--volumes]         Stop and remove containers, networks (add --volumes to also remove named/anonymous volumes)"
  echo "  ps                       List containers"
  echo "  logs                     View output from containers"
  echo "  --force-recreate         Recreate containers even if configuration and image haven't changed"
  echo "  Note: Building from source is not supported. Only registry images should be used."
  echo ""
  echo "Examples:"
  echo "  $0 --env alphanet up -d                         # Start operator services in alphanet environment"
  echo "  $0 --env local --tag latest-anvil up -d         # Start operators with specific tag"
  echo "  $0 --env alphanet --fresh up -d                 # Clean and start alphanet operators"
  echo "  $0 --env local --fresh up -d                    # Clean and start local operators"
  echo "  $0 --env alphanet down                          # Stop all operator services"
  echo "  $0 --env alphanet down --volumes                # Cleanup operators including volumes (destructive)"
  echo "  $0 --env alphanet logs                          # View logs"
  echo "  $0 --env alphanet ps                            # Check service status"
  echo "  $0 --env alphanet up --force-recreate           # Recreate containers from registry images"
  echo ""
  echo "Any additional arguments will be passed directly to docker compose."
  exit 0
}

if [[ -z "${WALLET_PRIVATE_KEY}" ]]; then
  echo "Error: WALLET_PRIVATE_KEY environment variable is not set."
  exit 1
fi

# Parse args
while [[ $# -gt 0 ]]; do
  case $1 in
    --help)
      print_help
      ;;
    --tag)
      UC_TAG="$2"
      shift 2
      ;;
    --env)
      if [[ "$2" == "alphanet" ]]; then
        OP_PREFIX="testnet_"
        ENV_FILE="${SCRIPT_DIR}/.env.alphanet"
        if [[ -z "$UC_TAG" ]]; then
          UC_TAG="latest-alphanet"
        fi

      elif [[ "$2" == "local" ]]; then
        OP_PREFIX=""
        ENV_FILE="${SCRIPT_DIR}/.env.local"
        if [[ -z "$UC_TAG" ]]; then
          UC_TAG="latest-anvil"
        fi
      else
        echo "Invalid environment. Use 'alphanet' or 'local'"
        exit 1
      fi
      shift 2
      ;;
    --fresh)
      FRESH=true
      shift
      ;;
    *)
      # Store any other arguments to pass to docker compose
      DOCKER_COMPOSE_ARGS+=("$1")
      shift
      ;;
  esac
done

if [[ -z "$ENV_FILE" ]]; then
  echo "Usage: $0 --env <alphanet|local> [--tag <tag>] [down]"
  exit 1
fi

# Check if build command is being used
for arg in "${DOCKER_COMPOSE_ARGS[@]}"; do
  if [[ "$arg" == "build" || "$arg" == "--build" || "$arg" == "-b" ]]; then
    echo "Error: Building from source is not supported with this script."
    echo "Only registry images should be used. Building images is not allowed."
    exit 1
  fi
done

NETWORK_NAME="bitvmx-shared-network"
if ! docker network inspect $NETWORK_NAME >/dev/null 2>&1; then
  echo "Creating docker network '$NETWORK_NAME'..."
  docker network create --driver bridge --subnet=172.20.0.0/16 $NETWORK_NAME
fi

# If requested, clean operator stacks regardless of the main command
if [[ "${FRESH}" == true ]]; then
  echo "Cleaning operator stacks (down --volumes) for projects op_1..op_4..."
  for i in 1 2 3 4; do
    cmd="docker compose -p op_${i} --env-file ${ENV_FILE} down --volumes || true"
    echo "Running: ${cmd}"
    eval "${cmd}"
  done
fi

USER_API_PORTS=(40001 40002 40003 40004)
BITVMX_PORTS=(22222 33333 44444 55554)
BITVMX_P2P_HOSTS=("172.20.0.11" "172.20.0.12" "172.20.0.13" "172.20.0.14")
BITVMX_P2P_PORTS=(61180 61181 61182 61183)
CLIENT_OPS=("${OP_PREFIX}op_1" "${OP_PREFIX}op_2" "${OP_PREFIX}op_3" "${OP_PREFIX}op_4")

for i in "${!USER_API_PORTS[@]}"; do
  USER_API_PORT=${USER_API_PORTS[$i]}
  BITVMX_PORT=${BITVMX_PORTS[$i]}
  BITVMX_P2P_HOST=${BITVMX_P2P_HOSTS[$i]}
  BITVMX_P2P_PORT=${BITVMX_P2P_PORTS[$i]}
  CLIENT_OP=${CLIENT_OPS[$i]}
  
  DOCKER_CMD="USER_API_PORT=${USER_API_PORT} BITVMX_PORT=${BITVMX_PORT} BITVMX_P2P_HOST=${BITVMX_P2P_HOST} BITVMX_P2P_PORT=${BITVMX_P2P_PORT} CLIENT_OP=${CLIENT_OP} UC_TAG=${UC_TAG} docker compose -p op_$((i+1)) --env-file ${ENV_FILE} ${DOCKER_COMPOSE_ARGS[*]}"
  echo
  echo "Starting operator $((i+1)) with command: '${DOCKER_CMD}'"
  eval "${DOCKER_CMD}"
done