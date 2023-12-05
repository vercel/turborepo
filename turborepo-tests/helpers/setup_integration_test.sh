#!/usr/bin/env bash

set -eo pipefail

FIXTURE_NAME="${1-basic_monorepo}"
PACKAGE_MANAGER="$2"

# TOOD: what is this for?
TMPDIR=$(mktemp -d)

THIS_DIR=$(dirname "${BASH_SOURCE[0]}")
MONOREPO_ROOT_DIR="$THIS_DIR/../.."
TURBOREPO_TESTS_DIR="${MONOREPO_ROOT_DIR}/turborepo-tests"

TARGET_DIR="$(pwd)"

"${TURBOREPO_TESTS_DIR}/helpers/copy_fixture.sh" "${TARGET_DIR}" "${FIXTURE_NAME}" "${TURBOREPO_TESTS_DIR}/integration/fixtures"
"${TURBOREPO_TESTS_DIR}/helpers/setup_git.sh" "${TARGET_DIR}"
"${TURBOREPO_TESTS_DIR}/helpers/setup_package_manager.sh" "${TARGET_DIR}" "$PACKAGE_MANAGER"

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

# Undo the set -eo pipefail at the top of this script
# This script is called with a leading ".", which means that it does not fork
# the process, so the set -eo pipefail would affect the calling script.
# Some of our tests actually assert non-zero exit codes, and we don't want to
# abort the test in those cases. So we undo the set -eo pipefail here.
set +eo pipefail
