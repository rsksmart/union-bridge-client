#!/usr/bin/env bash

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

# Change to script directory early so all docker compose calls find the correct compose files
cd "${SCRIPT_DIR}" || {
  echo "Error: Failed to change to script directory: ${SCRIPT_DIR}"
  exit 1
}

# Parse UC_* variables from root .envrc if not already set in the environment.
# Only reads `export UC_...=` lines — does not execute arbitrary shell code.
PROJECT_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"
ENVRC_FILE="${PROJECT_ROOT}/.envrc"
if [[ -f "$ENVRC_FILE" ]]; then
  while IFS='=' read -r key value; do
    key=$(echo "$key" | xargs)
    value=$(echo "$value" | sed 's/^["'\''"]//;s/["'\''"]$//')
    if [[ -z "${!key:-}" ]]; then
      export "$key=$value"
    fi
  done < <(grep -E '^\s*export\s+UC_[A-Z_]+=' "$ENVRC_FILE" | sed 's/^\s*export\s*//')
fi

# Initialize from environment variables (can be overridden by command line args)
# Note: UC_TAG, UC_OPERATOR_ID, UC_OPERATOR_ROLE are loaded from root .envrc above
# Use UC_TAG from .envrc if available, can be overridden by --tag flag
UC_TAG="${UC_TAG:-}"
DOCKER_COMPOSE_ARGS=()
NUM_OPERATORS=""
OPS_EXPLICITLY_PROVIDED=false
# Track if --op was explicitly provided (vs loaded from .envrc)
OP_EXPLICITLY_PROVIDED=false
# Track if --tag was explicitly provided (vs loaded from .envrc or .env file)
TAG_EXPLICITLY_PROVIDED=false
# Use UC_OPERATOR_ID from .envrc if available, can be overridden by --op flag
OPERATOR_ARG="${UC_OPERATOR_ID:-}"
ENVIRONMENT="${UC_ENV:-}"
AUTO_CONFIRM=false
FRESH=false

# Display help message
print_help() {
  echo "Usage: $0 [--env <ENV>] [--op <ID>] [OPTIONS] [DOCKER_COMPOSE_ARGS...]"
  echo ""
  echo "Environment:"
  echo "  --env <ENV>              Target environment: alphanet, testnet, local, local-docker, or regtest"
  echo "                            Falls back to UC_ENV from .envrc if not provided."
  echo "                            local, local-docker, regtest deploy 4 operators by default (see --ops)."
  echo "                            alphanet, testnet deploy 1 operator per host (requires --op <ID>)."
  echo ""
  echo "Options:"
  echo "  --op <ID>                Operator ID (1-10) - required for alphanet/testnet startup"
  echo "                            (Optional if UC_OPERATOR_ID is set in .envrc)"
  echo "  --ops <N>                Number of operators to start (1-10) for local/regtest (default: 4)"
  echo "  --tag <TAG>              Docker image tag for Union Client"
  echo "                            (Optional if UC_TAG is set in .envrc)"
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
  echo "    - Config: bitvmx-client/config/local/client/config/op_X.yaml"
  echo "    - Uses bridge network (bitvmx-shared-network) for P2P communication"
  echo "    - Project name: op_1, op_2, op_3 & op_4"
  echo "  Alphanet:"
  echo "    - Runs one operator per host (testnet_op_X where X is from --op)"
  echo "    - Config: bitvmx-client/config/alphanet/client/config/testnet_op_X.yaml"
  echo "    - Uses host network mode for P2P connectivity across physical machines"
  echo "    - Project name: union-operator"
  echo "  Testnet:"
  echo "    - Runs one operator per host (testnet_op_X where X is from --op)"
  echo "    - Config: bitvmx-client/config/testnet/client/config/testnet_op_X.yaml"
  echo "    - Uses host network mode for P2P connectivity across physical machines"
  echo "    - Project name: union-operator"
  echo "  Regtest:"
  echo "    - Runs all 4 operators on one host (op_1, op_2, op_3, op_4)"
  echo "    - Config: bitvmx-client/config/regtest/client/config/op_X.yaml"
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
  echo "  Values can be set in .envrc (root directory) to avoid passing flags:"
  echo "    export UC_ENV=\"local-docker\"        # Sets default environment"
  echo "    export UC_OPERATOR_ID=1              # Sets default operator ID"
  echo "    export UC_TAG=\"latest-anvil\"        # Sets default image tag"
  echo "  Command-line flags override .envrc values if provided."
  echo ""
  echo "Examples:"
  echo "  $0 --env local up -d                                     # Start 4 operators locally (default)"
  echo "  $0 --env local --ops 10 up -d                            # Start all 10 operators locally"
  echo "  $0 up -d                                                  # Same as above, if UC_ENV=local in .envrc"
  echo "  $0 --env local --fresh up -d                             # Clean and start operators locally"
  echo "  $0 --env local --fresh --yes up -d                       # Clean and start operators locally, no confirmation prompt"
  echo "  $0 --env local down                                      # Stop all local operators"
  echo "  $0 --env regtest up -d                                   # Start all 4 operators in regtest mode"
  echo "  $0 --env regtest --ops 6 up -d                           # Start 6 operators in regtest mode"
  echo "  $0 --env regtest --fresh up -d                           # Clean and start all operators in regtest mode"
  echo "  $0 --env regtest down                                    # Stop all regtest operators"
  echo "  $0 --env alphanet --op 1 up -d                           # Start operator 1 in alphanet"
  echo "  $0 --env alphanet up -d                                  # Same, if UC_OPERATOR_ID=1 in .envrc"
  echo "  $0 --env alphanet --op 2 up -d                           # Start operator 2 in alphanet"
  echo "  $0 --env alphanet --op 1 --tag latest-alphanet up -d     # Start operator 1 with specific tag"
  echo "  $0 --env alphanet down --volumes                         # Stop operator on this alphanet host"
  echo "  $0 --env alphanet logs -f                                # View logs for operator on this host"
  echo "  $0 --env testnet --op 1 up -d                            # Start operator 1 in testnet"
  echo "  $0 --env testnet --op 2 up -d                            # Start operator 2 in testnet"
  echo "  $0 --env testnet --op 1 --tag latest-testnet up -d       # Start operator 1 with specific tag"
  echo "  $0 --env testnet down --volumes                          # Stop operator on this testnet host"
  echo "  $0 --env testnet logs -f                                 # View logs for operator on this host"
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
      OP_EXPLICITLY_PROVIDED=true
      if ! [[ "$OPERATOR_ARG" =~ ^(10|[1-9])$ ]]; then
        echo "Error: --op must be between 1 and 10"
        exit 1
      fi
      # Explicit --op overrides any UC_OPERATOR_ID from .envrc
      shift 2
      ;;
    --ops)
      NUM_OPERATORS="$2"
      OPS_EXPLICITLY_PROVIDED=true
      if ! [[ "$NUM_OPERATORS" =~ ^(10|[1-9])$ ]]; then
        echo "Error: --ops must be between 1 and 10"
        exit 1
      fi
      shift 2
      ;;
    --tag)
      UC_TAG="$2"
      TAG_EXPLICITLY_PROVIDED=true
      # Explicit --tag overrides any UC_TAG from .envrc or .env file
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
    echo "Alternatively, set UC_ENV in .envrc (root directory) to avoid passing the flag."
    echo "Run '$0 --help' for usage information."
    exit 1
  fi
