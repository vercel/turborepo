#!/usr/bin/env bash

set -eo pipefail

# Enable corepack so that when we set the packageManager
# in package.json it actually makes a diference.
if [ "$PRYSK_TEMP" == "" ]; then
  COREPACK_INSTALL_DIR_CMD=
else
  COREPACK_INSTALL_DIR="${PRYSK_TEMP}/corepack"
  mkdir -p "${COREPACK_INSTALL_DIR}"
  export PATH=${COREPACK_INSTALL_DIR}:$PATH
  COREPACK_INSTALL_DIR_CMD="--install-directory=${COREPACK_INSTALL_DIR}"
fi
corepack enable "${COREPACK_INSTALL_DIR_CMD}"

# TODO: Should we default to pnpm here?
PACKAGE_MANAGER=${1-npm}

# If a lock file already exists, we will exit. Some fixtures already have the lock
# file. The caller is responsible for deleting this before calling this script.
# TODO: we should make the fixtures consistent in either having or not having lockfiles.
if [[ -f "package-lock.json" || -f "pnpm-lock.yaml" || -f "yarn.lock" ]]; then
  exit 0
fi

if [ "$PACKAGE_MANAGER" == "npm" ]; then
  npm install > /dev/null 2>&1
  if [[ "$OSTYPE" == "msys" ]]; then
    dos2unix --quiet "package-lock.json"
  fi

elif [ "$PACKAGE_MANAGER" == "pnpm" ]; then
  pnpm install > /dev/null 2>&1

  if [[ "$OSTYPE" == "msys" ]]; then
    dos2unix --quiet "pnpm-lock.yaml"
  fi
elif [ "$PACKAGE_MANAGER" == "yarn" ]; then
  # Pass a --cache-folder here because yarn seems to have trouble
  # running multiple yarn installs at the same time and we are running
  # examples tests in parallel. https://github.com/yarnpkg/yarn/issues/1275
  yarn install --cache-folder="$PWD/.yarn-cache" > /dev/null 2>&1

  # And ignore this new cache folder from the new git repo we're about to create.
  echo ".yarn-cache" >> .gitignore

  if [[ "$OSTYPE" == "msys" ]]; then
    dos2unix --quiet "yarn.lock"
  fi
fi

git add .

# Check if there are changes before trying to run git commit, so it doesn't exit with 0
if [[ $(git status --porcelain) ]]; then
  git commit -am "Install dependencies" --quiet > /dev/null 2>&1 || true
fi
