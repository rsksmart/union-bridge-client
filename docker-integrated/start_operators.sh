#!/usr/bin/env bash

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

UC_TAG=""
DOCKER_COMPOSE_ARGS=()
OPERATOR_ID=""
ENVIRONMENT=""

# Display help message
print_help() {
  echo "Usage: $0 --env <ENV> [--op <ID>] [OPTIONS] [DOCKER_COMPOSE_ARGS...]"
  echo ""
  echo "Required:"
  echo "  --env alphanet           Deploy on Alphanet, a specific operator (requires --op <ID>)"
  echo "  --env local              Deploy locally, all 4 operators"
  echo ""
  echo "Options:"
  echo "  --op <ID>                Specify operator ID (1, 2, 3, or 4) - required for alphanet"
  echo "  --help                   Display this help message"
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
  echo "  $0 --env local up -d                                     # Start all 4 operators locally"
  echo "  $0 --env local --fresh up -d                             # Clean and start all operators locally"
  echo "  $0 --env local down                                      # Stop all local operators"
  echo "  $0 --env alphanet --op 1 up -d                           # Start operator 1 in alphanet"
  echo "  $0 --env alphanet --op 2 up -d                           # Start operator 2 in alphanet"
  echo "  $0 --env alphanet --op 1 --tag latest-alphanet up -d     # Start operator 1 with specific tag"
  echo "  $0 --env alphanet --op 3 --fresh up -d                   # Clean and start operator 3 in alphanet"
  echo "  $0 --env alphanet --op 1 down --volumes                  # Cleanup operator 1 including volumes"
  echo "  $0 --env alphanet --op 1 logs                            # View logs for operator 1"
  echo "  $0 --env alphanet --op 4 up --force-recreate             # Recreate operator 4 from registry images"
  echo ""
  echo "Any additional arguments will be passed directly to docker compose."
  exit 0
}

if [[ -z "${MEMBER_BITCOIN_WIF}" ]]; then
  echo "Error: MEMBER_BITCOIN_WIF environment variable is not set."
  exit 1
fi

# Parse args
while [[ $# -gt 0 ]]; do
  case $1 in
    --help)
      print_help
      ;;
    --op)
      OPERATOR_ID="$2"
      if ! [[ "$OPERATOR_ID" =~ ^[1-4]$ ]]; then
        echo "Error: --op must be 1, 2, 3, or 4"
        exit 1
      fi
      shift 2
      ;;
    --tag)
      UC_TAG="$2"
      shift 2
      ;;
    --env)
      ENVIRONMENT="$2"
      if [[ "$ENVIRONMENT" == "alphanet" ]]; then
        ENV_FILE="${SCRIPT_DIR}/.env.alphanet"
        ENV_NAME="docker-alphanet"
        if [[ -z "$UC_TAG" ]]; then
          UC_TAG="latest-alphanet"
        fi
      elif [[ "$ENVIRONMENT" == "local" ]]; then
        ENV_FILE="${SCRIPT_DIR}/.env.local"
        ENV_NAME="docker-local"
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

if [[ -z "$ENVIRONMENT" ]]; then
  echo "Error: --env flag is required. Use 'alphanet' or 'local'."
  echo "Run '$0 --help' for usage information."
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

# Check if we're using a startup command
IS_STARTUP_COMMAND=false
for arg in "${DOCKER_COMPOSE_ARGS[@]}"; do
  if [[ "$arg" == "up" || "$arg" == "restart" || "$arg" == "start" || "$arg" == "create" ]]; then
    IS_STARTUP_COMMAND=true
    break
  fi
done

# Reject --op flag when not allowed
if [[ -n "$OPERATOR_ID" ]]; then
  # Case 1: --op is never allowed in local environment
  if [[ "$ENVIRONMENT" == "local" ]]; then
    echo "Error: --op is not allowed in local environment. All operators will be deployed."
    exit 1
  fi

  # Case 2: --op is not allowed with non-startup commands in alphanet
  if [[ "$ENVIRONMENT" == "alphanet" && "${IS_STARTUP_COMMAND}" == false ]]; then
    echo "Error: --op can only be used with startup commands (up, restart, start, create)."
    echo "For other commands, the script will target the operator on this host automatically."
    exit 1
  fi
fi

# Set OPERATOR_ID based on environment
if [[ "$ENVIRONMENT" == "local" ]]; then
  OPERATOR_ID="all"
elif [[ "$ENVIRONMENT" == "alphanet" ]]; then
  if [[ "${IS_STARTUP_COMMAND}" == true ]]; then
    # Startup commands: --op is required
    if [[ -z "$OPERATOR_ID" ]]; then
      echo "Error: --op <ID> is required when using --env alphanet with startup commands (up, restart, start, create)."
      echo "Run '$0 --help' for usage information."
      exit 1
    fi
  else
    # Non-startup commands: default to all
    OPERATOR_ID="all"
  fi
fi