fi

# Map local-docker to local (they use the same configuration)
if [[ "$ENVIRONMENT" == "local-docker" ]]; then
  ENVIRONMENT="local"
fi

# Function to restore UC_TAG based on precedence: --tag flag > .envrc > .env file > default
# Args: $1 = default tag value for the environment
restore_uc_tag() {
  local default_tag="$1"
  if [[ "${TAG_EXPLICITLY_PROVIDED}" == true ]]; then
    UC_TAG="${SAVED_UC_TAG_FROM_FLAG}"
  elif [[ -n "${SAVED_UC_TAG_FROM_ENVRC}" ]]; then
    UC_TAG="${SAVED_UC_TAG_FROM_ENVRC}"
  elif [[ -z "$UC_TAG" ]]; then
    UC_TAG="${default_tag}"
  fi
}

# Set ENV_FILE and load environment-specific variables (for other vars like BITCOIND_URL, etc.)
# UC_TAG, UC_OPERATOR_ID, UC_OPERATOR_ROLE are already loaded from .envrc above
# Save UC_TAG values before .env file might overwrite them
# Precedence: --tag flag > UC_TAG from .envrc > UC_TAG from .env file > default
SAVED_UC_TAG_FROM_FLAG=""
SAVED_UC_TAG_FROM_ENVRC=""
if [[ "${TAG_EXPLICITLY_PROVIDED}" == true ]]; then
  # Save the --tag flag value
  SAVED_UC_TAG_FROM_FLAG="${UC_TAG}"
