#!/usr/bin/env bash

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

# Change to script directory early so all docker compose calls find the correct compose files
cd "${SCRIPT_DIR}" || {
  echo "Error: Failed to change to script directory: ${SCRIPT_DIR}"
  exit 1
}

# Initialize from environment variables (can be overridden by command line args)
DOCKER_COMPOSE_ARGS=()
NUM_OPERATORS=""
OPERATOR_ARG="${UC_OPERATOR_ID:-}"
ENVIRONMENT="${UC_ENV:-}"
AUTO_CONFIRM=false
FRESH=false
BASE_STORAGE_PATH="${BASE_STORAGE_PATH:-$HOME}"

# Display help message
print_help() {
  echo "Usage: $0 [--env <ENV>] [--op <ID>] [OPTIONS] [DOCKER_COMPOSE_ARGS...]"
  echo ""
  echo "Before startup, prepare the operator env files on this host:"
  echo "  <project_root>/cli-setup-operators.sh --env local --ops 4"
  echo "  <project_root>/cli-setup-operators.sh --env alphanet --op 1"
  echo ""
  echo "Environment:"
  echo "  --env <ENV>              Target environment: alphanet, testnet, local, local-docker, or regtest"
  echo "                            Falls back to exported UC_ENV if not provided."
  echo "                            local, local-docker, regtest deploy 4 operators by default (see --ops)."
  echo "                            alphanet, testnet deploy 1 operator per host (requires --op <ID>)."
  echo ""
  echo "Options:"
  echo "  --op <ID>                Operator ID (1-10) - required for alphanet/testnet commands"
  echo "                            (Optional if UC_OPERATOR_ID is exported in the shell)"
  echo "  --ops <N>                Number of operators to start (1-10) for local/regtest (default: 4)"
  echo "  --tag <TAG>              Override UC_TAG for this docker compose invocation only"
  echo "  --help                   Display this help message"
  echo "  --fresh                  Tear down operators (and volumes) before running the command"
  echo "                           - Includes confirmation prompt to prevent accidental data loss in local"
  echo "                           - No confirmation prompt in regtest"
  echo "                           - In regtest startup, auto-syncs BitVMX checkpoint/start heights to current BTC height"
  echo "                           - Clears all operator state and databases"
  echo "                           - Only allowed with --env local or --env regtest"
  echo "  --yes, -y                Automatic yes to fresh confirmation prompt (use with caution)"
  echo ""
  echo "Environment Details:"
  echo "  Local:"
  echo "    - Runs operators on one host (default: 4, up to 10 with --ops)"
  echo "    - BitVMX config: \${BASE_STORAGE_PATH:-\$HOME}/.union_bridge/op_X/bitvmx/op_X.yaml"
  echo "    - Uses bridge network (bitvmx-shared-network) for P2P communication"
  echo "    - Project name: op_1, op_2, op_3 & op_4"
  echo "  Alphanet:"
  echo "    - Runs one operator per host (testnet_op_X where X is from --op)"
  echo "    - BitVMX config: \${BASE_STORAGE_PATH:-\$HOME}/.union_bridge/op_X/bitvmx/testnet_op_X.yaml"
  echo "    - Uses host network mode for P2P connectivity across physical machines"
  echo "    - Project name: union-operator"
  echo "  Testnet:"
  echo "    - Runs one operator per host (testnet_op_X where X is from --op)"
  echo "    - BitVMX config: \${BASE_STORAGE_PATH:-\$HOME}/.union_bridge/op_X/bitvmx/testnet_op_X.yaml"
  echo "    - Uses host network mode for P2P connectivity across physical machines"
  echo "    - Project name: union-operator"
  echo "  Regtest:"
  echo "    - Runs all 4 operators on one host (op_1, op_2, op_3, op_4)"
  echo "    - BitVMX config: \${BASE_STORAGE_PATH:-\$HOME}/.union_bridge/op_X/bitvmx/op_X.yaml"
  echo "    - Uses bridge network (bitvmx-shared-network) for P2P communication"
  echo "    - Project name: op_1, op_2, op_3 & op_4"
  echo ""
  echo "Common Docker Compose Arguments can be used. Examples:"
  echo "  up                       Create and start containers"
  echo "  down [--volumes]         Stop and remove containers, networks (add --volumes to also remove named/anonymous volumes)"
  echo "  ps                       List containers"
  echo "  logs                     View output from containers"
  echo "  --force-recreate         Recreate containers even if configuration and image haven't changed"
  echo "  Note: Building from source is not supported. Only registry images should be used."
  echo ""
  echo "Configuration:"
  echo "  Values can be exported in the shell to avoid passing flags:"
  echo "    export UC_ENV=\"local-docker\"        # Sets default environment"
  echo "    export UC_OPERATOR_ID=1              # Sets default operator ID"
  echo "    export BITCOIND_URL=\"http://user:password@bitcoin-node:18332\""
  echo "                                       # Required before cli-setup-operators.sh and operator startup"
  echo "  Command-line flags override exported values when provided."
  echo ""
  echo "Examples:"
  echo "  $0 --env local up -d                                     # Start 4 operators locally (default)"
  echo "  $0 --env local --ops 10 up -d                            # Start all 10 operators locally"
  echo "  $0 up -d                                                  # Same as above, if UC_ENV=local is exported"
  echo "  $0 --env local --fresh up -d                             # Clean and start operators locally"
  echo "  $0 --env local --fresh --yes up -d                       # Clean and start operators locally, no confirmation prompt"
  echo "  $0 --env local --tag latest-anvil up -d                  # Start local operators with an explicit image tag"
  echo "  $0 --env local down                                      # Stop all local operators"
  echo "  $0 --env regtest up -d                                   # Start all 4 operators in regtest mode"
  echo "  $0 --env regtest --ops 6 up -d                           # Start 6 operators in regtest mode"
  echo "  $0 --env regtest --fresh up -d                           # Clean and start all operators in regtest mode"
  echo "  $0 --env regtest down                                    # Stop all regtest operators"
  echo "  $0 --env alphanet --op 1 up -d                           # Start operator 1 in alphanet"
  echo "  $0 --env alphanet up -d                                  # Same, if UC_OPERATOR_ID=1 is exported"
  echo "  $0 --env alphanet --op 2 up -d                           # Start operator 2 in alphanet"
  echo "  $0 --env alphanet down --volumes                         # Stop operator on this alphanet host"
  echo "  $0 --env alphanet logs -f                                # View logs for operator on this host"
  echo "  $0 --env testnet --op 1 up -d                            # Start operator 1 in testnet"
  echo "  $0 --env testnet --op 2 up -d                            # Start operator 2 in testnet"
  echo "  $0 --env testnet down --volumes                          # Stop operator on this testnet host"
  echo "  $0 --env testnet logs -f                                 # View logs for operator on this host"
  echo ""
  echo "Any additional arguments will be passed directly to docker compose."
  exit 0
}

