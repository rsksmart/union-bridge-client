#!/usr/bin/env bash
set -e

if [ "$(id -u)" -eq 0 ]; then
  mkdir -p /keystore /app/db
  chown -R appuser:appuser /keystore /app/db
  exec gosu appuser "$@"
fi

exec "$@"
