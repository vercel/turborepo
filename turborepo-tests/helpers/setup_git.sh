#!/usr/bin/env bash

TARGET_DIR=$1

git init ${TARGET_DIR} --quiet --initial-branch=main

GIT_ARGS="--git-dir=${TARGET_DIR}/.git --work-tree=${TARGET_DIR}"
git ${GIT_ARGS} config user.email "turbo-test@example.com"
git ${GIT_ARGS} config user.name "Turbo Test"

# https://docs.npmjs.com/cli/v9/using-npm/config#script-shell
# Setting script-shell=bash for consistency. We can provide the name of the
# shell rather than the full path and npm will find it on its own on each platform.
echo "script-shell=bash" > ${TARGET_DIR}/.npmrc

git ${GIT_ARGS} add .
git ${GIT_ARGS} commit -m "Initial" --quiet
