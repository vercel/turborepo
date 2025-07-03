#!/bin/bash

# This script updates the turbo dependency in all examples that are using any package manager.

# Usage: ./scripts/update-examples-dep.sh
# Example: ./scripts/update-examples-dep.sh

set -e

# Change directory to the script's directory
cd "$(dirname "$0")"

package="turbo"

# Fetch the latest version of the package from npm
latest_version=$(npm show $package version)

echo "Upgrading $package to version $latest_version in all examples..."

# Get the list of example directories
examples="../examples"

for dir in "$examples"/*; do
  if [ -d "$dir" ]; then
    cd "$dir"
    example=$(basename "$(pwd)")
    echo $example
    if [ -e "pnpm-lock.yaml" ]; then
      echo "• Updating to $package@$latest_version using pnpm"
      pnpm up $package@$latest_version 2>&1 >/dev/null
    elif [ -e ".yarn" ]; then
      echo "• Updating to $package@$latest_version using yarn"
      yarn add $package@$latest_version 2>&1 >/dev/null
    elif [ -e "yarn.lock" ]; then
      echo "• Updating to $package@$latest_version using yarn"
      yarn upgrade $package@$latest_version --ignore-workspace-root-check 2>&1 >/dev/null
    elif [ -e "package-lock.json" ]; then
      echo "• Updating to $package@$latest_version using npm"
      npm install $package@$latest_version 2>&1 >/dev/null
    else
      echo "• No recognized package manager - skipping."
    fi
    cd - >/dev/null
    echo ""
  fi
done
