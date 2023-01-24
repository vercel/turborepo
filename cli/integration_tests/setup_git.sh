#!/usr/bin/env bash

TARGET_DIR=$1
git init ${TARGET_DIR} --quiet
GIT_ARGS="--git-dir=${TARGET_DIR}/.git --work-tree=${TARGET_DIR}"
git ${GIT_ARGS} config user.email "turbo-test@example.com"
git ${GIT_ARGS} config user.name "Turbo Test"
echo "script-shell=$(which bash)" > ${TARGET_DIR}/.npmrc
npm --prefix=${TARGET_DIR} install --silent
git ${GIT_ARGS} add .
git ${GIT_ARGS} commit -m "Initial" --quiet