elif [[ -n "${UC_TAG}" ]]; then
  # Save UC_TAG from .envrc if it was set (and --tag wasn't provided)
  SAVED_UC_TAG_FROM_ENVRC="${UC_TAG}"
fi
# Save --ops value before .env file might overwrite NUM_OPERATORS
SAVED_NUM_OPERATORS="${NUM_OPERATORS}"

if [[ "$ENVIRONMENT" == "alphanet" ]]; then
  ENV_FILE="${SCRIPT_DIR}/.env.alphanet"
  if [[ -f "$ENV_FILE" ]]; then
    set -a
    source "$ENV_FILE"
    set +a
  fi
  restore_uc_tag "latest-alphanet"
elif [[ "$ENVIRONMENT" == "testnet" ]]; then
  ENV_FILE="${SCRIPT_DIR}/.env.testnet"
  if [[ -f "$ENV_FILE" ]]; then
    set -a
    source "$ENV_FILE"
    set +a
  fi
  restore_uc_tag "latest-testnet"
elif [[ "$ENVIRONMENT" == "local" ]]; then
  ENV_FILE="${SCRIPT_DIR}/.env.local"
  if [[ -f "$ENV_FILE" ]]; then
    set -a
    source "$ENV_FILE"
    set +a
  fi
  restore_uc_tag "latest-anvil"
elif [[ "$ENVIRONMENT" == "regtest" ]]; then
  ENV_FILE="${SCRIPT_DIR}/.env.regtest"
  if [[ -f "$ENV_FILE" ]]; then
    set -a
    source "$ENV_FILE"
    set +a
  fi
  restore_uc_tag "latest-regtest"
else
  echo "Invalid environment. Use 'alphanet', 'testnet', 'local', 'local-docker', or 'regtest'"
  exit 1
fi

# Restore --ops flag value if it was explicitly provided (overrides .env file)
if [[ "${OPS_EXPLICITLY_PROVIDED}" == true ]]; then
  NUM_OPERATORS="${SAVED_NUM_OPERATORS}"
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

# Validate --op flag usage
# Consolidated validation logic for operator ID across all environments
if [[ ("$ENVIRONMENT" == "local" || "$ENVIRONMENT" == "regtest") && -n "$OPERATOR_ARG" ]]; then
  echo "Error: --op is not allowed in ${ENVIRONMENT} environment. All operators will be deployed."
  exit 1
elif [[ ("$ENVIRONMENT" == "alphanet" || "$ENVIRONMENT" == "testnet") && "${IS_STARTUP_COMMAND}" == false ]]; then
  # For non-startup commands on alphanet/testnet, clear OPERATOR_ARG if it came from .envrc
  # (the script will automatically target the operator on this host)
  if [[ "${OP_EXPLICITLY_PROVIDED}" == true ]]; then
    echo "Error: --op can only be used with startup commands (up, restart, start, create)."
    echo "For other commands, the script will target the operator on this host automatically."
    exit 1
  fi
  # Clear OPERATOR_ARG for non-startup commands (it came from .envrc, not explicit --op)
  OPERATOR_ARG=""
