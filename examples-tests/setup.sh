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
  jq --arg version "canary" '.devDependencies.turbo = $version' package.json > package.json.new
  mv package.json.new package.json
fi

function set_package_manager() {
  jq --arg pm "$1" '.packageManager = $pm' package.json > package.json.new
  mv package.json.new package.json
}

# Enable corepack so that when we set the packageManager in package.json it actually makes a diference.
corepack enable

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

# We set this explicitly to stream, so we can lock into to streaming logs (i.e. not "auto") behavior.
#
# We do this because when these tests are invoked in CI (through .github/actions/test.yml), they will
# inherit the GITHUB_ACTIONS=true env var, and each of the `turbo run` invocations into behavior
# we do not want. Since prysk mainly tests log output, this extra behavior will break all the tests
# and can be unpredictable over time, as we make "auto" do more magic.
#
# Note: since these tests are invoked _through_ turbo, the ideal setup would be to pass --env-mode=strict
# so we can prevent the `GITHUB_ACTIONS` env var from being passed down here from the top level turbo.
# But as of now, this breaks our tests (and I'm not sure why). If we make that work, we can remove this
# explicit locking of log order. See PR attempt here: https://github.com/vercel/turbo/pull/5324
export TURBO_LOG_ORDER=stream
