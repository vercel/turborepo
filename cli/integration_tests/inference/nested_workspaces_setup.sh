#!/bin/bash

SCRIPT_DIR=$(dirname ${BASH_SOURCE[0]})
TARGET_DIR=$1

cp -a ${SCRIPT_DIR}/nested-workspaces/. ${TARGET_DIR}/
${SCRIPT_DIR}/../setup_git.sh ${TARGET_DIR}/outer
${SCRIPT_DIR}/../setup_git.sh ${TARGET_DIR}/outer/inner
${SCRIPT_DIR}/../setup_git.sh ${TARGET_DIR}/outer/inner-no-turbo

${SCRIPT_DIR}/../setup_git.sh ${TARGET_DIR}/outer-no-turbo
${SCRIPT_DIR}/../setup_git.sh ${TARGET_DIR}/outer-no-turbo/inner
${SCRIPT_DIR}/../setup_git.sh ${TARGET_DIR}/outer-no-turbo/inner-no-turbo
