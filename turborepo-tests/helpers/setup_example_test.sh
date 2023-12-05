#!/bin/bash

set -eo pipefail

FIXTURE_NAME=$1
PACKAGE_MANAGER_NAME=$2 # e.g. "npm"
PACKAGE_MANAGER=$3      # e.g. yarn@1.22.17

THIS_DIR=$(dirname "${BASH_SOURCE[0]}")
MONOREPO_ROOT_DIR="$THIS_DIR/../.."
TURBOREPO_TESTS_DIR="${MONOREPO_ROOT_DIR}/turborepo-tests"

TARGET_DIR="$(pwd)"

"${TURBOREPO_TESTS_DIR}/helpers/copy_fixture.sh" "${TARGET_DIR}" "${FIXTURE_NAME}" "${MONOREPO_ROOT_DIR}/examples"
"${TURBOREPO_TESTS_DIR}/helpers/setup_git.sh" "${TARGET_DIR}"
"${TURBOREPO_TESTS_DIR}/helpers/setup_package_manager.sh" "${TARGET_DIR}" "$PACKAGE_MANAGER"
"${TURBOREPO_TESTS_DIR}/helpers/install_deps.sh" "$PACKAGE_MANAGER_NAME"

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
