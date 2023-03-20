#!/bin/bash

set -e

python3 -m venv .cram_env
.cram_env/bin/pip install prysk

export folder=$1
export pkgManager=$2

TEST_FILE="examples_tests/$2-$1.t"

if [ -f "$TEST_FILE" ]; then
  echo "Running $TEST_FILE"
  .cram_env/bin/prysk --shell="$(which bash)" "$TEST_FILE"
else
  echo "Could not find $TEST_FILE"
  exit 1
fi
