#!/bin/bash
set -e

exampleName=$1
pkgManager=$2

# Copy the example dir over to the test dir that prysk puts you in
SCRIPT_DIR=$(dirname "${BASH_SOURCE[0]}")
EXAMPLE_DIR="../examples/$exampleName"
TARGET_DIR="$(pwd)"
cp -a "${SCRIPT_DIR}/$EXAMPLE_DIR/." "${TARGET_DIR}/"


function set_package_manager() {
  cat package.json | jq ".packageManager=\"$1\"" | sponge package.json
}

# Set the packageManger version and install from lockfile
NPM_PACKAGE_MANAGER_VALUE="npm@8.12.1"
PNPM_PACKAGE_MANAGER_VALUE="pnpm@7.15.0"
YARN_PACKAGE_MANAGER_VALUE="yarn@1.22.17"
if [ "$pkgManager" == "npm" ]; then
  set_package_manager "$NPM_PACKAGE_MANAGER_VALUE"
  if [[ -f "package-lock.json" ]]; then
    npm ci > /dev/null
  fi
elif [ "$pkgManager" == "pnpm" ]; then
  set_package_manager "$PNPM_PACKAGE_MANAGER_VALUE"
  if [[ -f "pnpm-lock.yaml" ]]; then
    pnpm install --frozen-lockfile > /dev/null
  fi
elif [ "$pkgManager" == "yarn" ]; then
  set_package_manager "$YARN_PACKAGE_MANAGER_VALUE"
  if [[ -f "yarn.lock" ]]; then
    yarn install --frozen-lockfile > /dev/null
  fi
fi

# $TESTDIR is set by prysk to be the directory the test script is in
# (not this setup.sh script, but it happens to be the same.
SOURCE_TURBO_DIR="$TESTDIR/../cli"
TURBO_VERSION_FILE="${SOURCE_TURBO_DIR}/../version.txt"
# Change package.json in the example directory to point to @canary if our branch is currently at that version
TURBO_TAG=$(cat "$TURBO_VERSION_FILE" | sed -n '2 p')
if [ "$TURBO_TAG" == "canary" ]; then
  cat package.json | jq '.devDependencies.turbo = "canary"' | sponge package.json
fi

# Install to get correct version of turbo
if [ "$pkgManager" == "npm" ]; then
  npm install > /dev/null
elif [ "$pkgManager" == "pnpm" ]; then
  pnpm install > /dev/null
elif [ "$pkgManager" == "yarn" ]; then
  yarn install > /dev/null
fi

# Delete .git directory if it's there, we'll set up a new git repo
[ ! -d .git ] || rm -rf .git
"${SCRIPT_DIR}/../cli/integration_tests/setup_git.sh" "${TARGET_DIR}" --skip-install
