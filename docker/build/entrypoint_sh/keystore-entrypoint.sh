#!/usr/bin/env bash
set -e

if [ "$(id -u)" -eq 0 ]; then
  source_keystore_dir="/keystore-src"
  runtime_keystore_dir="/keystore"

  mkdir -p "${runtime_keystore_dir}"
  chown -R appuser:appuser "${runtime_keystore_dir}"

  if [ -d "${source_keystore_dir}" ]; then
    cp -R "${source_keystore_dir}/." "${runtime_keystore_dir}/"
    chown -R appuser:appuser "${runtime_keystore_dir}"
    find "${runtime_keystore_dir}" -type f -exec chmod 600 {} +
  fi

  exec sh /app/db-entrypoint.sh "$@"
fi

exec "$@"
