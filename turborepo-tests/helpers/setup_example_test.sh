#!/usr/bin/env bash

set -eo pipefail

export TURBO_TELEMETRY_MESSAGE_DISABLED=1
export TURBO_DOWNLOAD_LOCAL_ENABLED=0

# Start by figuring out which example we're testing and its package manager
example_path=$1
package_manager=$2

if [ -z "$example_path" ]; then
  echo "No example path was provided"
  exit 1
fi

if [ -z "$package_manager" ]; then
  echo "No package manager was provided"
  exit 1
fi

echo "node --version: $(node --version)"

# Use the right command for each package manager
if [ "$package_manager" == "npm" ]; then
  package_manager_command="npx @turbo/workspaces convert . npm || true && npm install"
elif [ "$package_manager" == "pnpm" ]; then
  package_manager_command="npx @turbo/workspaces convert . pnpm || true && pnpm install"
elif [ "$package_manager" == "yarn" ]; then
  package_manager_command="npx @turbo/workspaces convert . yarn || true && yarn"
fi

# All examples implement these two tasks
# and it's reasonable to assume that they will continue to do so
turbo_command="turbo build lint"

# Head into a temporary directory
mkdir -p ../../examples-tests-tmp
cd ../../examples-tests-tmp

# Start up a fresh directory for the test
rm -rf "$example_path" || true
rsync -avq \
  --exclude='node_modules' \
  --exclude="dist" \
  --exclude=".turbo" \
  --exclude=".expo" \
  --exclude=".cache" \
  --exclude=".next" \
  "../examples/$example_path" "."

cd "$example_path"
"../../turborepo-tests/helpers/setup_git.sh" .

# Make /tmp dir for writing dump logs
mkdir -p ./tmp
echo "/tmp/" >>".gitignore"

# Simulating the user's first run and dumping logs to a file
$package_manager --version >./tmp/package_manager_version.txt
$package_manager_command
# $package_manager_command >./tmp/install.txt 2>&1
$turbo_command >./tmp/run-1.txt

# We don't want to hit cache on first run because we're acting like a user.
# A user would never hit cache on first run. Why should we?
if grep -q ">>> FULL TURBO" ./tmp/run-1.txt; then
  echo "[ERROR] A 'FULL TURBO' was found. This test must be misconfigured since it hit a cache on what was expected to be the very first run."
  echo "Dumping logs:"
  echo ""
  cat ./tmp/run-1.txt >&2
  exit 1
fi

# Simulating the user's second run
$turbo_command >./tmp/run-2.txt

# Make sure the user hits FULL TURBO on the second go
if ! grep -q ">>> FULL TURBO" ./tmp/run-2.txt; then
  echo "[ERROR] No 'FULL TURBO' was found.  This indicateds that at least one 'cache miss' occurred on the second run when all tasks were expected to be 'cache hit'."
  echo "Dumping logs:"
  echo ""
  cat ./tmp/run-2.txt >&2
  exit 1
fi
