#!/usr/bin/env bash
# Validates that the commit message follows Conventional Commits:
#   <type>(<scope>)?: <description>
#
# Invoked by git's commit-msg hook with the path to the commit message file
# as $1; defaults to .git/COMMIT_EDITMSG when run manually.

set -e

msg_file="${1:-.git/COMMIT_EDITMSG}"

if [ ! -f "$msg_file" ]; then
    echo "❌ Failed to read commit message file: $msg_file" >&2
    exit 1
fi

# Pull the first non-blank line (skip generated "#" comment lines too).
msg=$(grep -v '^#' "$msg_file" | sed '/^[[:space:]]*$/d' | head -n1 | \
      sed -E 's/^[[:space:]]+|[[:space:]]+$//g')

if [[ "$msg" =~ ^(feat|fix|chore|docs|refactor|test|style|perf|build)(\([^\)]+\))?:[[:space:]]+.+$ ]]; then
    echo "✅ Commit message is valid: \"$msg\""
    exit 0
fi

echo "❌ Invalid commit message:" >&2
echo "" >&2
echo "\"$msg\"" >&2
echo "" >&2
echo "Expected format: type(scope?): description" >&2
echo "Allowed types: feat, fix, chore, docs, refactor, test, style, perf, build" >&2
echo "Example: fix(wallet): handle gas estimation issue" >&2
exit 1
