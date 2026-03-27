#!/usr/bin/env bash

set -euo pipefail

PROJECT_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SCRIPT_DIR="${PROJECT_ROOT}/docker/operator"

cd "${SCRIPT_DIR}" || {
  echo "Error: Failed to change to script directory: ${SCRIPT_DIR}"
  exit 1
}

OPERATOR_ARG="${UC_OPERATOR_ID:-}"
ENVIRONMENT="${UC_ENV:-}"
NUM_OPERATORS=""
BASE_STORAGE_PATH="${BASE_STORAGE_PATH:-$HOME}"
OPERATORS_TO_RUN=()
NEW_USER_BITCOIN_WIF="${USER_BITCOIN_WIF:-}"
USED_EXPORTED_USER_BITCOIN_WIF=false
BROKER_SERVICES=("block-indexer" "log-indexer" "user-api" "coordinator")
RESOLVED_USER_BITCOIN_WIF=""
RESOLVED_BITVMX_BROKER_PUBKEY_HASH=""

print_help() {
  echo "Usage: $0 [--env <ENV>] [--op <ID> | --ops <N>]"
  echo ""
  echo "Creates or reuses host-side Docker operator artifacts:"
  echo "  - broker identities under ${BASE_STORAGE_PATH}/.union_bridge/op_N/broker/<service>.*"
  echo "  - generated operator docker.env files under ${BASE_STORAGE_PATH}/.union_bridge/op_N/docker.env"
  echo "  - in --env local only: local cargo-mode keystores under ${BASE_STORAGE_PATH}/.union_bridge/op_N/keystore/{member,user}"
  echo "  Existing operator env files are refreshed in place."
  echo ""
  echo "Options:"
  echo "  --env <ENV>                Target environment: alphanet, testnet, local, local-docker, or regtest"
  echo "  --op <ID>                  Operator ID (1-10) for alphanet/testnet"
  echo "  --ops <N>                  Number of operators to prepare (1-10) for local/regtest"
  echo "  --help                     Display this help message"
  exit 0
}

ensure_dependencies() {
  if ! command -v openssl >/dev/null 2>&1; then
    echo "Error: openssl is required to create broker identities."
    exit 1
  fi
  if ! command -v perl >/dev/null 2>&1; then
    echo "Error: perl is required to patch generated BitVMX config."
    exit 1
  fi
}

check_key_store_password() {
  if [[ -z "${KEY_STORE_PASSWORD:-}" ]]; then
    echo "Error: KEY_STORE_PASSWORD is required." >&2
    echo "Export KEY_STORE_PASSWORD and rerun <project_root>/cli-setup-operators.sh." >&2
    exit 1
  fi
}

check_bitcoind_url() {
  if [[ -z "${BITCOIND_URL:-}" ]]; then
    echo "Error: BITCOIND_URL is required." >&2
    echo "Export BITCOIND_URL and rerun <project_root>/cli-setup-operators.sh." >&2
    echo "The generated BitVMX operator YAMLs are patched from the current shell environment." >&2
    exit 1
  fi
}

