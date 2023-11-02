#!/usr/bin/env bash

TARGET_DIR=$1
# If a second parameter isn't passed, default to true
SHOULD_INSTALL=${2:-true}

# Set the autocrlf option to "input", so Unix-style line endings are kept when checking out files
# on Windows, and any CRLF is converted to LF when committing files during a test also. This will
# allow file content and hashes to be hardcoded in test cases and work cross platform.
# https://www.git-scm.com/book/en/v2/Customizing-Git-Git-Configuration#_core_autocrlf
if [[ "$OSTYPE" == "msys" ]]; then
  git config --global core.autocrlf input
fi

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
