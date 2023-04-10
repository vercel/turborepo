#!/bin/bash

SCRIPT_DIR=$(dirname ${BASH_SOURCE[0]})
TARGET_DIR=$1
cp -a ${SCRIPT_DIR}/../_fixtures/composable_config/. ${TARGET_DIR}/
${SCRIPT_DIR}/../_helpers/setup_git.sh ${TARGET_DIR}
