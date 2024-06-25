#!/bin/bash

export TURBO_GLOBAL_WARNING_DISABLED=1
export TURBO_DOWNLOAD_LOCAL_ENABLED=0
SCRIPT_DIR=$(dirname ${BASH_SOURCE[0]})
TARGET_DIR=$1
FIXTURE_DIR=$2
cp -a ${SCRIPT_DIR}/../../fixtures/lockfile_aware_caching/. ${TARGET_DIR}/
cp -a ${SCRIPT_DIR}/${FIXTURE_DIR}/. ${TARGET_DIR}/
#  Setup git
git init ${TARGET_DIR} --quiet
GIT_ARGS="--git-dir=${TARGET_DIR}/.git --work-tree=${TARGET_DIR}"
git ${GIT_ARGS} config user.email "turbo-test@example.com"
git ${GIT_ARGS} config user.name "Turbo Test"
echo ".turbo" >> ${TARGET_DIR}/.gitignore
echo "node_modules" >> ${TARGET_DIR}/.gitignore
git ${GIT_ARGS} add .
git ${GIT_ARGS} commit -m "Initial" --quiet

