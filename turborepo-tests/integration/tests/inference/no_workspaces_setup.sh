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

cp -a ${SCRIPT_DIR}/../../fixtures/inference/no_workspaces/. ${TARGET_DIR}/
${SETUP_GIT_SCRIPT} ${TARGET_DIR}
install_deps "${TARGET_DIR}"

${SETUP_GIT_SCRIPT} ${TARGET_DIR}/parent
install_deps "${TARGET_DIR}/parent"

${SETUP_GIT_SCRIPT} ${TARGET_DIR}/parent/child
install_deps "${TARGET_DIR}/parent/child"