read_env_value() {
  local env_file="$1"
  local key="$2"

  awk -F= -v key="${key}" '$1 == key {print substr($0, index($0, "=") + 1); exit}' "${env_file}"
}

require_key_store_password() {
  local env_file="$1"
  local configured_password="${KEY_STORE_PASSWORD:-}"

  if [[ -z "${configured_password}" ]] && [[ -f "${env_file}" ]]; then
    configured_password="$(read_env_value "${env_file}" "KEY_STORE_PASSWORD")"
  fi

  if [[ -z "${configured_password}" ]]; then
    echo "Error: KEY_STORE_PASSWORD is required for operator startup." >&2
    echo "Export KEY_STORE_PASSWORD or define it in ${env_file} before running startup commands." >&2
    exit 1
  fi
}

# Parse args
while [[ $# -gt 0 ]]; do
  case $1 in
    --help)
      print_help
      ;;
    --op)
      OPERATOR_ARG="$2"
      if ! [[ "$OPERATOR_ARG" =~ ^(10|[1-9])$ ]]; then
        echo "Error: --op must be between 1 and 10"
        exit 1
      fi
      shift 2
      ;;
    --ops)
      NUM_OPERATORS="$2"
      if ! [[ "$NUM_OPERATORS" =~ ^(10|[1-9])$ ]]; then
        echo "Error: --ops must be between 1 and 10"
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
      # Clear any default from env var when explicitly provided
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

