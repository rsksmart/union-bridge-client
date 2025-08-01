#!/usr/bin/env bash

mocking=0

for arg in "$@"; do
  if [[ "$arg" == "mocking" ]]; then
    mocking=1
  fi
done

if [[ $mocking -eq 1 ]]; then
  cmd=(docker-compose -f docker-compose.yml -f docker-compose.mocking.yml up)
else
  cmd=(docker compose up)
fi

echo "Running with command: ${cmd[@]}"

"${cmd[@]}"