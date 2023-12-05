#!/usr/bin/env bash

set -eo pipefail

FIXTURE_NAME="${1-basic_monorepo}"
PACKAGE_MANAGER="$2"

SCRIPT_DIR=$(dirname "${BASH_SOURCE[0]}")
MONOREPO_ROOT_DIR="${SCRIPT_DIR}/../.."
TURBOREPO_TESTS_DIR="${MONOREPO_ROOT_DIR}/turborepo-tests"
FIXTURES_DIR="${TURBOREPO_TESTS_DIR}/integration/fixtures"

# TODO: what is this for?
TMPDIR=$(mktemp -d)

cp -a "${FIXTURES_DIR}/$FIXTURE_NAME/." "${TARGET_DIR}/"

TARGET_DIR="$(pwd)"

"${TURBOREPO_TESTS_DIR}/helpers/copy_fixture.sh" "${TARGET_DIR}" "${FIXTURE_NAME}" "${TURBOREPO_TESTS_DIR}/integration/fixtures"
"${TURBOREPO_TESTS_DIR}/helpers/setup_git.sh" ${TARGET_DIR}
"${TURBOREPO_TESTS_DIR}/helpers/setup_package_manager.sh" ${TARGET_DIR} "$PACKAGE_MANAGER"

# Install dependencies with the given package manager
PACKAGE_MANAGER_NAME="npm"
if [ "$PACKAGE_MANAGER" != "" ]; then
  PACKAGE_MANAGER_NAME=$(echo "$PACKAGE_MANAGER" | sed 's/@.*//')
fi

"${TURBOREPO_TESTS_DIR}/helpers/install_deps.sh" "$PACKAGE_MANAGER_NAME"

if [[ "${OSTYPE}" == "msys" ]]; then
  EXT=".exe"
else
  EXT=""
fi

export TURBO=${MONOREPO_ROOT_DIR}/target/debug/turbo${EXT}