# Prompt for WALLET_PRIVATE_KEY if running startup commands
if [[ "${IS_STARTUP_COMMAND}" == true ]]; then
  if [[ -z "${WALLET_PRIVATE_KEY}" ]]; then
    echo "Please enter WALLET_PRIVATE_KEY (input will be hidden):"
    read -s WALLET_PRIVATE_KEY
    echo ""

    if [[ -z "${WALLET_PRIVATE_KEY}" ]]; then
      echo "Error: WALLET_PRIVATE_KEY is required for 'up' or 'restart' commands."
      exit 1
    fi

    export WALLET_PRIVATE_KEY
  fi
fi

# Determine which operators to process
if [[ "$OPERATOR_ID" == "all" ]]; then
  OPERATORS_TO_RUN=(1 2 3 4)
else
  OPERATORS_TO_RUN=("$OPERATOR_ID")
fi

# If requested, clean operator stacks regardless of the main command
if [[ "${FRESH}" == true ]]; then
  if [[ "$OPERATOR_ID" == "all" ]]; then
    echo "Cleaning operator stacks (down --volumes) for projects op_1..op_4..."
    for i in 1 2 3 4; do
      cmd="docker compose -p op_${i} --env-file ${ENV_FILE} down --volumes || true"
      echo "Running: ${cmd}"
      eval "${cmd}"
    done
  else
    echo "Cleaning operator stack (down --volumes) for project op_${OPERATOR_ID}..."
    cmd="docker compose -p op_${OPERATOR_ID} --env-file ${ENV_FILE} down --volumes || true"
    echo "Running: ${cmd}"
    eval "${cmd}"
  fi
fi

run_local_operators() {
  # LOCAL ENVIRONMENT: All 4 operators on one host
  # Each operator uses different ports to avoid conflicts
  # Static IPs assigned via docker-compose.local.yml overlay

  # Create shared network for P2P communication between operators if missing
  local NETWORK_NAME="bitvmx-shared-network"
  if ! docker network inspect $NETWORK_NAME >/dev/null 2>&1; then
    echo "Creating docker network '$NETWORK_NAME'..."
    docker network create --driver bridge --subnet=172.20.0.0/16 $NETWORK_NAME
  fi
  
  local USER_API_PORTS=(40001 40002 40003 40004)
  local BITVMX_PORTS=(22222 33333 44444 55554)
  local BITVMX_P2P_PORTS=(61180 61181 61182 61183)
  # should match docker-integrated/bitvmx-client/config/local/broker/config/peers.yaml
  local BITVMX_P2P_HOSTS=("172.20.0.11" "172.20.0.12" "172.20.0.13" "172.20.0.14")
  local CLIENT_OPS=("op_1" "op_2" "op_3" "op_4")
  local COMPOSE_FILE_ARG="-f docker-compose.yml -f docker-compose.local.yml"

  for op_num in "${OPERATORS_TO_RUN[@]}"; do
    local i=$((op_num - 1))
    local USER_API_PORT=${USER_API_PORTS[$i]}
    local BITVMX_PORT=${BITVMX_PORTS[$i]}
    local BITVMX_P2P_HOST=${BITVMX_P2P_HOSTS[$i]}
    local BITVMX_P2P_PORT=${BITVMX_P2P_PORTS[$i]}
    local CLIENT_OP=${CLIENT_OPS[$i]}

    local DOCKER_CMD="USER_API_PORT=${USER_API_PORT} BITVMX_PORT=${BITVMX_PORT} BITVMX_P2P_HOST=${BITVMX_P2P_HOST} BITVMX_P2P_PORT=${BITVMX_P2P_PORT} CLIENT_OP=${CLIENT_OP} UC_TAG=${UC_TAG} docker compose ${COMPOSE_FILE_ARG} -p op_${op_num} --env-file ${ENV_FILE} ${DOCKER_COMPOSE_ARGS[*]}"

    echo
    echo "Starting operator ${op_num} with command: '${DOCKER_CMD}'"
    eval "${DOCKER_CMD}"
  done
}

run_alphanet_operators() {
  # ALPHANET ENVIRONMENT: Each operator on separate host
  # All operators use same ports since each is on its own host
  # No static IPs needed - bind to 0.0.0.0 in config
  # For startup commands: OPERATORS_TO_RUN will contain a single operator ID
  # For non-startup commands: CLIENT_OP doesn't matter (not creating containers)

  local CLIENT_OP
  if [[ "${IS_STARTUP_COMMAND}" == true ]]; then
    local op_num=${OPERATORS_TO_RUN[0]}
    CLIENT_OP="testnet_op_${op_num}"
    echo
    echo "Starting operator ${op_num} with command:"
  else
    # For non-startup commands, CLIENT_OP value doesn't matter but needs to be set for compose file parsing
    CLIENT_OP="dummy_op"
    echo
    echo "Running command on alphanet operator:"
  fi

  local DOCKER_CMD="USER_API_PORT=40001 BITVMX_PORT=22222 BITVMX_P2P_PORT=61180 CLIENT_OP=${CLIENT_OP} UC_TAG=${UC_TAG} docker compose -f docker-compose.yml --env-file ${ENV_FILE} ${DOCKER_COMPOSE_ARGS[*]}"
  echo "'${DOCKER_CMD}'"
  eval "${DOCKER_CMD}"
}

# Run operators based on environment
if [[ "$ENVIRONMENT" == "local" ]]; then
  run_local_operators
else
  run_alphanet_operators
fi