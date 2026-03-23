#!/usr/bin/env bash

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

cd "${SCRIPT_DIR}" || {
  echo "Error: Failed to change to script directory: ${SCRIPT_DIR}"
  exit 1
}

OPERATOR_ARG="${UC_OPERATOR_ID:-}"
ENVIRONMENT="${UC_ENV:-}"
NUM_OPERATORS=""
USER_BITCOIN_WIF_ARG="${USER_BITCOIN_WIF:-}"
BASE_STORAGE_PATH="${BASE_STORAGE_PATH:-$HOME}"
OPERATORS_TO_RUN=()

print_help() {
  echo "Usage: $0 [--env <ENV>] [--op <ID> | --ops <N>] [--user-bitcoin-wif <WIF>]"
  echo ""
  echo "Creates or reuses host-side Docker operator artifacts:"
  echo "  - broker identities under ${BASE_STORAGE_PATH}/.union_bridge/op_N/broker/<service>.*"
  echo "  - generated operator env files under ${BASE_STORAGE_PATH}/.union_bridge/op_N/docker/<env>.env"
  echo "  Existing operator env files are refreshed in place."
  echo ""
  echo "Options:"
  echo "  --env <ENV>                Target environment: alphanet, testnet, local, local-docker, or regtest"
  echo "  --op <ID>                  Operator ID (1-10) for alphanet/testnet"
  echo "  --ops <N>                  Number of operators to prepare (1-10) for local/regtest"
  echo "  --user-bitcoin-wif <WIF>   User WIF to persist into generated operator env files"
  echo "  --help                     Display this help message"
  exit 0
}

prompt_environment() {
  local response=""

  while [[ -z "${response}" ]]; do
    read -r -p "Environment [local]: " response
    response="${response:-local}"
    case "${response}" in
      alphanet|testnet|local|local-docker|regtest)
        ENVIRONMENT="${response}"
        ;;
      *)
        echo "Error: environment must be one of alphanet, testnet, local, local-docker, or regtest."
        response=""
        ;;
    esac
  done
}

prompt_operator_id() {
  local response=""

  while [[ -z "${response}" ]]; do
    read -r -p "Operator ID (1-10): " response
    if [[ "${response}" =~ ^(10|[1-9])$ ]]; then
      OPERATOR_ARG="${response}"
    else
      echo "Error: operator ID must be between 1 and 10."
      response=""
    fi
  done
}

prompt_num_operators() {
  local response=""

  while [[ -z "${response}" ]]; do
    read -r -p "Number of operators to prepare [4]: " response
    response="${response:-4}"
    if [[ "${response}" =~ ^(10|[1-9])$ ]]; then
      NUM_OPERATORS="${response}"
    else
      echo "Error: number of operators must be between 1 and 10."
      response=""
    fi
  done
}

operator_root_path() {
  local op_num="$1"

  echo "${BASE_STORAGE_PATH}/.union_bridge/op_${op_num}"
}

operator_env_file_path() {
  local op_num="$1"

  echo "$(operator_root_path "${op_num}")/docker/${ENVIRONMENT}.env"
}

project_name_for_operator() {
  local op_num="$1"

  if [[ "${ENVIRONMENT}" == "local" || "${ENVIRONMENT}" == "regtest" ]]; then
    echo "op_${op_num}"
  else
    echo "union-operator"
  fi
}

operator_client_op() {
  local op_num="$1"

  if [[ "${ENVIRONMENT}" == "local" || "${ENVIRONMENT}" == "regtest" ]]; then
    echo "op_${op_num}"
  else
    echo "testnet_op_${op_num}"
  fi
}

operator_user_api_port() {
  local op_num="$1"
  local -a ports=(40001 40002 40003 40004 40005 40006 40007 40008 40009 40010)

  if [[ "${ENVIRONMENT}" == "local" || "${ENVIRONMENT}" == "regtest" ]]; then
    echo "${ports[$((op_num - 1))]}"
  fi
}

operator_bitvmx_port() {
  local op_num="$1"
  local -a ports=(22222 33333 44444 55554 55555 55556 55557 55558 55559 55560)

  if [[ "${ENVIRONMENT}" == "local" || "${ENVIRONMENT}" == "regtest" ]]; then
    echo "${ports[$((op_num - 1))]}"
  fi
}