# Use environment variable as default if --env was not provided
if [[ -z "$ENVIRONMENT" ]]; then
  if [[ -n "${UC_ENV:-}" ]]; then
    ENVIRONMENT="${UC_ENV}"
  else
    echo "Error: --env flag is required. Use 'alphanet', 'testnet', 'local', 'local-docker', or 'regtest'."
    echo "Alternatively, export UC_ENV in the shell before running the script."
    echo "Run '$0 --help' for usage information."
    exit 1
  fi
fi

# Map local-docker to local (they use the same configuration)
if [[ "$ENVIRONMENT" == "local-docker" ]]; then
  ENVIRONMENT="local"
fi

# Set the base environment file used by docker compose.
if [[ "$ENVIRONMENT" == "alphanet" ]]; then
  ENV_FILE="${SCRIPT_DIR}/.env.alphanet"
elif [[ "$ENVIRONMENT" == "testnet" ]]; then
  ENV_FILE="${SCRIPT_DIR}/.env.testnet"
elif [[ "$ENVIRONMENT" == "local" ]]; then
  ENV_FILE="${SCRIPT_DIR}/.env.local"
elif [[ "$ENVIRONMENT" == "regtest" ]]; then
  ENV_FILE="${SCRIPT_DIR}/.env.regtest"
else
  echo "Invalid environment. Use 'alphanet', 'testnet', 'local', 'local-docker', or 'regtest'"
  exit 1
fi

# Validate --ops flag usage
if [[ -n "$NUM_OPERATORS" && "$ENVIRONMENT" != "local" && "$ENVIRONMENT" != "regtest" ]]; then
  echo "Error: --ops is only allowed with --env local or --env regtest."
  exit 1
fi

# Validate --fresh flag usage
if [[ "${FRESH}" == true && "$ENVIRONMENT" != "local" && "$ENVIRONMENT" != "regtest" ]]; then
  echo "Error: --fresh is only allowed with --env local or --env regtest."
  echo "For alphanet/testnet, manually tear down the operator if needed."
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

if [[ "${IS_STARTUP_COMMAND}" == true ]]; then
  require_key_store_password "${ENV_FILE}"
fi

# Validate --op flag usage
if [[ ("$ENVIRONMENT" == "local" || "$ENVIRONMENT" == "regtest") && -n "$OPERATOR_ARG" ]]; then
  echo "Error: --op is not allowed in ${ENVIRONMENT} environment. All operators will be deployed."
  exit 1
elif [[ ("$ENVIRONMENT" == "alphanet" || "$ENVIRONMENT" == "testnet") && -z "$OPERATOR_ARG" ]]; then
  echo "Error: --op <ID> is required when using --env ${ENVIRONMENT}."
  echo "Alternatively, export UC_OPERATOR_ID in the shell before running the script."
  echo "Run '$0 --help' for usage information."
  exit 1
fi

# Set OPERATORS_TO_RUN based on environment
if [[ "$ENVIRONMENT" == "alphanet" || "$ENVIRONMENT" == "testnet" ]]; then
  if [[ "${IS_STARTUP_COMMAND}" == true ]]; then
    echo "You are about to start operator ${OPERATOR_ARG} on ${ENVIRONMENT} as ${UC_OPERATOR_ROLE:-undefined}."
    read -p "Is this correct? (yes/no): " confirmation

    if [[ "$confirmation" != "yes" ]]; then
      echo "Aborted."
      exit 1
    fi
  fi
  OPERATORS_TO_RUN=("$OPERATOR_ARG")
elif [[ "$ENVIRONMENT" == "local" || "$ENVIRONMENT" == "regtest" ]]; then
  # Local/Regtest: default to 4 operators, overridable with --ops
  OPERATORS_TO_RUN=($(seq 1 "${NUM_OPERATORS:-4}"))
fi

operator_docker_env_file_path() {
  local op_num="$1"
  echo "${BASE_STORAGE_PATH}/.union_bridge/op_${op_num}/docker.env"
}

require_operator_docker_env_file() {
  local file_path="$1"

  if [[ ! -f "${file_path}" ]]; then
    echo "Error: missing prepared operator env file ${file_path}" >&2
    echo "Run <project_root>/cli-setup-operators.sh for this environment/operator before starting containers." >&2
    return 1
  fi
}

