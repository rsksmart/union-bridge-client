#!/usr/bin/env bash
set -e

if [ "$(id -u)" -eq 0 ]; then
  mkdir -p /app/db "${UB_LOG_DIR:-/app/logs}"
  chown -R appuser:appuser /app/db "${UB_LOG_DIR:-/app/logs}"
  exec gosu appuser "$@"
fi

exec "$@"