elif [[ ("$ENVIRONMENT" == "alphanet" || "$ENVIRONMENT" == "testnet") && "${IS_STARTUP_COMMAND}" == true && -z "$OPERATOR_ARG" ]]; then
  echo "Error: --op <ID> is required when using --env ${ENVIRONMENT} with startup commands (up, restart, start, create)."
  echo "Alternatively, set UC_OPERATOR_ID in .envrc (root directory) to avoid passing the flag."
  echo "Run '$0 --help' for usage information."
  exit 1
fi

# Set OPERATORS_TO_RUN based on environment
if [[ ("$ENVIRONMENT" == "alphanet" || "$ENVIRONMENT" == "testnet") && "${IS_STARTUP_COMMAND}" == true ]]; then
  # Alphanet/Testnet startup: use the single operator from --op
  echo "You are about to start operator ${OPERATOR_ARG} on ${ENVIRONMENT}."
  read -p "Is this correct? (yes/no): " confirmation

  if [[ "$confirmation" != "yes" ]]; then
    echo "Aborted."
    exit 1
  fi

  OPERATORS_TO_RUN=("$OPERATOR_ARG")
elif [[ "$ENVIRONMENT" == "local" || "$ENVIRONMENT" == "regtest" ]]; then
  # Local/Regtest: default to 4 operators, overridable with --ops
  OPERATORS_TO_RUN=($(seq 1 "${NUM_OPERATORS:-4}"))
fi

# If requested, clean operator stacks regardless of the main command
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
      cmd="docker compose -p op_${op_num} --env-file ${ENV_FILE} down --volumes"
      echo "Running: ${cmd}"
      eval "${cmd}"
    done
  else
    # alphanet/testnet always use union-operator project name
    echo "Cleaning operator stack (down --volumes) for project union-operator..."
    cmd="docker compose -p union-operator --env-file ${ENV_FILE} down --volumes"
    echo "Running: ${cmd}"
    eval "${cmd}"
  fi
fi

if [[ "${IS_STARTUP_COMMAND}" == true ]]; then
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

sync_regtest_bitvmx_heights() {
  local cfg_dir="${SCRIPT_DIR}/../bitvmx-client/config/regtest/client/config"
  local sample_cfg="${cfg_dir}/op_1.yaml"
  local height_delta="${REGTEST_BITVMX_HEIGHT_DELTA:-10}"
  local rpc_payload='{"jsonrpc":"1.0","id":"ub","method":"getblockcount","params":[]}'

  if ! command -v curl >/dev/null 2>&1; then
    echo "Error: curl is required to auto-sync regtest checkpoint/start heights."
    exit 1
  fi

  if ! command -v jq >/dev/null 2>&1; then
    echo "Error: jq is required to auto-sync regtest checkpoint/start heights."
    exit 1
  fi

  if [[ ! -f "${sample_cfg}" ]]; then
    echo "Error: missing regtest BitVMX config file: ${sample_cfg}"
    exit 1
  fi

  local bitcoin_rpc_url
  bitcoin_rpc_url="$(
    awk '/^[[:space:]]*url:[[:space:]]*/ {print $2; exit}' "${sample_cfg}" | tr -d "\"'"
  )"
  if [[ -z "${bitcoin_rpc_url}" ]]; then
    echo "Error: could not parse Bitcoin RPC URL from ${sample_cfg}"
    exit 1
  fi

  local rpc_response block_height start_height timestamp
  rpc_response="$(
    curl -sS --max-time 10 -H 'content-type:text/plain' \
      --data-binary "${rpc_payload}" \
      "${bitcoin_rpc_url}"
  )"
  block_height="$(echo "${rpc_response}" | jq -r '.result // empty')"
  if ! [[ "${block_height}" =~ ^[0-9]+$ ]]; then
    echo "Error: failed to read Bitcoin block height from ${bitcoin_rpc_url}. Response: ${rpc_response}"
    exit 1
  fi

  start_height=$((block_height - height_delta))
  if ((start_height < 1)); then
    start_height=1
  fi

  timestamp="$(date -u +%Y%m%dT%H%M%SZ)"
  for op_num in "${OPERATORS_TO_RUN[@]}"; do
    local cfg_file backup_file
    cfg_file="${cfg_dir}/op_${op_num}.yaml"
    backup_file="${cfg_file}.${timestamp}.bak"

    if [[ ! -f "${cfg_file}" ]]; then
      echo "Error: missing operator config file: ${cfg_file}"
      exit 1
    fi

    cp "${cfg_file}" "${backup_file}"
    UB_START_HEIGHT="${start_height}" perl -0777 -i -pe 's/(checkpoint_height:\s*)\d+/$1$ENV{UB_START_HEIGHT}/g; s/(start_height:\s*)\d+/$1$ENV{UB_START_HEIGHT}/g' "${cfg_file}"
  done

  echo "Regtest BitVMX heights synchronized: btc_height=${block_height}, start_height=${start_height}, delta=${height_delta}."
}

