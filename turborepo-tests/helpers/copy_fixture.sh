#!/bin/bash
set -eo pipefail

TARGET_DIR=$1
FIXTURE="${2-basic_monorepo}"
<<<<<<< HEAD

SCRIPT_DIR=$(dirname ${BASH_SOURCE[0]})
MONOREPO_ROOT_DIR="${SCRIPT_DIR}/../.."
TURBOREPO_TESTS_DIR="${MONOREPO_ROOT_DIR}/turborepo-tests"
FIXTURES_DIR="${TURBOREPO_TESTS_DIR}/integration/fixtures"
=======
FIXTURES_DIR="$3"

# If a fixtures directory isn't provided, we use the default one we use for integration tests
if [ "$FIXTURES_DIR" == "" ]; then
  echo "Pass a fixtures directory"
  exit 1
fi
>>>>>>> main

cp -a "${FIXTURES_DIR}/$FIXTURE/." "${TARGET_DIR}/"
