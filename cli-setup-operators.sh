#!/usr/bin/env bash

set -euo pipefail

PROJECT_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SCRIPT_DIR="${PROJECT_ROOT}/docker/operator"

cd "${SCRIPT_DIR}" || {
  echo "Error: Failed to change to script directory: ${SCRIPT_DIR}"
  exit 1
}

NUM_OPERATORS=""
AUTO_CONFIRM=false
BASE_STORAGE_PATH="${BASE_STORAGE_PATH:-$HOME}"
OPERATORS_TO_RUN=()
NEW_KEY_STORE_PASSWORD="${KEY_STORE_PASSWORD:-}"
USED_EXPORTED_KEY_STORE_PASSWORD=false
NEW_BITVMX_MNEMONIC_SENTENCE="${BITVMX_MNEMONIC_SENTENCE:-}"
USED_EXPORTED_BITVMX_MNEMONIC_SENTENCE=false
USED_EXPORTED_BITVMX_MNEMONIC_PASSPHRASE=false
HAS_EXPORTED_BITVMX_MNEMONIC_PASSPHRASE=false
if [[ "${BITVMX_MNEMONIC_PASSPHRASE+x}" == x ]]; then
  HAS_EXPORTED_BITVMX_MNEMONIC_PASSPHRASE=true
fi
BROKER_SERVICES=("block-indexer" "log-indexer" "user-api" "coordinator")
RESOLVED_BITVMX_BROKER_PUBKEY_HASH=""
RESOLVED_KEY_STORE_PASSWORD=""
RESOLVED_BITVMX_MNEMONIC_SENTENCE=""
RESOLVED_BITVMX_MNEMONIC_PASSPHRASE=""

