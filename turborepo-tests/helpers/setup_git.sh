#!/usr/bin/env bash

TARGET_DIR=$1

# If a second parameter isn't passed, default to true
SHOULD_INSTALL=${2:-true}

git init ${TARGET_DIR} --quiet --initial-branch=main
GIT_ARGS="--git-dir=${TARGET_DIR}/.git --work-tree=${TARGET_DIR}"
git ${GIT_ARGS} config user.email "turbo-test@example.com"
git ${GIT_ARGS} config user.name "Turbo Test"
echo "script-shell=$(which bash)" > ${TARGET_DIR}/.npmrc

if [ $SHOULD_INSTALL == "true" ]; then
  npm --prefix=${TARGET_DIR} install --silent
fi

git ${GIT_ARGS} add .
git ${GIT_ARGS} commit -m "Initial" --quiet
