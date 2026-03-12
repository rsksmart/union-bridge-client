#!/usr/bin/env bash

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

UC_TAG=""
DOCKER_COMPOSE_ARGS=()
OPERATOR_ARG=""
ENVIRONMENT=""
AUTO_CONFIRM=false
export BASE_STORAGE_PATH="${BASE_STORAGE_PATH:-$HOME}"

# Ensure broker key exists in host storage and compute broker pubkey hash.
# Uses ${BASE_STORAGE_PATH}/.union_bridge/keystore/broker.key (defaults BASE_STORAGE_PATH to $HOME).
bootstrap_broker_keystore() {
  export BASE_STORAGE_PATH="${BASE_STORAGE_PATH:-$HOME}"
  local keystore_dir="${BASE_STORAGE_PATH}/.union_bridge/keystore"
  local broker_key_path="${keystore_dir}/broker.key"
  local broker_hash_path="${keystore_dir}/broker.pubkey_hash"

  mkdir -p "${keystore_dir}"

  if [[ ! -f "${broker_key_path}" ]]; then
    echo "Broker key not found at ${broker_key_path}. Generating..."
    openssl genpkey -algorithm RSA -out "${broker_key_path}" -pkeyopt rsa_keygen_bits:2048 2>/dev/null
    chmod 600 "${broker_key_path}" || true
  fi

  openssl pkey -in "${broker_key_path}" -pubout -outform DER 2>/dev/null \
    | openssl dgst -sha256 -binary \
    | od -A n -v -t x1 | tr -d ' \n' > "${broker_hash_path}"

  echo "Using broker key: ${broker_key_path}"
  echo "Computed broker pubkey hash at: ${broker_hash_path}"
}

get_bootstrapped_broker_pubkey_hash() {
  local broker_hash_path="${BASE_STORAGE_PATH}/.union_bridge/keystore/broker.pubkey_hash"
  if [[ ! -f "${broker_hash_path}" ]]; then
    echo "Error: broker pubkey hash not found at ${broker_hash_path}" >&2
    exit 1
  fi
  tr -d ' \n' < "${broker_hash_path}"
}

# Copy externally managed broker.key and broker.pubkey_hash into the docker-compose
# project keystore volume so each project can keep independent user/member keys.
sync_broker_key_to_project_volume() {
  local project_name="$1"
  local host_keystore="${BASE_STORAGE_PATH}/.union_bridge/keystore"
  local volume_name="${project_name}_keystore"

  docker volume create "${volume_name}" >/dev/null
  docker run --rm \
    -v "${volume_name}:/keystore" \
    -v "${host_keystore}:/host-keystore:ro" \
    alpine:3.20 \
    sh -c "cp /host-keystore/broker.key /keystore/broker.key && cp /host-keystore/broker.pubkey_hash /keystore/broker.pubkey_hash"
}

# Display help message
print_help() {
  echo "Usage: $0 --env <ENV> [--op <ID>] [OPTIONS] [DOCKER_COMPOSE_ARGS...]"
  echo ""
  echo "Required:"
  echo "  --env alphanet           Deploy on Alphanet, a specific operator (requires --op <ID>)"
  echo "  --env local              Deploy locally, all 4 operators"
  echo ""
  echo "Options:"
  echo "  --op <ID>                Specify operator ID (1, 2, 3, or 4) - required for alphanet startup"
  echo "  --help                   Display this help message"
  echo "  --tag <TAG>              Set tag for Union Client"
  echo "  --fresh                  Tear down operators (and volumes) before running the command"
  echo "                           - Includes confirmation prompt to prevent accidental data loss"
  echo "                           - Clears all operator state and databases"
  echo "                           - Only allowed with --env local"
  echo "  --yes, -y                Automatic yes to fresh confirmation prompt (use with caution)"
  echo ""
  echo "Environment Details:"
  echo "  Local:"
  echo "    - Runs all 4 operators on one host (op_1, op_2, op_3, op_4)"
  echo "    - Config: bitvmx-client/config/local/client/config/op_X.yaml"
  echo "    - Uses bridge network (bitvmx-shared-network) for P2P communication"
  echo "    - Project name: op_1, op_2, op_3 & op_4"
  echo "  Alphanet:"
  echo "    - Runs one operator per host (testnet_op_X where X is from --op)"
  echo "    - Config: bitvmx-client/config/alphanet/client/config/testnet_op_X.yaml"
  echo "    - Uses host network mode for P2P connectivity across physical machines"
  echo "    - Project name: union-operator"
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
  echo "  $0 --env local --fresh --yes up -d                       # Clean and start all operators locally, no confirmation prompt"
  echo "  $0 --env local down                                      # Stop all local operators"
  echo "  $0 --env alphanet --op 1 up -d                           # Start operator 1 in alphanet"
  echo "  $0 --env alphanet --op 2 up -d                           # Start operator 2 in alphanet"
  echo "  $0 --env alphanet --op 1 --tag latest-alphanet up -d     # Start operator 1 with specific tag"
  echo "  $0 --env alphanet down --volumes                         # Stop operator on this alphanet host"
  echo "  $0 --env alphanet logs -f                                # View logs for operator on this host"
  echo ""
  echo "Any additional arguments will be passed directly to docker compose."
  exit 0
}

