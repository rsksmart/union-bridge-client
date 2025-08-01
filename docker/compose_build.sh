#!/usr/bin/env bash

features=""
service=""
mocking=0

for arg in "$@"; do
  if [[ "$arg" == "anvil" ]]; then
    features="anvil"
  elif [[ "$arg" =~ ^service=(.+)$ ]]; then
    service="${BASH_REMATCH[1]}"
  elif [[ "$arg" == "mocking" ]]; then
    mocking=1
  fi
done

if [[ $mocking -eq 1 ]]; then
  cmd=(docker-compose -f docker-compose.yml -f docker-compose.mocking.yml build)
else 
  cmd=(docker compose build)
fi

[[ -n $service ]] && cmd+=("$service" --build-arg JUST_CRATE="$service")
[[ -n $features ]] && cmd+=(--build-arg FEATURES="$features")

echo "Building with command: ${cmd[@]}"

"${cmd[@]}"