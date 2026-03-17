#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

# Setup BitVMX docker
# Usage:
#   ./check_bitvmx_updates.sh [-r|--ref <branch-or-tag>]
# Defaults to 'main' when not provided.

print_help() {
  cat <<EOF
Usage: $0 [-r|--ref <branch-or-tag>] [-s|--ssh] [-h|--help]

Options:
  -r, --ref <ref>   Branch or tag of BitVMX upstream docker-bitvmx repo to use (default: main)
  -s, --ssh         Use SSH URL for cloning instead of HTTPS
  -h, --help        Show this help and exit
EOF
}

REF="main"
USE_SSH=false

# Parse args (short and long)
while [ $# -gt 0 ]; do
  case "$1" in
    -r|--ref)
      if [ $# -lt 2 ]; then
        echo "Error: $1 requires a value" >&2
        exit 1
      fi
      REF="$2"
      shift 2
      ;;
    -s|--ssh)
      USE_SSH=true
      shift
      ;;
    -h|--help)
      print_help
      exit 0
      ;;
    --) # end of options
      shift
      break
      ;;
    -*)
      echo "Unknown option: $1" >&2
      print_help
      exit 1
      ;;
    *)
      echo "Unexpected argument: $1" >&2
      print_help
      exit 1
      ;;
  esac
done

echo "Setting up BitVMX... (ref=${REF})"

BC_PATH="${SCRIPT_DIR}/bitvmx-client"
BC_REPO="docker-bitvmx"

# Set URL based on SSH flag
if [ "${USE_SSH}" = true ]; then
  BC_URL="git@github.com:FairgateLabs/${BC_REPO}.git"
  echo "Using SSH URL for cloning..."
else
  BC_URL="https://github.com/FairgateLabs/${BC_REPO}.git"
  echo "Using HTTPS URL for cloning..."
fi

# Ensure target path exists
mkdir -p "${BC_PATH}"

# Clone the specified branch or tag (shallow)
rm -rf "${SCRIPT_DIR:?}/${BC_REPO:?}" || true
git clone --depth 1 --branch "${REF}" --single-branch "${BC_URL}" "${SCRIPT_DIR}/${BC_REPO}"

# Copy compose file into place
cp "${SCRIPT_DIR}/${BC_REPO}/docker-compose.yml" "${BC_PATH}/docker-compose.fetched.yml"

# Cleanup
rm -rf "${SCRIPT_DIR:?}/${BC_REPO:?}"

echo "BitVMX compose updated from ${BC_REPO}@${REF}."

# Print diff between fetched compose and the working compose
FETCHED_COMPOSE="${BC_PATH}/docker-compose.fetched.yml"
WORKING_COMPOSE="${BC_PATH}/docker-compose.yml"

echo
echo "----- Diff: docker-compose.fetched.yml (fetched) vs docker-compose.yml (working) -----"
# diff exits with 1 when files differ; we don't want to fail the script under 'set -e'
set +e
diff -u --label "docker-compose.fetched.yml (fetched)" --label "docker-compose.yml (working)" "${FETCHED_COMPOSE}" "${WORKING_COMPOSE}"
DIFF_STATUS=$?
set -e
if [ ${DIFF_STATUS} -eq 0 ]; then
  echo "No differences found."
elif [ ${DIFF_STATUS} -eq 1 ]; then
  : # differences already printed by diff
else
  echo "Warning: diff command failed with exit code ${DIFF_STATUS}." >&2
fi

echo
echo "Setup complete, check the diff above for any potential adjustments on docker-compose.yml."