# Parse args
while [[ $# -gt 0 ]]; do
  case $1 in
    --help)
      print_help
      ;;
    --op)
      OPERATOR_ARG="$2"
      if ! [[ "$OPERATOR_ARG" =~ ^[1-4]$ ]]; then
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
        if [[ -z "$UC_TAG" ]]; then
          UC_TAG="latest-alphanet"
        fi
      elif [[ "$ENVIRONMENT" == "local" ]]; then
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
    --yes|-y)
      AUTO_CONFIRM=true
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

# Validate --op flag usage
if [[ "$ENVIRONMENT" == "local" && -n "$OPERATOR_ARG" ]]; then
  echo "Error: --op is not allowed in local environment. All operators will be deployed."
  exit 1
elif [[ "$ENVIRONMENT" == "alphanet" && "${IS_STARTUP_COMMAND}" == false && -n "$OPERATOR_ARG" ]]; then
  echo "Error: --op can only be used with startup commands (up, restart, start, create)."
  echo "For other commands, the script will target the operator on this host automatically."
  exit 1
elif [[ "$ENVIRONMENT" == "alphanet" && "${IS_STARTUP_COMMAND}" == true && -z "$OPERATOR_ARG" ]]; then
  echo "Error: --op <ID> is required when using --env alphanet with startup commands (up, restart, start, create)."
  echo "Run '$0 --help' for usage information."
  exit 1
fi

# Set OPERATORS_TO_RUN based on environment
if [[ "$ENVIRONMENT" == "alphanet" && "${IS_STARTUP_COMMAND}" == true ]]; then
  # Alphanet startup: use the single operator from --op
  echo "You are about to start operator ${OPERATOR_ARG} on alphanet."
  read -p "Is this correct? (yes/no): " confirmation

  if [[ "$confirmation" != "yes" ]]; then
    echo "Aborted."
    exit 1
  fi

  OPERATORS_TO_RUN=("$OPERATOR_ARG")
elif [[ "$ENVIRONMENT" == "local" ]]; then
  # Local: run all operators
  OPERATORS_TO_RUN=(1 2 3 4)
fi

# If requested, clean operator stacks regardless of the main command
if [[ "${FRESH}" == true ]]; then
  echo "WARNING: --fresh will tear down operators and DELETE ALL VOLUMES (including data)."
  if [[ "${AUTO_CONFIRM}" != true ]]; then
    read -p "Are you sure you want to continue? (yes/no): " confirmation
    if [[ "$confirmation" != "yes" ]]; then
      echo "Aborted."
      exit 1
    fi
  fi

  if [[ "$ENVIRONMENT" == "local" ]]; then
    echo "Cleaning operator stacks (down --volumes)..."
    for op_num in "${OPERATORS_TO_RUN[@]}"; do
      cmd="docker compose -p op_${op_num} --env-file ${ENV_FILE} down --volumes"
      echo "Running: ${cmd}"
      eval "${cmd}"
    done
  else
    # alphanet always uses union-operator project name
    echo "Cleaning operator stack (down --volumes) for project union-operator..."
    cmd="docker compose -p union-operator --env-file ${ENV_FILE} down --volumes"
    echo "Running: ${cmd}"
    eval "${cmd}"
  fi
fi

if [[ "${IS_STARTUP_COMMAND}" == true ]]; then
  bootstrap_broker_keystore

  # Prompt for USER_BITCOIN_WIF if running startup commands if not present on env
  if [[ -z "${USER_BITCOIN_WIF}" ]]; then
    echo "Please enter USER_BITCOIN_WIF (input will be hidden):"
    read -s USER_BITCOIN_WIF
    echo ""

    if [[ -z "${USER_BITCOIN_WIF}" ]]; then
      echo "Error: USER_BITCOIN_WIF is required for 'up' or 'restart' commands."
      exit 1
    fi
  fi
