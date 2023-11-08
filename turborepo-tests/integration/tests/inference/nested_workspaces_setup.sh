#!/bin/bash

SCRIPT_DIR=$(dirname ${BASH_SOURCE[0]})
TARGET_DIR=$1
TURBOREPO_TESTS_DIR="$SCRIPT_DIR/../../.."
SETUP_GIT_SCRIPT="${TURBOREPO_TESTS_DIR}/helpers/setup_git.sh"

cp -a ${SCRIPT_DIR}/../_fixtures/inference/nested_workspaces/. ${TARGET_DIR}/
${SETUP_GIT_SCRIPT} ${TARGET_DIR}/outer
${SETUP_GIT_SCRIPT} ${TARGET_DIR}/outer/inner
${SETUP_GIT_SCRIPT} ${TARGET_DIR}/outer/inner-no-turbo

${SETUP_GIT_SCRIPT} ${TARGET_DIR}/outer-no-turbo
${SETUP_GIT_SCRIPT} ${TARGET_DIR}/outer-no-turbo/inner
${SETUP_GIT_SCRIPT} ${TARGET_DIR}/outer-no-turbo/inner-no-turbo