resolve_regtest_check_fork_elf_path() {
  local configured_path="${UB_CHECK_FORK_GUEST_ELF_PATH:-}"
  local default_path
  default_path="$(cd "${SCRIPT_DIR}/.." && pwd)/bitvmx-client/config/regtest/client/config/check-fork-guest.bin"

  if [[ -z "${configured_path}" || "${configured_path}" == "/app/config/check-fork-guest.bin" ]]; then
    configured_path="${default_path}"
  fi

  if [[ ! -f "${configured_path}" ]]; then
    echo "Error: missing CheckFork guest ELF for regtest dispatcher at ${configured_path}"
    exit 1
  fi

  echo "${configured_path}"
}

if [[ "${ENVIRONMENT}" == "regtest" && "${IS_STARTUP_COMMAND}" == true ]]; then
  sync_regtest_bitvmx_heights
fi

run_all_operators() {
  # LOCAL/REGTEST ENVIRONMENT: Multiple operators on one host (default 4, up to 10)
  # Each operator uses different ports to avoid conflicts

  # Create shared network for P2P communication between operators if missing
  local NETWORK_NAME="bitvmx-shared-network"
  if ! docker network inspect $NETWORK_NAME >/dev/null 2>&1; then
    echo "Creating docker network '$NETWORK_NAME'..."
    docker network create --driver bridge --subnet=172.20.0.0/16 $NETWORK_NAME
  fi
  
  local USER_API_PORTS=(40001 40002 40003 40004 40005 40006 40007 40008 40009 40010)
  local BITVMX_PORTS=(22222 33333 44444 55554 55555 55556 55557 55558 55559 55560)
  # should match docker/bitvmx-client/config/local/broker/config/peers.yaml
  local BITVMX_P2P_HOSTS=("172.20.0.11" "172.20.0.12" "172.20.0.13" "172.20.0.14" "172.20.0.15" "172.20.0.16" "172.20.0.17" "172.20.0.18" "172.20.0.19" "172.20.0.20")
  local CLIENT_OPS=("op_1" "op_2" "op_3" "op_4" "op_5" "op_6" "op_7" "op_8" "op_9" "op_10")
  local COMPOSE_FILE_ARG="-f docker-compose.yml -f docker-compose.all.yml"
  local regtest_check_fork_elf_path=""

  if [[ "${ENVIRONMENT}" == "regtest" ]]; then
    regtest_check_fork_elf_path="$(resolve_regtest_check_fork_elf_path)"
    echo "Using regtest CheckFork guest ELF path for host dispatcher: ${regtest_check_fork_elf_path}"
  fi

  for op_num in "${OPERATORS_TO_RUN[@]}"; do
    local i=$((op_num - 1))
    local USER_API_PORT=${USER_API_PORTS[$i]}
    local BITVMX_PORT=${BITVMX_PORTS[$i]}
    local BITVMX_P2P_HOST=${BITVMX_P2P_HOSTS[$i]}
    local CLIENT_OP=${CLIENT_OPS[$i]}
    local extra_env=""

    if [[ "${ENVIRONMENT}" == "regtest" ]]; then
      extra_env="UB_CHECK_FORK_GUEST_ELF_PATH=${regtest_check_fork_elf_path}"
    fi

    local DOCKER_CMD="CONFIG_DIR=${CONFIG_DIR} USER_BITCOIN_WIF=${USER_BITCOIN_WIF} USER_API_PORT=${USER_API_PORT} BITVMX_PORT=${BITVMX_PORT} BITVMX_P2P_HOST=${BITVMX_P2P_HOST} CLIENT_OP=${CLIENT_OP} UC_TAG=${UC_TAG} ${extra_env:+${extra_env} }docker compose ${COMPOSE_FILE_ARG} -p op_${op_num} --env-file ${ENV_FILE} ${DOCKER_COMPOSE_ARGS[*]}"

    echo
    echo "Starting operator ${op_num} with command: '$(echo "${DOCKER_CMD}" | sed "s/USER_BITCOIN_WIF=[^ ]*/USER_BITCOIN_WIF=******/")'"
    eval "${DOCKER_CMD}"
  done
}

