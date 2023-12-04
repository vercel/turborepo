#!/usr/bin/env bash

THIS_DIR=$(dirname "${BASH_SOURCE[0]}")
MONOREPO_ROOT_DIR="${THIS_DIR}/../.."

# TODO: what is this for?
TMPDIR=$(mktemp -d)

TARGET_DIR=$1
FIXTURE_NAME="${2-basic_monorepo}"
PACKAGE_MANAGER="$3"

SCRIPT_DIR=$(dirname ${BASH_SOURCE[0]})
FIXTURE="_fixtures/${FIXTURE_NAME}"
TURBOREPO_TESTS_DIR="$SCRIPT_DIR/.."
TURBOREPO_INTEGRATION_TESTS_DIR="${TURBOREPO_TESTS_DIR}/integration/tests"

cp -a "${TURBOREPO_INTEGRATION_TESTS_DIR}/$FIXTURE/." "${TARGET_DIR}/"

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

TURBO=${MONOREPO_ROOT_DIR}/target/debug/turbo${EXT}
