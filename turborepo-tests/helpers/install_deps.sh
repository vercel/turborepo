#!/usr/bin/env bash

set -eo pipefail

# TODO: Should we default to pnpm here?
PACKAGE_MANAGER=${1-npm}

if [ "$PACKAGE_MANAGER" == "npm" ]; then
  npm install > /dev/null 2>&1
elif [ "$PACKAGE_MANAGER" == "pnpm" ]; then
  pnpm install > /dev/null 2>&1
elif [ "$PACKAGE_MANAGER" == "yarn" ]; then
  # Pass a --cache-folder here because yarn seems to have trouble
  # running multiple yarn installs at the same time and we are running
  # examples tests in parallel. https://github.com/yarnpkg/yarn/issues/1275
  yarn install --cache-folder="$PWD/.yarn-cache" > /dev/null 2>&1

  # And ignore this new cache folder from the new git repo we're about to create.
  echo ".yarn-cache" >> .gitignore
fi

git add .
git commit -am "Install dependencies" --quiet
