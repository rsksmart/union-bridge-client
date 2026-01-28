#!/bin/bash
# Script to create a debug tag in union-bridge-client repo
# This tag includes extensive logging for debugging act/GitHub Actions issues

set -e

CURRENT_TAG="v0.2.0"
DEBUG_TAG="v0.2.0-debug-act"

echo "🔧 Creating debug tag: ${DEBUG_TAG}"
echo "📋 Current tag: ${CURRENT_TAG}"

# Check if tag already exists
if git rev-parse "${DEBUG_TAG}" >/dev/null 2>&1; then
    echo "⚠️  Tag ${DEBUG_TAG} already exists. Deleting it..."
    git tag -d "${DEBUG_TAG}" || true
    git push origin ":refs/tags/${DEBUG_TAG}" || true
fi

# Create new tag from current HEAD
echo "✅ Creating tag ${DEBUG_TAG} from current HEAD..."
git tag -a "${DEBUG_TAG}" -m "Debug version with extensive logging for act/GitHub Actions debugging

- Added extensive logging to rsk_wallet.rs::run_cast_send_local
- Logs cast version, RPC connectivity checks, timing, stdout/stderr
- Helps debug cast send failures in act environment"

# Show the tag
git show "${DEBUG_TAG}" --no-patch

echo ""
echo "✅ Tag created: ${DEBUG_TAG}"
echo ""
echo "To push the tag:"
echo "  git push origin ${DEBUG_TAG}"
echo ""
echo "To use in act workflow, set client_ref input to: ${DEBUG_TAG}"