prepare_local_keystore_password() {
  if [[ "${ENVIRONMENT}" != "local" ]]; then
    return 0
  fi

  if ! command -v cargo >/dev/null 2>&1; then
    echo "Error: cargo is required to create local keystores in --env local mode." >&2
    exit 1
  fi
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

operator_docker_env_file_path() {
  local op_num="$1"

  echo "$(operator_root_path "${op_num}")/docker.env"
}

bitvmx_template_dir() {
  echo "${SCRIPT_DIR}/../bitvmx-client/config/${ENVIRONMENT}"
}

operator_bitvmx_root_path() {
  local op_num="$1"

  echo "$(operator_root_path "${op_num}")/bitvmx"
}

operator_bitvmx_config_dir() {
  local op_num="$1"

  echo "$(operator_bitvmx_root_path "${op_num}")"
}

operator_bitvmx_keys_dir() {
  local op_num="$1"

  echo "$(operator_bitvmx_config_dir "${op_num}")/keys"
}

operator_bitvmx_yaml_path() {
  local op_num="$1"
  local client_op

  client_op="$(operator_client_op "${op_num}")"
  echo "$(operator_bitvmx_config_dir "${op_num}")/${client_op}.yaml"
}

prune_extra_bitvmx_operator_yaml_files() {
  local target_dir="$1"
  local cfg_file="$2"
  local yaml_file
  local yaml_name

  for yaml_file in "${target_dir}"/*.yaml; do
    [[ -e "${yaml_file}" ]] || continue
    yaml_name="$(basename "${yaml_file}")"
    if [[ "${yaml_file}" != "${cfg_file}" ]] && [[ "${yaml_name}" =~ ^(op_[0-9]+|testnet_op_[0-9]+)\.yaml$ ]]; then
      rm -f "${yaml_file}"
    fi
  done
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

compute_pubkey_hash() {
  local key_path="$1"

  openssl pkey -in "${key_path}" -pubout -outform DER 2>/dev/null \
    | openssl dgst -sha256 -binary \
    | od -A n -v -t x1 \
    | tr -d ' \n'
}

generate_private_key() {
  local output_path="$1"

  openssl genpkey -algorithm RSA -out "${output_path}" -pkeyopt rsa_keygen_bits:2048 2>/dev/null
  chmod 600 "${output_path}"
}

provision_operator_broker_identities() {
  local op_num="$1"
  local broker_dir pem_path pubkey_hash_path action service

  echo "- Preparing broker identities for op_${op_num}:"

  for service in "${BROKER_SERVICES[@]}"; do
    broker_dir="$(operator_root_path "${op_num}")/broker"
    pem_path="${broker_dir}/${service}.pem"
    pubkey_hash_path="${broker_dir}/${service}.pubkey_hash"

    mkdir -p "${broker_dir}"

    if [[ -f "${pem_path}" ]]; then
      action="Reusing"
    else
      action="Creating"
      generate_private_key "${pem_path}"
    fi

    compute_pubkey_hash "${pem_path}" > "${pubkey_hash_path}"
    echo "  - ${action} ${service} key at ${pem_path} (pubkey_hash: $(cat "${pubkey_hash_path}"))"
  done
}

patch_bitvmx_key_storage_password() {
  local cfg_file="$1"
  local password="$2"

  if grep -q '^[[:space:]]*key_storage:' "${cfg_file}" && grep -q '^[[:space:]]*password:' "${cfg_file}"; then
    BITVMX_KEY_STORAGE_PASSWORD="${password}" \
      perl -0pi -e 's/(key_storage:\s*\n\s*password:\s*)[^\n]+/${1}$ENV{BITVMX_KEY_STORAGE_PASSWORD}/m' "${cfg_file}"
  fi
}

patch_bitvmx_bitcoin_url() {
  local cfg_file="$1"
  local bitcoin_url="$2"

  if ! grep -q '^[[:space:]]*bitcoin:' "${cfg_file}"; then
    return 0
  fi

  if perl -0ne 'exit 0 if /bitcoin:\s*\n(?:[ \t]+.*\n)*?[ \t]+url:\s*/m; END { exit 1 }' "${cfg_file}"; then
    BITVMX_BITCOIN_URL="${bitcoin_url}" \
      perl -0pi -e 's/(bitcoin:\s*\n(?:[ \t]+.*\n)*?[ \t]+url:\s*)[^\n]+/${1}$ENV{BITVMX_BITCOIN_URL}/m' "${cfg_file}"
    return 0
  fi

  BITVMX_BITCOIN_URL="${bitcoin_url}" \
    perl -0pi -e 's/(bitcoin:\s*\n(?:[ \t]+.*\n)*?[ \t]+wallet:\s*[^\n]+\n)/${1}  url: $ENV{BITVMX_BITCOIN_URL}\n/m' "${cfg_file}"
}

patch_bitvmx_component_pubkey_hash() {
  local cfg_file="$1"
  local component_name="$2"
  local pubkey_hash="$3"

  if grep -q "^[[:space:]]*${component_name}:" "${cfg_file}" && grep -q '^[[:space:]]*pubkey_hash:' "${cfg_file}"; then
    BITVMX_COMPONENT_NAME="${component_name}" BITVMX_COMPONENT_PUBKEY_HASH="${pubkey_hash}" \
      perl -0pi -e 's/($ENV{BITVMX_COMPONENT_NAME}:\s*\n\s*pubkey_hash:\s*)[^\n]+/${1}$ENV{BITVMX_COMPONENT_PUBKEY_HASH}/m' "${cfg_file}"
  fi
}

ensure_operator_bitvmx_config_tree() {
  local op_num="$1"
  local template_dir="$2"
  local target_dir="$3"
  local cfg_file="$4"

  if [[ ! -d "${template_dir}" ]]; then
    echo "Error: missing BitVMX template directory ${template_dir}" >&2
    exit 1
  fi

  if [[ ! -d "${target_dir}" ]]; then
    mkdir -p "$(dirname "${target_dir}")"
    cp -R "${template_dir}" "${target_dir}"
  fi

  if [[ ! -f "${cfg_file}" ]]; then
    echo "Error: missing generated BitVMX operator config ${cfg_file}" >&2
    echo "Delete $(operator_root_path "${op_num}") and rerun cli-setup-operators.sh from a clean state." >&2
    exit 1
  fi
}

generate_operator_bitvmx_keys() {
  local target_keys_dir="$1"
  shift
  local key_file

  mkdir -p "${target_keys_dir}"

  for key_file in "$@"; do
    if [[ "${key_file}" == "l2.key" ]]; then
      continue
    fi
    if [[ -f "${target_keys_dir}/${key_file}" ]]; then
      chmod 600 "${target_keys_dir}/${key_file}"
      echo "  - Reusing BitVMX key at ${target_keys_dir}/${key_file}"
    else
      generate_private_key "${target_keys_dir}/${key_file}"
      echo "  - Creating BitVMX key at ${target_keys_dir}/${key_file}"
    fi
  done
}

write_operator_bitvmx_pubkey_hash_files() {
  local target_keys_dir="$1"

  if [[ -f "${target_keys_dir}/services.key" ]]; then
    compute_pubkey_hash "${target_keys_dir}/services.key" > "${target_keys_dir}/services.pubkey_hash"
  fi
  if [[ -f "${target_keys_dir}/emulator.key" ]]; then
    compute_pubkey_hash "${target_keys_dir}/emulator.key" > "${target_keys_dir}/emulator.pubkey_hash"
  fi
  if [[ -f "${target_keys_dir}/prover.key" ]]; then
    compute_pubkey_hash "${target_keys_dir}/prover.key" > "${target_keys_dir}/prover.pubkey_hash"
  fi
}

patch_operator_bitvmx_identity_hashes() {
  local cfg_file="$1"
  local coordinator_pubkey_hash="$2"
  local target_keys_dir="$3"
  local bitvmx_pubkey_hash=""
  local emulator_pubkey_hash=""
  local prover_pubkey_hash=""

  if [[ -f "${target_keys_dir}/services.pubkey_hash" ]]; then
    bitvmx_pubkey_hash="$(tr -d ' \n' < "${target_keys_dir}/services.pubkey_hash")"
  fi
  if [[ -f "${target_keys_dir}/emulator.pubkey_hash" ]]; then
    emulator_pubkey_hash="$(tr -d ' \n' < "${target_keys_dir}/emulator.pubkey_hash")"
  fi
  if [[ -f "${target_keys_dir}/prover.pubkey_hash" ]]; then
    prover_pubkey_hash="$(tr -d ' \n' < "${target_keys_dir}/prover.pubkey_hash")"
  fi

  patch_bitvmx_component_pubkey_hash "${cfg_file}" "l2" "${coordinator_pubkey_hash}"
  if [[ -n "${bitvmx_pubkey_hash}" ]]; then
    patch_bitvmx_component_pubkey_hash "${cfg_file}" "bitvmx" "${bitvmx_pubkey_hash}"
  fi
  if [[ -n "${emulator_pubkey_hash}" ]]; then
    patch_bitvmx_component_pubkey_hash "${cfg_file}" "emulator" "${emulator_pubkey_hash}"
  fi
  if [[ -n "${prover_pubkey_hash}" ]]; then
    patch_bitvmx_component_pubkey_hash "${cfg_file}" "prover" "${prover_pubkey_hash}"
  fi

  RESOLVED_BITVMX_BROKER_PUBKEY_HASH="${bitvmx_pubkey_hash}"
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

resolve_user_bitcoin_wif() {
  local op_num="$1"
  local project_name="$2"
  local env_file="$3"
  local existing_wif=""

  if [[ -f "${env_file}" ]]; then
    existing_wif="$(read_env_value "${env_file}" "USER_BITCOIN_WIF")"
    if [[ -n "${existing_wif}" ]]; then
      RESOLVED_USER_BITCOIN_WIF="${existing_wif}"
      return 0
    fi

    echo "Error: existing operator env file ${env_file} is missing USER_BITCOIN_WIF." >&2
    echo "Fix the file or delete it and rerun cli-setup-operators.sh." >&2
    exit 1
  fi

  if [[ -z "${NEW_USER_BITCOIN_WIF}" ]]; then
    while [[ -z "${NEW_USER_BITCOIN_WIF}" ]]; do
      read -r -s -p "Please enter USER_BITCOIN_WIF for ${project_name} (op_${op_num}): " NEW_USER_BITCOIN_WIF
      echo ""
      if [[ -z "${NEW_USER_BITCOIN_WIF}" ]]; then
        echo "Error: USER_BITCOIN_WIF is required."
      fi
    done
  elif [[ "${USED_EXPORTED_USER_BITCOIN_WIF}" != true ]]; then
    echo "Using exported USER_BITCOIN_WIF for new operator env files." >&2
    USED_EXPORTED_USER_BITCOIN_WIF=true
  fi

  RESOLVED_USER_BITCOIN_WIF="${NEW_USER_BITCOIN_WIF}"
}

write_operator_env_file() {
  local env_file_path="$1"
  local op_num="$2"
  local user_bitcoin_wif="$3"
  local bitvmx_pubkey_hash="$4"
  local client_op

  mkdir -p "$(dirname "${env_file_path}")"

  client_op="$(operator_client_op "${op_num}")"

  cat > "${env_file_path}" <<EOF
CLIENT_OP=${client_op}
BITVMX_CONFIG_DIR=$(operator_bitvmx_config_dir "${op_num}")
BLOCK_INDEXER_BROKER_PEM_PATH=$(broker_pem_path "block-indexer" "${op_num}")
LOG_INDEXER_BROKER_PEM_PATH=$(broker_pem_path "log-indexer" "${op_num}")
USER_API_BROKER_PEM_PATH=$(broker_pem_path "user-api" "${op_num}")
COORDINATOR_BROKER_PEM_PATH=$(broker_pem_path "coordinator" "${op_num}")
UB__COORDINATOR__BLOCKS__PUBKEY_HASH=$(read_broker_pubkey_hash "block-indexer" "${op_num}")
UB__COORDINATOR__LOGS__PUBKEY_HASH=$(read_broker_pubkey_hash "log-indexer" "${op_num}")
UB__COORDINATOR__USER__PUBKEY_HASH=$(read_broker_pubkey_hash "user-api" "${op_num}")
UB__COORDINATOR__BITVMX__PUBKEY_HASH=${bitvmx_pubkey_hash}
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

  chmod 600 "${env_file_path}"
}

create_or_reuse_local_keystore() {
  local op_num="$1"
  local wallet_name="$2"
  local keystore_dir
  local target_path
  local cmd_output
  local generated_path

  keystore_dir="$(operator_root_path "${op_num}")/keystore"
  target_path="${keystore_dir}/${wallet_name}"
  mkdir -p "${keystore_dir}"

  if [[ -f "${target_path}" ]]; then
    chmod 600 "${target_path}" || true
    echo "  - Reusing local ${wallet_name} keystore at ${target_path}"
    return 0
  fi

  cmd_output="$(
    cd "${PROJECT_ROOT}" && cargo run --quiet --manifest-path key-manager/Cargo.toml -- \
      new-key -p "${KEY_STORE_PASSWORD}" -d "${keystore_dir}"
  )"

  generated_path="$(printf '%s\n' "${cmd_output}" | sed -n 's/^Generated key @ \([^,]*\),.*/\1/p')"
  if [[ -z "${generated_path}" || ! -f "${generated_path}" ]]; then
    echo "Error: failed to create ${wallet_name} keystore for op_${op_num}." >&2
    echo "key-manager output: ${cmd_output}" >&2
    exit 1
  fi

  mv "${generated_path}" "${target_path}"
  chmod 600 "${target_path}" || true
  echo "  - Created local ${wallet_name} keystore at ${target_path}"
}

prepare_local_keystores_if_needed() {
  local op_num="$1"

  if [[ "${ENVIRONMENT}" != "local" ]]; then
    return 0
  fi

  echo "- Preparing local cargo-mode keystores for op_${op_num}:"
  create_or_reuse_local_keystore "${op_num}" "member"
  create_or_reuse_local_keystore "${op_num}" "user"
}

prepare_operator_bitvmx_config() {
  local op_num="$1"
  local template_dir
  local target_dir
  local cfg_file
  local coordinator_pubkey_hash
  local client_op
  local target_keys_dir
  local config_action
  local -a referenced_key_files=()

  template_dir="$(bitvmx_template_dir)"
  target_dir="$(operator_bitvmx_root_path "${op_num}")"
  cfg_file="$(operator_bitvmx_yaml_path "${op_num}")"
  coordinator_pubkey_hash="$(read_broker_pubkey_hash "coordinator" "${op_num}")"
  client_op="$(operator_client_op "${op_num}")"
  target_keys_dir="$(operator_bitvmx_keys_dir "${op_num}")"

  if [[ -d "${target_dir}" ]]; then
    config_action="Reusing"
  else
    config_action="Creating"
  fi

  ensure_operator_bitvmx_config_tree "${op_num}" "${template_dir}" "${target_dir}" "${cfg_file}"

  prune_extra_bitvmx_operator_yaml_files "${target_dir}" "${cfg_file}"

  mapfile -t referenced_key_files < <(grep -Eo 'config/keys/[^[:space:]]+' "${cfg_file}" | sed 's#config/keys/##' | sort -u)
  generate_operator_bitvmx_keys "${target_keys_dir}" "${referenced_key_files[@]}"
  write_operator_bitvmx_pubkey_hash_files "${target_keys_dir}"
  patch_operator_bitvmx_identity_hashes "${cfg_file}" "${coordinator_pubkey_hash}" "${target_keys_dir}"
  patch_bitvmx_key_storage_password "${cfg_file}" "${KEY_STORE_PASSWORD}"
  patch_bitvmx_bitcoin_url "${cfg_file}" "${BITCOIND_URL}"

  rm -rf "${target_dir}/broker"

  echo "- ${config_action} BitVMX config for ${client_op} at ${cfg_file} (coordinator pubkey_hash: ${coordinator_pubkey_hash})"
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

ensure_dependencies
check_key_store_password
check_bitcoind_url
prepare_local_keystore_password

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

for op_num in "${OPERATORS_TO_RUN[@]}"; do
  project_name="$(project_name_for_operator "${op_num}")"
  env_file_path="$(operator_docker_env_file_path "${op_num}")"
  resolve_user_bitcoin_wif "${op_num}" "${project_name}" "${env_file_path}"
  user_bitcoin_wif_value="${RESOLVED_USER_BITCOIN_WIF}"

  echo "=== op_${op_num} (${ENVIRONMENT}) ==="
  provision_operator_broker_identities "${op_num}"
  prepare_local_keystores_if_needed "${op_num}"

  prepare_operator_bitvmx_config "${op_num}"
  bitvmx_pubkey_hash="${RESOLVED_BITVMX_BROKER_PUBKEY_HASH}"

  if [[ -f "${env_file_path}" ]]; then
    env_file_action="Updated"
  else
    env_file_action="Created"
  fi
  write_operator_env_file "${env_file_path}" "${op_num}" "${user_bitcoin_wif_value}" "${bitvmx_pubkey_hash}"
  echo "- ${env_file_action} operator env file ${env_file_path}"

  echo ""
done
