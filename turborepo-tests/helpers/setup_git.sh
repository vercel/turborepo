#!/usr/bin/env bash

TARGET_DIR=$1
# If a second parameter isn't passed, default to true
SHOULD_INSTALL=${2:-true}

git init ${TARGET_DIR} --quiet --initial-branch=main

# Set the autocrlf option to "true", that when we init this new git repo
# any new files added to the index will convert CRLF to LF. The files in the fixture
# at the start should all be LF already because they were created with LF line endings
# and in CI, on Windows, we set autocrlf to `input` to tell git to retain those line endings
# when the monorepo is cloned (see .github/workflows/test.yml). We do this after `git init`
# and set the config locally so it doesn't interfere with the global config set in test.yml.
# https://www.git-scm.com/book/en/v2/Customizing-Git-Git-Configuration#_core_autocrlf
# if [[ "$OSTYPE" == "msys" ]]; then
#   git config --local core.autocrlf input
# fi

git config --list

GIT_ARGS="--git-dir=${TARGET_DIR}/.git --work-tree=${TARGET_DIR}"
git ${GIT_ARGS} config user.email "turbo-test@example.com"
git ${GIT_ARGS} config user.name "Turbo Test"

echo "script-shell=$(which bash)" > ${TARGET_DIR}/.npmrc

if [ $SHOULD_INSTALL == "true" ]; then
  pushd ${TARGET_DIR} > /dev/null || exit 1
  npm install --silent
  popd > /dev/null || exit 1
fi

git ${GIT_ARGS} add .
git ${GIT_ARGS} commit -m "Initial" --quiet
