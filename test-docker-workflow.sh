#!/usr/bin/env bash

# Test script for Docker release workflow using Act
set -e

echo "🧪 Testing Docker Release Workflow with Act"
echo "============================================="

# Check if act is installed
if ! command -v act &> /dev/null; then
    echo "❌ Act is not installed. Install it with: brew install act"
    exit 1
fi

# Check if Docker is running
if ! docker info &> /dev/null; then
    echo "❌ Docker is not running. Please start Docker first."
    exit 1
fi

echo "✅ Act and Docker are available"
echo ""

# Test 1: Dry run with all workflows
echo "🔍 Test 1: Dry run all workflows"
echo "--------------------------------"
act --dryrun --container-architecture linux/amd64
echo ""

# Test 2: Dry run only Docker release workflow
echo "🔍 Test 2: Dry run Docker release workflow only"
echo "-----------------------------------------------"
act push --workflows .github/workflows/docker-release.yml --dryrun --container-architecture linux/amd64
echo ""

# Test 3: Simulate tag push (dry run)
echo "🔍 Test 3: Simulate tag push (dry run)"
echo "--------------------------------------"
act push --workflows .github/workflows/docker-release.yml --dryrun --container-architecture linux/amd64 --eventpath <(echo '{"ref": "refs/tags/v1.0.0-test"}')
echo ""

echo "✅ All tests completed successfully!"
echo ""
echo "🚀 To test with a real tag:"
echo "   git tag v1.0.0-test"
echo "   git push origin v1.0.0-test"
echo ""
echo "🔧 To run without dry run (WARNING: will actually build images):"
echo "   act push --workflows .github/workflows/docker-release.yml --container-architecture linux/amd64"
