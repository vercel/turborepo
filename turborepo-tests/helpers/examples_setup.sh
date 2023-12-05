#!/bin/bash

set -eo pipefail

FIXTURE_NAME=$1
pkgManager=$2
pkgManagerWithVersion=$3

THIS_DIR=$(dirname "${BASH_SOURCE[0]}")
MONOREPO_ROOT_DIR="$THIS_DIR/../.."
TURBOREPO_TESTS_DIR="${MONOREPO_ROOT_DIR}/turborepo-tests"

TARGET_DIR="$(pwd)"

"${TURBOREPO_TESTS_DIR}/helpers/copy_fixture.sh" "${TARGET_DIR}" "${FIXTURE_NAME}" "${MONOREPO_ROOT_DIR}/examples"

# cleanup lockfiles so we can install from scratch
[ ! -f yarn.lock ] || mv yarn.lock yarn.lock.bak
[ ! -f pnpm-lock.yaml ] || mv pnpm-lock.yaml pnpm-lock.yaml.bak
[ ! -f package-lock.json ] || mv package-lock.json package-lock.json.bak

# Delete .git directory if it's there, we'll set up a new git repo
[ ! -d .git ] || rm -rf .git
"${TURBOREPO_TESTS_DIR}/helpers/setup_git.sh" "${TARGET_DIR}"
"${TURBOREPO_TESTS_DIR}/helpers/setup_package_manager.sh" "${TARGET_DIR}" "$pkgManagerWithVersion"
"${TURBOREPO_TESTS_DIR}/helpers/install_deps.sh" "$pkgManager"

# Set the TURBO_BINARY_PATH env var. The examples themselves invoke the locally installed turbo,
# but turbo has an internal feature that will look for this environment variable and use it if it's set.
# This is our way of running a locally built turbo version in our examples/ instead of the version
# that is installed in the example's node_modules.
if [ "${OSTYPE}" == "msys" ]; then
  EXT=".exe"
else
  EXT=""
fi
export TURBO_BINARY_PATH=${MONOREPO_ROOT_DIR}/target/debug/turbo${EXT}
