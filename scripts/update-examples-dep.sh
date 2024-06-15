#!/bin/bash

# This script updates a dependency in all examples that are using pnpm.

# Usage: ./scripts/update-examples-dep.sh <package> <version>
# Example: ./scripts/update-examples-dep.sh typescript

set -e

# Change directory to the script's directory
cd "$(dirname "$0")"

package="$1"
version=${2:-latest}

echo "Upgrading $package to $version in all examples..."

# Get the list of top-level directories
examples=$(find ../examples -depth 1 -type d)
lock="pnpm-lock.yaml"

for dir in $examples; do
  if [ "$dir" != "." ]; then
    cd "$dir"
    example=$(basename "$(pwd)")
    echo $example
    if [ -e "$lock" ]; then
      echo "• Updating all workspaces to $package@$version"
      # pnpm upgrade $package@latest -r
      pnpm install
    else
    # yarn doesn't have a nice recursive upgrade command, so we do those manually for now
      echo "• Not using pnpm - skipping."
    fi
    cd - > /dev/null
    echo ""
  fi
done
