#!/bin/sh

SCRIPT_DIR=$(dirname $0)
TARGET_DIR=$1
cp -a ${SCRIPT_DIR}/my-pkg/* ${TARGET_DIR}/
git init ${TARGET_DIR} --quiet
GIT_ARGS="--git-dir=${TARGET_DIR}/.git --work-tree=${TARGET_DIR}"
npm --prefix=${TARGET_DIR} install --silent
git ${GIT_ARGS} add .
git ${GIT_ARGS} commit -m "Initial" --quiet
