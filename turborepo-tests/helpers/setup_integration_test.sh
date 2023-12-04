#!/usr/bin/env bash

TARGET_DIR=$1
FIXTURE_NAME="${2-basic_monorepo}"
PACKAGE_MANAGER="$3"

# TOOD: what is this for?
TMPDIR=$(mktemp -d)

THIS_DIR=$(dirname "${BASH_SOURCE[0]}")
MONOREPO_ROOT_DIR="$THIS_DIR/../.."
TURBOREPO_TESTS_DIR="${MONOREPO_ROOT_DIR}/turborepo-tests"

"${TURBOREPO_TESTS_DIR}/helpers/copy_fixture.sh" "${TARGET_DIR}" "${FIXTURE_NAME}" "${TURBOREPO_TESTS_DIR}/integration/tests/_fixtures"
"${TURBOREPO_TESTS_DIR}/helpers/setup_git.sh" ${TARGET_DIR}
"${TURBOREPO_TESTS_DIR}/helpers/setup_package_manager.sh" ${TARGET_DIR} "$PACKAGE_MANAGER"

# Install dependencies with the given package manager
PACKAGE_MANAGER_NAME="npm"
if [ "$PACKAGE_MANAGER" != "" ]; then
  PACKAGE_MANAGER_NAME=$(echo "$PACKAGE_MANAGER" | sed 's/@.*//')
fi

"${TURBOREPO_TESTS_DIR}/helpers/install_deps.sh" "$PACKAGE_MANAGER_NAME"

# Set TURBO env var, it is used by tests to run the binary
if [[ "${OSTYPE}" == "msys" ]]; then
  EXT=".exe"
else
  EXT=""
fi

TURBO=${MONOREPO_ROOT_DIR}/target/debug/turbo${EXT}
VERSION=${MONOREPO_ROOT_DIR}/version.txt
