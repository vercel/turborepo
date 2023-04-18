#!/bin/bash

SCRIPT_DIR=$(dirname ${BASH_SOURCE[0]})
TURBOREPO_TESTS_DIR="$SCRIPT_DIR/../../.."

REAL_SCRIPT="${TURBOREPO_TESTS_DIR}/helpers/setup_git.sh"
. "${REAL_SCRIPT}"
