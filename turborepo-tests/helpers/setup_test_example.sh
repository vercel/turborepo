#!/usr/bin/env bash
# Start by figuring out which example we're testing and its package manager
example_path=$1
package_manager=$2

# If you need to do some debugging, you can set these to true
debug=false
force=false

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
./helpers/setup_git.sh .

if [ $force == true ]; then
  echo "Forcing execution of tasks on first run..."
fi

# Do the work!
if [ $debug == true ]; then
  $package_manager_command
  $update_turbo_version
  [[ $force == true ]] && $turbo_command --force || $turbo_command
else
  $package_manager_command > /dev/null 2>&1
  $update_turbo_version > /dev/null 2>&1
  [[ $force == true ]] && $turbo_command --force || $turbo_command > /dev/null
fi

# Make sure the tmp directory exists
mkdir -p ./tmp
$turbo_command > ./tmp/grep-me.txt

# Make sure the task hit a FULL TURBO
if ! grep -q ">>> FULL TURBO" ./tmp/grep-me.txt; then
  echo "No FULL TURBO was found."
  echo "Dumping logs:"
  cat ./tmp/grep-me.txt >&2
  exit 1
fi
