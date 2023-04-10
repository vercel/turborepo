#!/bin/bash

SCRIPT_DIR=$(dirname ${BASH_SOURCE[0]})
TARGET_DIR=$1

cp -a ${SCRIPT_DIR}/../_fixtures/inference/nested_workspaces/. ${TARGET_DIR}/
${SCRIPT_DIR}/../_helpers/setup_git.sh ${TARGET_DIR}/outer
${SCRIPT_DIR}/../_helpers/setup_git.sh ${TARGET_DIR}/outer/inner
${SCRIPT_DIR}/../_helpers/setup_git.sh ${TARGET_DIR}/outer/inner-no-turbo

${SCRIPT_DIR}/../_helpers/setup_git.sh ${TARGET_DIR}/outer-no-turbo
${SCRIPT_DIR}/../_helpers/setup_git.sh ${TARGET_DIR}/outer-no-turbo/inner
${SCRIPT_DIR}/../_helpers/setup_git.sh ${TARGET_DIR}/outer-no-turbo/inner-no-turbo
