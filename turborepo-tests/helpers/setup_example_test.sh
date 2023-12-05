#!/bin/bash

set -eo pipefail

FIXTURE_NAME=$1
PACKAGE_MANAGER_NAME=$2 # e.g. "npm"
PACKAGE_MANAGER=$3      # e.g. yarn@1.22.17

# Copy the example dir over to the test dir that prysk puts you in
SCRIPT_DIR=$(dirname "${BASH_SOURCE[0]}")
MONOREPO_ROOT_DIR="$SCRIPT_DIR/../.."
TURBOREPO_TESTS_DIR="${MONOREPO_ROOT_DIR}/turborepo-tests"
FIXTURES_DIR="$MONOREPO_ROOT_DIR/examples"

TARGET_DIR="$(pwd)"

cp -a "$"${FIXTURES_DIR}/${FIXTURE_NAME}"/." "${TARGET_DIR}/"

# cleanup lockfiles so we can install from scratch
[ ! -f yarn.lock ] || mv yarn.lock yarn.lock.bak
[ ! -f pnpm-lock.yaml ] || mv pnpm-lock.yaml pnpm-lock.yaml.bak
[ ! -f package-lock.json ] || mv package-lock.json package-lock.json.bak

# Delete .git directory if it's there, we'll set up a new git repo
[ ! -d .git ] || rm -rf .git
"${TURBOREPO_TESTS_DIR}/helpers/setup_git.sh" "${TARGET_DIR}"
"${TURBOREPO_TESTS_DIR}/helpers/setup_package_manager.sh" "${TARGET_DIR}" "$PACKAGE_MANAGER"
"${TURBOREPO_TESTS_DIR}/helpers/install_deps.sh" "$PACKAGE_MANAGER_NAME"

# Set TURBO_BINARY_PATH env var.
if [ "${OSTYPE}" == "msys" ]; then
  EXT=".exe"
else
  EXT=""
fi
export TURBO_BINARY_PATH=${MONOREPO_ROOT_DIR}/target/debug/turbo${EXT}
