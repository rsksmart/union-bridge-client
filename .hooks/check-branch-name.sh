#!/usr/bin/env bash
# Validates that the current branch name follows the convention
# <type>/<rest>, where <type> is one of the conventional-commit types
# recognised by this repository.
#
# Skips when:
#   - A rebase is in progress.
#   - HEAD is on a tag (typical when pushing a release tag).
#   - HEAD is detached (typical when pushing a tag).

set -e

VALID_TYPES=(feat fix chore docs refactor test style perf build)

if [ -d .git/rebase-merge ] || [ -d .git/rebase-apply ]; then
    echo "In the middle of a rebase, skipping branch name check"
    exit 0
fi

if tag=$(git describe --tags --exact-match HEAD 2>/dev/null); then
    echo "Pushing tag '$tag', skipping branch name check"
    exit 0
fi

if ! branch=$(git symbolic-ref --short HEAD 2>/dev/null); then
    echo "Detached HEAD detected (likely pushing a tag), skipping branch name check"
    exit 0
fi

for prefix in "${VALID_TYPES[@]}"; do
    if [[ "$branch" == "$prefix"/* ]]; then
        exit 0
    fi
done

joined=$(IFS=, ; echo "${VALID_TYPES[*]}")
echo "❌ Error: Branch name '$branch' is invalid." >&2
echo "It must start with one of: ($joined)/" >&2
exit 1