print_help() {
  echo "Usage: $0 [--ops <N>] [--yes|-y]"
  echo ""
  echo "Removes and recreates host-side local operator artifacts:"
  echo "  - service identities under ${BASE_STORAGE_PATH}/.union_bridge/op_N/union-client/broker/<service>.*"
  echo "  - generated operator docker-compose.env/docker-service.env files under ${BASE_STORAGE_PATH}/.union_bridge/op_N/"
  echo "  - host-side Rootstock keystores under ${BASE_STORAGE_PATH}/.union_bridge/op_N/union-client/keystore/{member,user}"
  echo "    used by local cargo mode and docker/operator"
  echo "  Existing selected operator folders are removed before setup starts."
  echo "  Current KEY_STORE_PASSWORD must be exported or entered when prompted."
  echo ""
  echo "Options:"
  echo "  --ops <N>                  Number of operators to prepare (1-10)"
  echo "  --yes, -y                  Automatic yes to operator folder removal confirmation"
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
  if ! command -v cargo >/dev/null 2>&1; then
    echo "Error: cargo is required to create local keystores." >&2
    exit 1
  fi
  if ! command -v python3 >/dev/null 2>&1; then
    echo "Error: python3 is required to generate BitVMX mnemonics." >&2
    echo "Install python3 plus the 'mnemonic' package (for example: pip install mnemonic or pip3 install mnemonic)." >&2
    exit 1
  fi
  if ! python3 -c "import mnemonic" >/dev/null 2>&1; then
    echo "Error: python3 module 'mnemonic' is required to generate BitVMX mnemonics." >&2
    echo "Install it with 'pip install mnemonic' or 'pip3 install mnemonic' and rerun cli-setup-operators.sh." >&2
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

confirm_and_remove_operator_roots() {
  local op_num
  local op_root
  local confirmation
  local -a existing_roots=()

  for op_num in "${OPERATORS_TO_RUN[@]}"; do
    op_root="$(operator_root_path "${op_num}")"
    if [[ "${op_root}" != /* || ! "${op_root}" =~ /[.]union_bridge/op_(10|[1-9])$ ]]; then
      echo "Error: refusing to remove unexpected operator path ${op_root}" >&2
      echo "Set BASE_STORAGE_PATH to an absolute path before running setup." >&2
      exit 1
    fi
    if [[ -d "${op_root}" ]]; then
      existing_roots+=("${op_root}")
    fi
  done

  if [[ ${#existing_roots[@]} -eq 0 ]]; then
    return 0
  fi

  echo "The following operator folders will be removed before setup:"
  for op_root in "${existing_roots[@]}"; do
    echo "  - ${op_root}"
  done

  if [[ "${AUTO_CONFIRM}" != true ]]; then
    read -r -p "Are you sure you want to continue? (yes/no): " confirmation
    if [[ "${confirmation}" != "yes" ]]; then
      echo "Aborted."
      exit 1
    fi
  fi

  for op_root in "${existing_roots[@]}"; do
    rm -rf -- "${op_root}"
  done
}

operator_compose_env_file_path() {
  local op_num="$1"

  echo "$(operator_root_path "${op_num}")/docker-compose.env"
}

operator_runtime_env_file_path() {
  local op_num="$1"

  echo "$(operator_root_path "${op_num}")/docker-service.env"
}

bitvmx_template_dir() {
  echo "${SCRIPT_DIR}/../bitvmx-client/config/local"
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

  echo "$(operator_bitvmx_config_dir "${op_num}")/op_${op_num}.yaml"
}

prune_extra_bitvmx_operator_yaml_files() {
  local target_dir="$1"
  local cfg_file="$2"
  local yaml_file
  local yaml_name

  for yaml_file in "${target_dir}"/*.yaml; do
    [[ -e "${yaml_file}" ]] || continue
    yaml_name="$(basename "${yaml_file}")"
    if [[ "${yaml_file}" != "${cfg_file}" ]] && [[ "${yaml_name}" =~ ^op_[0-9]+\.yaml$ ]]; then
      rm -f "${yaml_file}"
    fi
  done
}

operator_user_api_port() {
  local op_num="$1"
  local -a ports=(40001 40002 40003 40004 40005 40006 40007 40008 40009 40010)

  echo "${ports[$((op_num - 1))]}"
}

operator_bitvmx_port() {
  local op_num="$1"
  local -a ports=(22222 33333 44444 55554 55555 55556 55557 55558 55559 55560)

  echo "${ports[$((op_num - 1))]}"
}

operator_bitvmx_p2p_host() {
  local op_num="$1"
  local -a hosts=("172.20.0.11" "172.20.0.12" "172.20.0.13" "172.20.0.14" "172.20.0.15" "172.20.0.16" "172.20.0.17" "172.20.0.18" "172.20.0.19" "172.20.0.20")

  echo "${hosts[$((op_num - 1))]}"
}

broker_pem_path() {
  local service="$1"
  local op_num="$2"

  echo "$(operator_root_path "${op_num}")/union-client/broker/${service}.pem"
}

broker_pubkey_hash_path() {
  local service="$1"
  local op_num="$2"

  echo "$(operator_root_path "${op_num}")/union-client/broker/${service}.pubkey_hash"
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
  local identity_dir pem_path pubkey_hash_path action service

  echo "- Preparing service identities for op_${op_num}:"

  for service in "${BROKER_SERVICES[@]}"; do
    identity_dir="$(operator_root_path "${op_num}")/union-client/broker"
    pem_path="${identity_dir}/${service}.pem"
    pubkey_hash_path="${identity_dir}/${service}.pubkey_hash"

    mkdir -p "${identity_dir}"

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

yaml_single_quote() {
  local value="$1"

  printf "'%s'" "$(printf '%s' "${value}" | sed "s/'/''/g")"
}

strip_yaml_string_quotes() {
  local value="$1"

  if [[ ${#value} -ge 2 && "${value:0:1}" == "'" && "${value: -1}" == "'" ]]; then
    value="${value:1:${#value}-2}"
    value="${value//\'\'/\'}"
  elif [[ ${#value} -ge 2 && "${value:0:1}" == "\"" && "${value: -1}" == "\"" ]]; then
    value="${value:1:${#value}-2}"
  fi

  printf '%s' "${value}"
}

is_patch_placeholder() {
  local value="$1"

  [[ "${value}" == "<to_patch_in_host>" ]]
}

read_bitvmx_key_manager_value() {
  local cfg_file="$1"
  local key="$2"

  awk -v key="${key}" '
    $1 == "key_manager:" { in_key_manager = 1; next }
    in_key_manager && /^[^[:space:]]/ { exit }
    in_key_manager && $1 == key ":" {
      sub(/^[[:space:]]*[^:]+:[[:space:]]*/, "", $0)
      print $0
      exit
    }
  ' "${cfg_file}"
}

patch_bitvmx_key_manager_field() {
  local cfg_file="$1"
  local field_name="$2"
  local field_value="$3"
  local yaml_value=""

  if grep -q '^[[:space:]]*key_manager:' "${cfg_file}" && grep -q "^[[:space:]]*${field_name}:" "${cfg_file}"; then
    yaml_value="$(yaml_single_quote "${field_value}")"
    BITVMX_KEY_MANAGER_FIELD="${field_name}" BITVMX_KEY_MANAGER_VALUE="${yaml_value}" \
      perl -0pi -e 's/(key_manager:\s*\n(?:\s+.*\n)*?\s+$ENV{BITVMX_KEY_MANAGER_FIELD}:\s*)[^\n]+/${1}$ENV{BITVMX_KEY_MANAGER_VALUE}/m' "${cfg_file}"
  fi
}

patch_bitvmx_bitcoin_url() {
  local cfg_file="$1"
  local bitcoin_url="$2"

  if grep -q '^[[:space:]]*bitcoin:' "${cfg_file}" && grep -q '^[[:space:]]*url:' "${cfg_file}"; then
    BITVMX_BITCOIN_URL="${bitcoin_url}" \
      perl -0pi -e 's/(bitcoin:\s*\n(?:\s+.*\n)*?\s+url:\s*)[^\n]+/${1}$ENV{BITVMX_BITCOIN_URL}/m' "${cfg_file}"
  fi
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

ensure_bitvmx_broker_settings() {
  local cfg_file="$1"

  if grep -q '^[[:space:]]*settings:[[:space:]]*config/broker_settings.yaml[[:space:]]*$' "${cfg_file}"; then
    return 0
  fi

  BITVMX_BROKER_SETTINGS_PATH="config/broker_settings.yaml" \
    perl -0pi -e 's/(^broker:\s*\n)/${1}  settings: $ENV{BITVMX_BROKER_SETTINGS_PATH}\n/m' "${cfg_file}"
}

sync_bitvmx_support_files() {
  local template_dir="$1"
  local target_dir="$2"
  local support_file

  while IFS= read -r support_file; do
    cp "${support_file}" "${target_dir}/$(basename "${support_file}")"
  done < <(find "${template_dir}" -maxdepth 1 -type f ! -name 'op_*.yaml' | sort)
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
  if [[ -f "${target_keys_dir}/garbler.key" ]]; then
    compute_pubkey_hash "${target_keys_dir}/garbler.key" > "${target_keys_dir}/garbler.pubkey_hash"
  fi
}

patch_operator_bitvmx_identity_hashes() {
  local cfg_file="$1"
  local coordinator_pubkey_hash="$2"
  local target_keys_dir="$3"
  local bitvmx_pubkey_hash=""
  local emulator_pubkey_hash=""
  local prover_pubkey_hash=""
  local garbler_pubkey_hash=""

  if [[ -f "${target_keys_dir}/services.pubkey_hash" ]]; then
    bitvmx_pubkey_hash="$(tr -d ' \n' < "${target_keys_dir}/services.pubkey_hash")"
  fi
  if [[ -f "${target_keys_dir}/emulator.pubkey_hash" ]]; then
    emulator_pubkey_hash="$(tr -d ' \n' < "${target_keys_dir}/emulator.pubkey_hash")"
  fi
  if [[ -f "${target_keys_dir}/prover.pubkey_hash" ]]; then
    prover_pubkey_hash="$(tr -d ' \n' < "${target_keys_dir}/prover.pubkey_hash")"
  fi
  if [[ -f "${target_keys_dir}/garbler.pubkey_hash" ]]; then
    garbler_pubkey_hash="$(tr -d ' \n' < "${target_keys_dir}/garbler.pubkey_hash")"
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
  if [[ -n "${garbler_pubkey_hash}" ]]; then
    patch_bitvmx_component_pubkey_hash "${cfg_file}" "garbler" "${garbler_pubkey_hash}"
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

resolve_bitvmx_mnemonic_sentence() {
  local op_num="$1"
  local cfg_file="$2"
  local existing_value=""
  local generated_value=""

  if [[ -n "${NEW_BITVMX_MNEMONIC_SENTENCE}" ]]; then
    if [[ "${USED_EXPORTED_BITVMX_MNEMONIC_SENTENCE}" != true ]]; then
      echo "Using exported BITVMX_MNEMONIC_SENTENCE for BitVMX configs." >&2
      USED_EXPORTED_BITVMX_MNEMONIC_SENTENCE=true
    fi
    RESOLVED_BITVMX_MNEMONIC_SENTENCE="${NEW_BITVMX_MNEMONIC_SENTENCE}"
    return 0
  fi

  if [[ -f "${cfg_file}" ]]; then
    existing_value="$(read_bitvmx_key_manager_value "${cfg_file}" "mnemonic_sentence")"
    existing_value="$(strip_yaml_string_quotes "${existing_value}")"
    if [[ -n "${existing_value}" ]] && ! is_patch_placeholder "${existing_value}"; then
      RESOLVED_BITVMX_MNEMONIC_SENTENCE="${existing_value}"
      return 0
    fi
  fi

  generated_value="$(python3 -c "from mnemonic import Mnemonic; print(Mnemonic('english').generate(strength=128))")"
  if [[ -z "${generated_value}" ]]; then
    echo "Error: failed to generate BITVMX_MNEMONIC_SENTENCE for op_${op_num}." >&2
    exit 1
  fi
  echo "Generated BITVMX_MNEMONIC_SENTENCE for op_${op_num} with python3 mnemonic." >&2

  RESOLVED_BITVMX_MNEMONIC_SENTENCE="${generated_value}"
}

resolve_bitvmx_mnemonic_passphrase() {
  local cfg_file="$1"
  local existing_value=""

  if [[ "${HAS_EXPORTED_BITVMX_MNEMONIC_PASSPHRASE}" == true ]]; then
    if [[ "${USED_EXPORTED_BITVMX_MNEMONIC_PASSPHRASE}" != true ]]; then
      echo "Using exported BITVMX_MNEMONIC_PASSPHRASE for BitVMX configs." >&2
      USED_EXPORTED_BITVMX_MNEMONIC_PASSPHRASE=true
    fi
    RESOLVED_BITVMX_MNEMONIC_PASSPHRASE="${BITVMX_MNEMONIC_PASSPHRASE}"
    return 0
  fi

  if [[ -f "${cfg_file}" ]]; then
    existing_value="$(read_bitvmx_key_manager_value "${cfg_file}" "mnemonic_passphrase")"
    if [[ -n "${existing_value}" ]]; then
      existing_value="$(strip_yaml_string_quotes "${existing_value}")"
      if ! is_patch_placeholder "${existing_value}"; then
        RESOLVED_BITVMX_MNEMONIC_PASSPHRASE="${existing_value}"
        return 0
      fi
    fi
  fi

  RESOLVED_BITVMX_MNEMONIC_PASSPHRASE=""
}

resolve_key_store_password() {
  local op_num="$1"

  if [[ -n "${KEY_STORE_PASSWORD:-}" ]]; then
    if [[ "${USED_EXPORTED_KEY_STORE_PASSWORD}" != true ]]; then
      echo "Using exported KEY_STORE_PASSWORD for operator env files." >&2
      USED_EXPORTED_KEY_STORE_PASSWORD=true
    fi
    RESOLVED_KEY_STORE_PASSWORD="${KEY_STORE_PASSWORD}"
    return 0
  fi

  if [[ -z "${NEW_KEY_STORE_PASSWORD}" ]]; then
    while [[ -z "${NEW_KEY_STORE_PASSWORD}" ]]; do
      read -r -s -p "Please enter KEY_STORE_PASSWORD for op_${op_num}: " NEW_KEY_STORE_PASSWORD
      echo ""
      if [[ -z "${NEW_KEY_STORE_PASSWORD}" ]]; then
        echo "Error: KEY_STORE_PASSWORD is required."
      fi
    done
  fi

  RESOLVED_KEY_STORE_PASSWORD="${NEW_KEY_STORE_PASSWORD}"
}

write_operator_compose_env_file() {
  local env_file_path="$1"
  local op_num="$2"

  mkdir -p "$(dirname "${env_file_path}")"

  cat > "${env_file_path}" <<EOF
CLIENT_OP=op_${op_num}
KEYSTORE_DIR=$(operator_root_path "${op_num}")/union-client/keystore
BITVMX_CONFIG_DIR=$(operator_bitvmx_config_dir "${op_num}")
BLOCK_INDEXER_BROKER_PEM_PATH=$(broker_pem_path "block-indexer" "${op_num}")
LOG_INDEXER_BROKER_PEM_PATH=$(broker_pem_path "log-indexer" "${op_num}")
USER_API_BROKER_PEM_PATH=$(broker_pem_path "user-api" "${op_num}")
COORDINATOR_BROKER_PEM_PATH=$(broker_pem_path "coordinator" "${op_num}")
USER_API_PORT=$(operator_user_api_port "${op_num}")
BITVMX_PORT=$(operator_bitvmx_port "${op_num}")
BITVMX_P2P_HOST=$(operator_bitvmx_p2p_host "${op_num}")
EOF

  chmod 600 "${env_file_path}"
}

write_operator_runtime_env_file() {
  local env_file_path="$1"
  local op_num="$2"
  local key_store_password="$3"
  local bitvmx_pubkey_hash="$4"

  mkdir -p "$(dirname "${env_file_path}")"

  cat > "${env_file_path}" <<EOF
UB__COORDINATOR__BLOCKS__PUBKEY_HASH=$(read_broker_pubkey_hash "block-indexer" "${op_num}")
UB__COORDINATOR__LOGS__PUBKEY_HASH=$(read_broker_pubkey_hash "log-indexer" "${op_num}")
UB__COORDINATOR__USER__PUBKEY_HASH=$(read_broker_pubkey_hash "user-api" "${op_num}")
UB__COORDINATOR__BITVMX__PUBKEY_HASH=${bitvmx_pubkey_hash}
UB__COORDINATOR__BITVMX__PORT=$(operator_bitvmx_port "${op_num}")
UB__USER_API__COORDINATOR__PUBKEY_HASH=$(read_broker_pubkey_hash "coordinator" "${op_num}")
KEY_STORE_PASSWORD=${key_store_password}
EOF

  chmod 600 "${env_file_path}"
}

create_or_reuse_local_keystore() {
  local op_num="$1"
  local wallet_name="$2"
  local key_store_password="$3"
  local keystore_dir
  local target_path
  local cmd_output
  local generated_path

  keystore_dir="$(operator_root_path "${op_num}")/union-client/keystore"
  target_path="${keystore_dir}/${wallet_name}"
  mkdir -p "${keystore_dir}"

  if [[ -f "${target_path}" ]]; then
    chmod 600 "${target_path}" || true
    echo "  - Reusing local ${wallet_name} keystore at ${target_path}"
    return 0
  fi

  cmd_output="$(
    cd "${PROJECT_ROOT}" && cargo run --quiet --manifest-path key-manager/Cargo.toml -- \
      new-key -p "${key_store_password}" -d "${keystore_dir}"
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

prepare_local_keystores() {
  local op_num="$1"
  local key_store_password="$2"

  echo "- Preparing host-side Rootstock keystores for op_${op_num}:"
  create_or_reuse_local_keystore "${op_num}" "member" "${key_store_password}"
  create_or_reuse_local_keystore "${op_num}" "user" "${key_store_password}"
}

prepare_operator_bitvmx_config() {
  local op_num="$1"
  local key_store_password="$2"
  local template_dir
  local target_dir
  local cfg_file
  local coordinator_pubkey_hash
  local target_keys_dir
  local config_action
  local bitvmx_mnemonic_sentence
  local bitvmx_mnemonic_passphrase
  local -a referenced_key_files=()

  template_dir="$(bitvmx_template_dir)"
  target_dir="$(operator_bitvmx_root_path "${op_num}")"
  cfg_file="$(operator_bitvmx_yaml_path "${op_num}")"
  coordinator_pubkey_hash="$(read_broker_pubkey_hash "coordinator" "${op_num}")"
  target_keys_dir="$(operator_bitvmx_keys_dir "${op_num}")"

  if [[ -d "${target_dir}" ]]; then
    config_action="Reusing"
  else
    config_action="Creating"
  fi

  ensure_operator_bitvmx_config_tree "${op_num}" "${template_dir}" "${target_dir}" "${cfg_file}"
  sync_bitvmx_support_files "${template_dir}" "${target_dir}"
  ensure_bitvmx_broker_settings "${cfg_file}"
  resolve_bitvmx_mnemonic_sentence "${op_num}" "${cfg_file}"
  bitvmx_mnemonic_sentence="${RESOLVED_BITVMX_MNEMONIC_SENTENCE}"
  resolve_bitvmx_mnemonic_passphrase "${cfg_file}"
  bitvmx_mnemonic_passphrase="${RESOLVED_BITVMX_MNEMONIC_PASSPHRASE}"

  prune_extra_bitvmx_operator_yaml_files "${target_dir}" "${cfg_file}"

  referenced_key_files=()
  while IFS= read -r key_file; do
    referenced_key_files+=("${key_file}")
  done < <(grep -Eo 'config/keys/[^[:space:]]+' "${cfg_file}" | sed 's#config/keys/##' | sort -u)
  generate_operator_bitvmx_keys "${target_keys_dir}" "${referenced_key_files[@]}"
  write_operator_bitvmx_pubkey_hash_files "${target_keys_dir}"
  patch_operator_bitvmx_identity_hashes "${cfg_file}" "${coordinator_pubkey_hash}" "${target_keys_dir}"
  patch_bitvmx_key_manager_field "${cfg_file}" "mnemonic_sentence" "${bitvmx_mnemonic_sentence}"
  patch_bitvmx_key_manager_field "${cfg_file}" "mnemonic_passphrase" "${bitvmx_mnemonic_passphrase}"
  patch_bitvmx_key_storage_password "${cfg_file}" "${key_store_password}"
  patch_bitvmx_bitcoin_url "${cfg_file}" "${BITCOIND_URL}"

  rm -rf "${target_dir}/broker"

  echo "- ${config_action} BitVMX config for op_${op_num} at ${cfg_file} (coordinator pubkey_hash: ${coordinator_pubkey_hash})"
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --help)
      print_help
      ;;
    --ops)
      NUM_OPERATORS="$2"
      if ! [[ "${NUM_OPERATORS}" =~ ^(10|[1-9])$ ]]; then
        echo "Error: --ops must be between 1 and 10"
        exit 1
      fi
      shift 2
      ;;
    --yes|-y)
      AUTO_CONFIRM=true
      shift
      ;;
    *)
      echo "Error: unknown argument '$1'"
      echo "Run '$0 --help' for usage information."
      exit 1
      ;;
  esac
done

ensure_dependencies
check_bitcoind_url

if [[ -z "${NUM_OPERATORS}" ]]; then
  prompt_num_operators
fi

OPERATORS_TO_RUN=()
while IFS= read -r op_num; do
  OPERATORS_TO_RUN+=("${op_num}")
done < <(seq 1 "${NUM_OPERATORS}")

confirm_and_remove_operator_roots

for op_num in "${OPERATORS_TO_RUN[@]}"; do
  compose_env_file_path="$(operator_compose_env_file_path "${op_num}")"
  runtime_env_file_path="$(operator_runtime_env_file_path "${op_num}")"
  resolve_key_store_password "${op_num}"
  key_store_password_value="${RESOLVED_KEY_STORE_PASSWORD}"

  echo "=== op_${op_num} ==="
  provision_operator_broker_identities "${op_num}"
  prepare_local_keystores "${op_num}" "${key_store_password_value}"

  prepare_operator_bitvmx_config "${op_num}" "${key_store_password_value}"
  bitvmx_pubkey_hash="${RESOLVED_BITVMX_BROKER_PUBKEY_HASH}"

  if [[ -f "${compose_env_file_path}" || -f "${runtime_env_file_path}" ]]; then
    env_file_action="Updated"
  else
    env_file_action="Created"
  fi
  write_operator_compose_env_file "${compose_env_file_path}" "${op_num}"
  write_operator_runtime_env_file "${runtime_env_file_path}" "${op_num}" "${key_store_password_value}" "${bitvmx_pubkey_hash}"
  echo "- ${env_file_action} operator env files ${compose_env_file_path} and ${runtime_env_file_path}"

  echo ""
done