fi

run_local_operators() {
  # LOCAL ENVIRONMENT: All 4 operators on one host
  # Each operator uses different ports to avoid conflicts

  # Create shared network for P2P communication between operators if missing
  local NETWORK_NAME="bitvmx-shared-network"
  if ! docker network inspect $NETWORK_NAME >/dev/null 2>&1; then
    echo "Creating docker network '$NETWORK_NAME'..."
    docker network create --driver bridge --subnet=172.20.0.0/16 $NETWORK_NAME
  fi
  
  local USER_API_PORTS=(40001 40002 40003 40004)
  local BITVMX_PORTS=(22222 33333 44444 55554)
  # should match docker/bitvmx-client/config/local/broker/config/peers.yaml
  local BITVMX_P2P_HOSTS=("172.20.0.11" "172.20.0.12" "172.20.0.13" "172.20.0.14")
  local CLIENT_OPS=("op_1" "op_2" "op_3" "op_4")
  local COMPOSE_FILE_ARG="-f docker-compose.yml -f docker-compose.all.yml"

  for op_num in "${OPERATORS_TO_RUN[@]}"; do
    local i=$((op_num - 1))
    local USER_API_PORT=${USER_API_PORTS[$i]}
    local BITVMX_PORT=${BITVMX_PORTS[$i]}
    local BITVMX_P2P_HOST=${BITVMX_P2P_HOSTS[$i]}
    local CLIENT_OP=${CLIENT_OPS[$i]}

    if [[ "${IS_STARTUP_COMMAND}" == true ]]; then
      sync_broker_key_to_project_volume "op_${op_num}"
    fi

    local BROKER_PUBKEY_HASH
    BROKER_PUBKEY_HASH="$(get_bootstrapped_broker_pubkey_hash)"

    local DOCKER_CMD="BASE_STORAGE_PATH=${BASE_STORAGE_PATH} BROKER_PUBKEY_HASH=${BROKER_PUBKEY_HASH} USER_BITCOIN_WIF=${USER_BITCOIN_WIF} USER_API_PORT=${USER_API_PORT} BITVMX_PORT=${BITVMX_PORT} BITVMX_P2P_HOST=${BITVMX_P2P_HOST} CLIENT_OP=${CLIENT_OP} UC_TAG=${UC_TAG} docker compose ${COMPOSE_FILE_ARG} -p op_${op_num} --env-file ${ENV_FILE} ${DOCKER_COMPOSE_ARGS[*]}"

    echo
    echo "Starting operator ${op_num} with command: '$(echo "${DOCKER_CMD}" | sed "s/USER_BITCOIN_WIF=[^ ]*/USER_BITCOIN_WIF=******/")'"
    eval "${DOCKER_CMD}"
  done
}

run_alphanet_operators() {
  # ALPHANET ENVIRONMENT: Each operator on separate host

  local ALPHANET_PROJECT_NAME="-p union-operator"
  local COMPOSE_FILE_ARG="-f docker-compose.yml -f docker-compose.one.yml"

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

  if [[ "${IS_STARTUP_COMMAND}" == true ]]; then
    sync_broker_key_to_project_volume "union-operator"
  fi

  local BROKER_PUBKEY_HASH
  BROKER_PUBKEY_HASH="$(get_bootstrapped_broker_pubkey_hash)"

  local DOCKER_CMD="BASE_STORAGE_PATH=${BASE_STORAGE_PATH} BROKER_PUBKEY_HASH=${BROKER_PUBKEY_HASH} USER_BITCOIN_WIF=${USER_BITCOIN_WIF} CLIENT_OP=${CLIENT_OP} UC_TAG=${UC_TAG} docker compose ${ALPHANET_PROJECT_NAME} ${COMPOSE_FILE_ARG} --env-file ${ENV_FILE} ${DOCKER_COMPOSE_ARGS[*]}"
  echo "'$(echo "${DOCKER_CMD}" | sed "s/USER_BITCOIN_WIF=[^ ]*/USER_BITCOIN_WIF=******/")'"
  eval "${DOCKER_CMD}"
}

# Run operators based on environment
if [[ "$ENVIRONMENT" == "local" ]]; then
  run_local_operators
else
  run_alphanet_operators
fi