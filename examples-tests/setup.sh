#!/bin/bash

# This script is called from within a prysk test, so pwd is already in the prysk tmp directory.

set -eo pipefail

exampleName=$1
pkgManager=$2

# Copy the example dir over to the test dir that prysk puts you in
SCRIPT_DIR=$(dirname "${BASH_SOURCE[0]}")
MONOREPO_ROOT_DIR="$SCRIPT_DIR/.."
EXAMPLE_DIR="$MONOREPO_ROOT_DIR/examples/$exampleName"

TARGET_DIR="$(pwd)"

cp -a "$EXAMPLE_DIR/." "${TARGET_DIR}/"

# cleanup lockfiles so we can install from scratch
[ ! -f yarn.lock ] || mv yarn.lock yarn.lock.bak
[ ! -f pnpm-lock.yaml ] || mv pnpm-lock.yaml pnpm-lock.yaml.bak
[ ! -f package-lock.json ] || mv package-lock.json package-lock.json.bak


TURBO_VERSION_FILE="${MONOREPO_ROOT_DIR}/version.txt"
# Change package.json in the example directory to point to @canary if our branch is currently at that version
TURBO_TAG=$(cat "$TURBO_VERSION_FILE" | sed -n '2 p')
if [ "$TURBO_TAG" == "canary" ]; then
  cat package.json | jq '.devDependencies.turbo = "canary"' | sponge package.json
fi

function set_package_manager() {
  cat package.json | jq ".packageManager=\"$1\"" | sponge package.json
}

# Set the packageManger version
NPM_PACKAGE_MANAGER_VALUE="npm@8.1.2"
PNPM_PACKAGE_MANAGER_VALUE="pnpm@6.26.1"
YARN_PACKAGE_MANAGER_VALUE="yarn@1.22.17"

if [ "$pkgManager" == "npm" ]; then
  # Note! We will packageManager for npm, but it doesn't actually change the version
  # We are effectively just removing any packageManager that's already set.
  # https://nodejs.org/api/corepack.html#how-does-corepack-interact-with-npm
  # > "While npm is a valid option in the "packageManager" property, the lack of shim will cause the global npm to be used."
  set_package_manager "$NPM_PACKAGE_MANAGER_VALUE"

  npm --version
  npm install > /dev/null 2>&1
elif [ "$pkgManager" == "pnpm" ]; then
  set_package_manager "$PNPM_PACKAGE_MANAGER_VALUE"
  pnpm --version
  pnpm install > /dev/null 2>&1
elif [ "$pkgManager" == "yarn" ]; then
  set_package_manager "$YARN_PACKAGE_MANAGER_VALUE"
  yarn --version
  # Pass a --cache-folder here because yarn seems to have trouble
  # running multiple yarn installs at the same time and we are running
  # examples tests in parallel. https://github.com/yarnpkg/yarn/issues/1275
  yarn install --cache-folder="$PWD/.yarn-cache" > /dev/null 2>&1

  # And ignore this new cache folder from the new git repo we're about to create.
  echo ".yarn-cache" >> .gitignore
fi

# Delete .git directory if it's there, we'll set up a new git repo
[ ! -d .git ] || rm -rf .git

"$MONOREPO_ROOT_DIR/turborepo-tests/helpers/setup_git.sh" "${TARGET_DIR}" "false"
