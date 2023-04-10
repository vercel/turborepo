#!/bin/bash

SCRIPT_DIR=$(dirname ${BASH_SOURCE[0]})
TARGET_DIR=$1
cp -a ${SCRIPT_DIR}/../_fixtures/invalid_turbo_json/. ${TARGET_DIR}/
${SCRIPT_DIR}/../_helpers/setup_git.sh ${TARGET_DIR}