run_compose_stack() {
  local project_name="$1"
  local overlay_compose="$2"
  local description="$3"
  local env_file_path="$4"
  local -a compose_cmd=(
    docker compose
    -p "${project_name}"
    -f docker-compose.yml
    -f "${overlay_compose}"
    --env-file "${ENV_FILE}"
    --env-file "${env_file_path}"
  )

  compose_cmd+=("${DOCKER_COMPOSE_ARGS[@]}")

  echo
  echo "Running ${description} with env file ${env_file_path}"
  if [[ -n "${UC_TAG:-}" ]]; then
    printf "'UC_TAG=%q " "${UC_TAG}"
  else
    printf "'"
  fi
  printf "%q " "${compose_cmd[@]}"
  echo "'"

  if [[ -n "${UC_TAG:-}" ]]; then
    UC_TAG="${UC_TAG}" "${compose_cmd[@]}"
  else
    "${compose_cmd[@]}"
  fi
}

if [[ "${FRESH}" == true ]]; then
  echo "WARNING: --fresh will tear down operators and DELETE ALL VOLUMES (including data)."
  if [[ "$ENVIRONMENT" == "local" && "${AUTO_CONFIRM}" != true ]]; then
    read -p "Are you sure you want to continue? (yes/no): " confirmation
    if [[ "$confirmation" != "yes" ]]; then
      echo "Aborted."
      exit 1
    fi
  elif [[ "$ENVIRONMENT" == "regtest" ]]; then
    echo "Regtest fresh mode enabled: continuing without confirmation prompt."
  fi

  if [[ "$ENVIRONMENT" == "local" || "$ENVIRONMENT" == "regtest" ]]; then
    echo "Cleaning operator stacks (down --volumes)..."
    for op_num in "${OPERATORS_TO_RUN[@]}"; do
      operator_env_file="$(operator_docker_env_file_path "${op_num}")"
      if ! require_operator_docker_env_file "${operator_env_file}"; then
        exit 1
      fi
      docker compose -p "op_${op_num}" -f docker-compose.yml -f docker-compose.all.yml --env-file "${ENV_FILE}" --env-file "${operator_env_file}" down --volumes
    done
  else
    echo "Cleaning operator stack (down --volumes) for project union-operator..."
    operator_env_file="$(operator_docker_env_file_path "${OPERATORS_TO_RUN[0]}")"
    if ! require_operator_docker_env_file "${operator_env_file}"; then
      exit 1
    fi
    docker compose -p union-operator -f docker-compose.yml -f docker-compose.one.yml --env-file "${ENV_FILE}" --env-file "${operator_env_file}" down --volumes
  fi
fi

run_all_operators() {
  # LOCAL/REGTEST ENVIRONMENT: Multiple operators on one host (default 4, up to 10)
  # Each operator uses different ports to avoid conflicts

  # The shared bridge network is only needed when containers are actually
  # being created/started.
  if [[ "${IS_STARTUP_COMMAND}" == true ]]; then
    local NETWORK_NAME="bitvmx-shared-network"
    if ! docker network inspect "${NETWORK_NAME}" >/dev/null 2>&1; then
      echo "Creating docker network '${NETWORK_NAME}'..."
      docker network create --driver bridge --subnet=172.20.0.0/16 "${NETWORK_NAME}"
    fi
  fi
  
  for op_num in "${OPERATORS_TO_RUN[@]}"; do
    local operator_env_file
    operator_env_file="$(operator_docker_env_file_path "${op_num}")"
    if ! require_operator_docker_env_file "${operator_env_file}"; then
      exit 1
    fi
    run_compose_stack "op_${op_num}" "docker-compose.all.yml" "operator ${op_num}" "${operator_env_file}"
  done
}

run_single_operator() {
  # Alphanet/testnet run a single operator per host and share the same
  # one-operator compose overlay.
  local environment_label="$1"
  local op_num=${OPERATORS_TO_RUN[0]}
  local operator_env_file
  operator_env_file="$(operator_docker_env_file_path "${op_num}")"
  if ! require_operator_docker_env_file "${operator_env_file}"; then
    exit 1
  fi
  run_compose_stack "union-operator" "docker-compose.one.yml" "${environment_label} operator ${op_num}" "${operator_env_file}"
}

# Run operators based on environment
if [[ "$ENVIRONMENT" == "local" || "$ENVIRONMENT" == "regtest" ]]; then
  run_all_operators
else
  run_single_operator "${ENVIRONMENT}"
fi
