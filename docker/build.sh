#!/usr/bin/env bash

features=""
service=""

for arg in "$@"; do
  if [[ "$arg" == "anvil" ]]; then
    features="anvil"
  elif [[ "$arg" =~ ^service=(.+)$ ]]; then
    service="${BASH_REMATCH[1]}"
  fi
done

cmd=(docker compose build)

[[ -n $service ]] && cmd+=("$service" --build-arg JUST_CRATE="$service")
[[ -n $features ]] && cmd+=(--build-arg FEATURES="$features")

echo "Running command: ${cmd[@]}"

"${cmd[@]}"