#!/bin/bash

SCRIPT_DIR=$(dirname ${BASH_SOURCE[0]})
TARGET_DIR=$1
FIXTURE_DIR=$2
cp -a ${SCRIPT_DIR}/monorepo/. ${TARGET_DIR}/
cp -a ${SCRIPT_DIR}/${FIXTURE_DIR}/. ${TARGET_DIR}/
#  Setup git
git init ${TARGET_DIR} --quiet
GIT_ARGS="--git-dir=${TARGET_DIR}/.git --work-tree=${TARGET_DIR}"
git ${GIT_ARGS} config user.email "turbo-test@example.com"
git ${GIT_ARGS} config user.name "Turbo Test"
git ${GIT_ARGS} add .
git ${GIT_ARGS} commit -m "Initial" --quiet

