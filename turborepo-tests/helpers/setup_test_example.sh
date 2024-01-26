#!/usr/bin/env bash
# Start by figuring out which example we're testing and its package manager
example_path=$1
package_manager=$2

# Use the right command for each package manager
if [ "$package_manager" == "npm" ]; then
  package_manager_command="npm install"
  update_turbo_version="npm install turbo@latest -w"
elif [ "$package_manager" == "pnpm" ]; then
  package_manager_command="pnpm install"
  update_turbo_version="pnpm install turbo@latest -w"
elif [ "$package_manager" == "yarn" ]; then
  package_manager_command="yarn"
  update_turbo_version="yarn upgrade turbo@latest -W"
fi

# All examples implement these two tasks
# and it's reasonable to assume that they will continue to do so
turbo_command="turbo build lint --output-logs=errors-only"

# Head into the example directory
cd $example_path

# Isolate the example from the rest of the repo from Git's perspective
"../../turborepo-tests/helpers/setup_git.sh . -n"  > /dev/null 2>&1

# Let's also isolate from turbo's perspective
rm -rf .turbo/ node_modules/ || true

# Simulating the user's first run and dumping logs to a file
$package_manager_command > ./tmp/first-install.txt 2>&1
$turbo_command > ./tmp/grep-me-for-miss.txt

# We do not want to hit cache on first run because we're acting like a user.
# A user will never hit cache on first run. Why should we?
if grep -q ">>> FULL TURBO" ./tmp/grep-me-for-miss.txt; then
  echo "A FULL TURBO was found. This test is misconfigured (since it can hit a cache)."
  echo "Dumping logs:"
  cat ./tmp/grep-me-for-miss.txt >&2
  exit 1
fi

# Make sure the tmp directory exists
mkdir -p ./tmp

# Simulating the user's second run
$package_manager_command > ./tmp/second-install.txt 2>&1
$turbo_command > ./tmp/grep-me-for-hit.txt

# Make sure the task hit a FULL TURBO
if ! grep -q ">>> FULL TURBO" ./tmp/grep-me-for-hit.txt; then
  echo "No FULL TURBO was found. Dumping logs:"
  cat ./tmp/grep-me-for-hit.txt >&2
  exit 1
fi
