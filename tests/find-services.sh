#!/usr/bin/env bash

SERVICES=("block-indexer" "log-indexer" "user-api" "coordinator")

echo "Checking for existing services..."

found=0
for svc in "${SERVICES[@]}"; do
    pids=$(pgrep -x "$svc" 2>/dev/null || true)
    if [[ -n "$pids" ]]; then
        echo "  $svc: PIDs [${pids//$'\n'/ }]"
        found=$((found + $(echo "$pids" | wc -w | tr -d ' ')))
    fi
done

if [[ $found -eq 0 ]]; then
    echo "No existing services found"
else
    echo "Found $found running service instance(s)"
fi
