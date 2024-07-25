#!/usr/bin/env bash

set -eo pipefail

FIXTURE_NAME="${1-basic_monorepo}"

# Default to version of npm installed with Node 18.20.2
PACKAGE_MANAGER="npm@10.5.0"
if [[ $2 != "" ]]; then
  PACKAGE_MANAGER="$2"
fi

THIS_DIR=$(dirname "${BASH_SOURCE[0]}")
MONOREPO_ROOT_DIR="$THIS_DIR/../.."
TURBOREPO_TESTS_DIR="${MONOREPO_ROOT_DIR}/turborepo-tests"

TARGET_DIR="$(pwd)"

# on macos, using the tmp dir set by prysk can fail, so set it
# to /tmp which is less secure (777) but wont crash
if [[ "$OSTYPE" == darwin* ]]; then
  export TMPDIR=/tmp
fi


"${TURBOREPO_TESTS_DIR}/helpers/copy_fixture.sh" "${TARGET_DIR}" "${FIXTURE_NAME}" "${TURBOREPO_TESTS_DIR}/integration/fixtures"
"${TURBOREPO_TESTS_DIR}/helpers/setup_git.sh" "${TARGET_DIR}"
"${TURBOREPO_TESTS_DIR}/helpers/setup_package_manager.sh" "${TARGET_DIR}" "$PACKAGE_MANAGER"
"${TURBOREPO_TESTS_DIR}/helpers/install_deps.sh" "$PACKAGE_MANAGER"

# Set TURBO env var, it is used by tests to run the binary
if [[ "${OSTYPE}" == "msys" ]]; then
  EXT=".exe"
else
  EXT=""
fi

export TURBO_TELEMETRY_MESSAGE_DISABLED=1
export TURBO_GLOBAL_WARNING_DISABLED=1
export TURBO_PRINT_VERSION_DISABLED=1
export TURBO=${MONOREPO_ROOT_DIR}/target/debug/turbo${EXT}

# Undo the set -eo pipefail at the top of this script
# This script is called with a leading ".", which means that it does not run
# in a new child process, so the set -eo pipefail would affect the calling script.
# Some of our tests actually assert non-zero exit codes, and we don't want to
# abort the test in those cases. So we undo the set -eo pipefail here.
set +eo pipefail
