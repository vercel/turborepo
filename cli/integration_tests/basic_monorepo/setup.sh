#!/bin/bash

SCRIPT_DIR=$(dirname ${BASH_SOURCE[0]})
TARGET_DIR=$1
cp -a ${SCRIPT_DIR}/monorepo/. ${TARGET_DIR}/
${SCRIPT_DIR}/../setup_git.sh ${TARGET_DIR}