operator_bitvmx_p2p_host() {
  local op_num="$1"
  local -a hosts=("172.20.0.11" "172.20.0.12" "172.20.0.13" "172.20.0.14" "172.20.0.15" "172.20.0.16" "172.20.0.17" "172.20.0.18" "172.20.0.19" "172.20.0.20")

  if [[ "${ENVIRONMENT}" == "local" || "${ENVIRONMENT}" == "regtest" ]]; then
    echo "${hosts[$((op_num - 1))]}"
  fi
}

broker_pem_path() {
  local service="$1"
  local op_num="$2"

  echo "$(operator_root_path "${op_num}")/broker/${service}.pem"
}

broker_pubkey_hash_path() {
  local service="$1"
  local op_num="$2"

  echo "$(operator_root_path "${op_num}")/broker/${service}.pubkey_hash"
}

read_broker_pubkey_hash() {
  local service="$1"
  local op_num="$2"
  local hash_path

  hash_path="$(broker_pubkey_hash_path "${service}" "${op_num}")"
  if [[ ! -f "${hash_path}" ]]; then
    echo "Error: missing broker pubkey hash file ${hash_path}" >&2
    exit 1
  fi

  tr -d ' \n' < "${hash_path}"
}

read_env_value() {
  local env_file="$1"
  local key="$2"

  awk -F= -v key="${key}" '$1 == key {print substr($0, index($0, "=") + 1); exit}' "${env_file}"
}

# TODO(iago) revisit if we need this
resolve_user_bitcoin_wif() {
  local op_num="$1"
  local project_name="$2"
  local env_file="$3"
  local existing_wif=""

  if [[ -n "${USER_BITCOIN_WIF_ARG}" ]]; then
    echo "${USER_BITCOIN_WIF_ARG}"
    return 0
  fi

  if [[ -f "${env_file}" ]]; then
    existing_wif="$(read_env_value "${env_file}" "USER_BITCOIN_WIF")"
    if [[ -n "${existing_wif}" ]]; then
      echo "${existing_wif}"
      return 0
    fi
  fi

  while [[ -z "${existing_wif}" ]]; do
    echo "Please enter USER_BITCOIN_WIF for ${project_name} (op_${op_num}); input will be hidden:"
    read -r -s existing_wif
    echo ""
    if [[ -z "${existing_wif}" ]]; then
      echo "Error: USER_BITCOIN_WIF is required."
    fi
  done

  echo "${existing_wif}"
}

