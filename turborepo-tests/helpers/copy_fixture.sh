#!/bin/bash
set -eo pipefail

TARGET_DIR=$1
FIXTURE="${2-basic_monorepo}"
FIXTURES_DIR="$3"

# If a fixtures directory isn't provided, we use the default one we use for integration tests
if [ "$FIXTURES_DIR" == "" ]; then
  echo "Pass a fixtures directory"
  exit 1
fi

cp -a "${FIXTURES_DIR}/$FIXTURE/." "${TARGET_DIR}/"
