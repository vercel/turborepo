#!/bin/bash
set -e

exampleName=$1
pkgManager=$2

# Copy the example dir over to the test dir that prysk puts you in
SCRIPT_DIR=$(dirname "${BASH_SOURCE[0]}")
MONOREPO_ROOT_DIR="$SCRIPT_DIR/../../.."
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
  set_package_manager "$NPM_PACKAGE_MANAGER_VALUE"
  npm install > /dev/null
elif [ "$pkgManager" == "pnpm" ]; then
  set_package_manager "$PNPM_PACKAGE_MANAGER_VALUE"
  pnpm install > /dev/null
elif [ "$pkgManager" == "yarn" ]; then
  set_package_manager "$YARN_PACKAGE_MANAGER_VALUE"
  yarn install > /dev/null
fi

# Delete .git directory if it's there, we'll set up a new git repo
[ ! -d .git ] || rm -rf .git

# Second arg passed is false, which will skip the npm install in setup_git.sh
"$MONOREPO_ROOT_DIR/turborepo-tests/helpers/setup_git.sh" "${TARGET_DIR}" "false"
