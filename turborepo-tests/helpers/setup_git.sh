#!/usr/bin/env bash

# Use a flag to prevent setting shell
while getopts ":n" opt; do
  case ${opt} in
    n ) NO_SHELL_SET="true"
      ;;
    \? ) echo "Usage: setup_git.sh [-n] target_dir"
      ;;
  esac
done

TARGET_DIR=$1

git init ${TARGET_DIR} --quiet --initial-branch=main

GIT_ARGS="--git-dir=${TARGET_DIR}/.git --work-tree=${TARGET_DIR}"
git ${GIT_ARGS} config user.email "turbo-test@example.com"
git ${GIT_ARGS} config user.name "Turbo Test"

# https://docs.npmjs.com/cli/v9/using-npm/config#script-shell
# Setting script-shell=bash for consistency. We can provide the name of the
# shell rather than the full path and npm will find it on its own on each platform.
if [ -z "${NO_SHELL_SET}" ]; then
  echo "script-shell=bash" > ${TARGET_DIR}/.npmrc
fi

git ${GIT_ARGS} add .
git ${GIT_ARGS} commit -m "Initial" --quiet
