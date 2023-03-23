#!/usr/bin/env bash

TARGET_DIR=$1
SKIP_INSTALL=$2
git init ${TARGET_DIR} --quiet --initial-branch=main
GIT_ARGS="--git-dir=${TARGET_DIR}/.git --work-tree=${TARGET_DIR}"
git ${GIT_ARGS} config user.email "turbo-test@example.com"
git ${GIT_ARGS} config user.name "Turbo Test"
echo "script-shell=$(which bash)" > ${TARGET_DIR}/.npmrc
# Some examples don't use npm
if [[ "$SKIP_INSTALL" != "--skip-install" ]]; then
  npm --prefix=${TARGET_DIR} install --silent
fi
git ${GIT_ARGS} add .
git ${GIT_ARGS} commit -m "Initial" --quiet
