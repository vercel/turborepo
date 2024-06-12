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

# Use the right command for each package manager
if [ "$package_manager" == "npm" ]; then
  package_manager_command="npm install"
elif [ "$package_manager" == "pnpm" ]; then
  package_manager_command="pnpm install"
elif [ "$package_manager" == "yarn" ]; then
  package_manager_command="yarn"
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
echo "/tmp/" >> ".gitignore"

# Simulating the user's first run and dumping logs to a file
$package_manager_command >./tmp/install.txt 2>&1
$turbo_command >./tmp/grep-me-for-miss.txt

# We don't want to hit cache on first run because we're acting like a user.
# A user would never hit cache on first run. Why should we?
if grep -q ">>> FULL TURBO" ./tmp/grep-me-for-miss.txt; then
  echo "A FULL TURBO was found. This test is misconfigured (since it can hit a cache)."
  echo "Dumping logs:"
  cat ./tmp/grep-me-for-miss.txt >&2
  exit 1
fi

# Simulating the user's second run
$turbo_command >./tmp/grep-me-for-hit.txt

# Make sure the user hits FULL TURBO on the second go
if ! grep -q ">>> FULL TURBO" ./tmp/grep-me-for-hit.txt; then
  echo "No FULL TURBO was found. Dumping logs:"
  cat ./tmp/grep-me-for-hit.txt >&2
  exit 1
fi
