#!/usr/bin/env bash
set -e

if [ "$(id -u)" -eq 0 ]; then
  log_dir="${UB_LOG_DIR:-/app/logs}"
  mkdir -p /app/db "$log_dir"
  chown -R appuser:appuser /app/db
  chown appuser:appuser "$log_dir"
  exec gosu appuser "$@"
fi

exec "$@"
