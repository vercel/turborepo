#!/bin/bash

SCRIPT_DIR=$(dirname ${BASH_SOURCE[0]})
TARGET_DIR=$1
TURBOREPO_TESTS_DIR="$SCRIPT_DIR/../../.."
SETUP_GIT_SCRIPT="${TURBOREPO_TESTS_DIR}/helpers/setup_git.sh"

function install_deps() {
  local dir=$1
  pushd $dir > /dev/null || exit 1
  npm install --silent
  popd > /dev/null || exit 1
  git --git-dir="${dir}/.git" --work-tree="${dir}" add .
  git --git-dir="${dir}/.git" --work-tree="${dir}" commit -m "Install dependencies" --quiet
}

cp -a ${SCRIPT_DIR}/../../fixtures/inference/nested_workspaces/. ${TARGET_DIR}/
${SETUP_GIT_SCRIPT} ${TARGET_DIR}/outer
install_deps "${TARGET_DIR}/outer"

${SETUP_GIT_SCRIPT} ${TARGET_DIR}/outer/inner
install_deps "${TARGET_DIR}/outer/inner"

${SETUP_GIT_SCRIPT} ${TARGET_DIR}/outer/inner-no-turbo
install_deps "${TARGET_DIR}/outer/inner-no-turbo"

${SETUP_GIT_SCRIPT} ${TARGET_DIR}/outer-no-turbo
install_deps "${TARGET_DIR}/outer-no-turbo"

${SETUP_GIT_SCRIPT} ${TARGET_DIR}/outer-no-turbo/inner
install_deps "${TARGET_DIR}/outer-no-turbo/inner"

${SETUP_GIT_SCRIPT} ${TARGET_DIR}/outer-no-turbo/inner-no-turbo
install_deps "${TARGET_DIR}/outer-no-turbo/inner-no-turbo"