write_operator_env_file() {
  local env_file_path="$1"
  local op_num="$2"
  local user_bitcoin_wif="$3"
  local client_op

  mkdir -p "$(dirname "${env_file_path}")"

  client_op="$(operator_client_op "${op_num}")"

  cat > "${env_file_path}" <<EOF
CLIENT_OP=${client_op}
BLOCK_INDEXER_BROKER_PEM_PATH=$(broker_pem_path "block-indexer" "${op_num}")
LOG_INDEXER_BROKER_PEM_PATH=$(broker_pem_path "log-indexer" "${op_num}")
USER_API_BROKER_PEM_PATH=$(broker_pem_path "user-api" "${op_num}")
COORDINATOR_BROKER_PEM_PATH=$(broker_pem_path "coordinator" "${op_num}")
UB__COORDINATOR__BLOCKS__PUBKEY_HASH=$(read_broker_pubkey_hash "block-indexer" "${op_num}")
UB__COORDINATOR__LOGS__PUBKEY_HASH=$(read_broker_pubkey_hash "log-indexer" "${op_num}")
UB__COORDINATOR__USER__PUBKEY_HASH=$(read_broker_pubkey_hash "user-api" "${op_num}")
UB__USER_API__COORDINATOR__PUBKEY_HASH=$(read_broker_pubkey_hash "coordinator" "${op_num}")
USER_BITCOIN_WIF=${user_bitcoin_wif}
EOF

  local user_api_port
  local bitvmx_port
  local bitvmx_p2p_host

  user_api_port="$(operator_user_api_port "${op_num}")"
  bitvmx_port="$(operator_bitvmx_port "${op_num}")"
  bitvmx_p2p_host="$(operator_bitvmx_p2p_host "${op_num}")"

  if [[ -n "${user_api_port}" ]]; then
    printf 'USER_API_PORT=%s\n' "${user_api_port}" >> "${env_file_path}"
  fi
  if [[ -n "${bitvmx_port}" ]]; then
    printf 'BITVMX_PORT=%s\n' "${bitvmx_port}" >> "${env_file_path}"
  fi
  if [[ -n "${bitvmx_p2p_host}" ]]; then
    printf 'BITVMX_P2P_HOST=%s\n' "${bitvmx_p2p_host}" >> "${env_file_path}"
  fi

  chmod 600 "${env_file_path}" || true
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --help)
      print_help
      ;;
    --env)
      ENVIRONMENT="$2"
      shift 2
      ;;
    --op)
      OPERATOR_ARG="$2"
      if ! [[ "${OPERATOR_ARG}" =~ ^(10|[1-9])$ ]]; then
        echo "Error: --op must be between 1 and 10"
        exit 1
      fi
      shift 2
      ;;
    --ops)
      NUM_OPERATORS="$2"
      if ! [[ "${NUM_OPERATORS}" =~ ^(10|[1-9])$ ]]; then
        echo "Error: --ops must be between 1 and 10"
        exit 1
      fi
      shift 2
      ;;
    --user-bitcoin-wif)
      USER_BITCOIN_WIF_ARG="$2"
      shift 2
      ;;
    *)
      echo "Error: unknown argument '$1'"
      echo "Run '$0 --help' for usage information."
      exit 1
      ;;
  esac
done

if [[ -z "${ENVIRONMENT}" ]]; then
  prompt_environment
fi

if [[ "${ENVIRONMENT}" == "local-docker" ]]; then
  ENVIRONMENT="local"
fi

case "${ENVIRONMENT}" in
  local|regtest)
    if [[ -n "${OPERATOR_ARG}" ]]; then
      echo "Error: --op is not allowed for ${ENVIRONMENT}. Use --ops instead."
      exit 1
    fi
    if [[ -z "${NUM_OPERATORS}" ]]; then
      prompt_num_operators
    fi
    mapfile -t OPERATORS_TO_RUN < <(seq 1 "${NUM_OPERATORS}")
    ;;
  alphanet|testnet)
    if [[ -n "${NUM_OPERATORS}" ]]; then
      echo "Error: --ops is not allowed for ${ENVIRONMENT}. Use --op instead."
      exit 1
    fi
    if [[ -z "${OPERATOR_ARG}" ]]; then
      prompt_operator_id
    fi
    OPERATORS_TO_RUN=("${OPERATOR_ARG}")
    ;;
  *)
    echo "Error: invalid environment '${ENVIRONMENT}'."
    exit 1
    ;;
esac

if [[ "${ENVIRONMENT}" == "local" || "${ENVIRONMENT}" == "regtest" ]]; then
  BASE_STORAGE_PATH="${BASE_STORAGE_PATH}" "${SCRIPT_DIR}/create_broker_identities.sh" --ops "${NUM_OPERATORS}"
else
  BASE_STORAGE_PATH="${BASE_STORAGE_PATH}" "${SCRIPT_DIR}/create_broker_identities.sh" --op "${OPERATOR_ARG}"
fi

for op_num in "${OPERATORS_TO_RUN[@]}"; do
  project_name="$(project_name_for_operator "${op_num}")"
  env_file_path="$(operator_env_file_path "${op_num}")"
  user_bitcoin_wif_value="$(resolve_user_bitcoin_wif "${op_num}" "${project_name}" "${env_file_path}")"

  if [[ -f "${env_file_path}" ]]; then
    write_operator_env_file "${env_file_path}" "${op_num}" "${user_bitcoin_wif_value}"
    echo "Updated operator env file ${env_file_path}"
  else
    write_operator_env_file "${env_file_path}" "${op_num}" "${user_bitcoin_wif_value}"
    echo "Created operator env file ${env_file_path}"
  fi
done