run_default_operator() {
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

  local DOCKER_CMD="CONFIG_DIR=${CONFIG_DIR} USER_BITCOIN_WIF=${USER_BITCOIN_WIF} CLIENT_OP=${CLIENT_OP} UC_TAG=${UC_TAG} docker compose ${ALPHANET_PROJECT_NAME} ${COMPOSE_FILE_ARG} --env-file ${ENV_FILE} ${DOCKER_COMPOSE_ARGS[*]}"
  echo "'$(echo "${DOCKER_CMD}" | sed "s/USER_BITCOIN_WIF=[^ ]*/USER_BITCOIN_WIF=******/")'"
  eval "${DOCKER_CMD}"
}

run_testnet_operators() {
  # TESTNET ENVIRONMENT: Each operator on separate host (same as alphanet)

  local TESTNET_PROJECT_NAME="-p union-operator"
  local COMPOSE_FILE_ARG="-f docker-compose.yml -f docker-compose.op_one.yml"

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
    echo "Running command on testnet operator:"
  fi

  local DOCKER_CMD="CONFIG_DIR=${CONFIG_DIR} USER_BITCOIN_WIF=${USER_BITCOIN_WIF} CLIENT_OP=${CLIENT_OP} UC_TAG=${UC_TAG} docker compose ${TESTNET_PROJECT_NAME} ${COMPOSE_FILE_ARG} --env-file ${ENV_FILE} ${DOCKER_COMPOSE_ARGS[*]}"
  echo "'$(echo "${DOCKER_CMD}" | sed "s/USER_BITCOIN_WIF=[^ ]*/USER_BITCOIN_WIF=******/")'"
  eval "${DOCKER_CMD}"
}

# Set CONFIG_DIR to absolute path for robust volume mounting
# This ensures the config directory is accessible regardless of where docker-compose is run from
CONFIG_DIR="${PROJECT_ROOT}/config"
export CONFIG_DIR

# Verify CONFIG_DIR exists
if [[ ! -d "${CONFIG_DIR}" ]]; then
  echo "Error: Config directory not found: ${CONFIG_DIR}"
  echo "Please ensure the config directory exists at the project root."
  exit 1
fi

# Run operators based on environment
if [[ "$ENVIRONMENT" == "local" || "$ENVIRONMENT" == "regtest" ]]; then
  run_all_operators
elif [[ "$ENVIRONMENT" == "testnet" ]]; then
  run_testnet_operators
else
  run_default_operator
fi
