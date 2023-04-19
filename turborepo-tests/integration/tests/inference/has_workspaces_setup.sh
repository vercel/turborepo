#!/bin/bash

SCRIPT_DIR=$(dirname ${BASH_SOURCE[0]})
TARGET_DIR=$1

cp -a ${SCRIPT_DIR}/../_fixtures/inference/has_workspaces/. ${TARGET_DIR}/

TURBOREPO_TESTS_DIR="$SCRIPT_DIR/../../.."
${TURBOREPO_TESTS_DIR}/helpers/setup_git.sh ${TARGET_DIR}
