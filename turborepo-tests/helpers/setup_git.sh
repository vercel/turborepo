#!/usr/bin/env bash

TARGET_DIR=$1
# If a second parameter isn't passed, default to true
SHOULD_INSTALL=${2:-true}

git init ${TARGET_DIR} --quiet --initial-branch=main

GIT_ARGS="--git-dir=${TARGET_DIR}/.git --work-tree=${TARGET_DIR}"
git ${GIT_ARGS} config user.email "turbo-test@example.com"
git ${GIT_ARGS} config user.name "Turbo Test"


# https://docs.npmjs.com/cli/v9/using-npm/config#script-shell
# Setting script-shell=bash for consistency. We can provide the name of the
# shell rather than the full path and npm will find it on its own on each platform.
echo "script-shell=bash" > ${TARGET_DIR}/.npmrc

if [ $SHOULD_INSTALL == "true" ]; then
  pushd ${TARGET_DIR} > /dev/null || exit 1
  npm install --silent
  popd > /dev/null || exit 1
fi

git ${GIT_ARGS} add .
git ${GIT_ARGS} commit -m "Initial" --quiet
