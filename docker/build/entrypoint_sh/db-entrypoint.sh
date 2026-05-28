#!/usr/bin/env bash
set -e

if [ "$(id -u)" -eq 0 ]; then
  mkdir -p /app/db /app/logs
  chown -R appuser:appuser /app/db /app/logs
  exec gosu appuser "$@"
fi

exec "$@"
