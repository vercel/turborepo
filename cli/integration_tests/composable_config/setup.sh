#!/bin/bash

SCRIPT_DIR=$(dirname ${BASH_SOURCE[0]})
TARGET_DIR=$1
TEST_DIR=$2
cp -a ${SCRIPT_DIR}/$TEST_DIR/. ${TARGET_DIR}/
${SCRIPT_DIR}/../setup_git.sh ${TARGET_DIR}
