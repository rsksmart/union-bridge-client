#!/usr/bin/env bash
set -e

if [ "$(id -u)" -eq 0 ]; then
  mkdir -p /app/db
  chown -R appuser:appuser /app/db
  exec gosu appuser "$@"
fi

exec "$@"
