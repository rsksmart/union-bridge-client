#!/usr/bin/env bash
set -e

# Patch op_*.yaml pubkey_hash fields using the shared broker pubkey hash.
# This runs inside the `bitvmx-client` container.

H=""
if [ -f /keystore/broker.pubkey_hash ]; then
  H="$(cat /keystore/broker.pubkey_hash)"
fi

if [ -n "${H}" ]; then
  for F in /app/config/op_*.yaml; do
    if [ -f "${F}" ]; then
      awk -v h="${H}" '/pubkey_hash:/ && ++n<=1 {sub(/pubkey_hash: .*/, "pubkey_hash: " h);} 1' \
        "${F}" > "${F}.tmp" && mv "${F}.tmp" "${F}" || true
    fi
  done
fi

