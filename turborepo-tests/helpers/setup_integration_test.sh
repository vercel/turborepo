#!/usr/bin/env bash

THIS_DIR=$(dirname "${BASH_SOURCE[0]}")

. ${THIS_DIR}/setup.sh

TARGET_DIR=$1
FIXTURE_NAME="${2-basic_monorepo}"
PACKAGE_MANAGER="${3-npm}"

SCRIPT_DIR=$(dirname ${BASH_SOURCE[0]})
FIXTURE="_fixtures/${FIXTURE_NAME}"
TURBOREPO_TESTS_DIR="$SCRIPT_DIR/.."
TURBOREPO_INTEGRATION_TESTS_DIR="${TURBOREPO_TESTS_DIR}/integration/tests"

cp -a "${TURBOREPO_INTEGRATION_TESTS_DIR}/$FIXTURE/." "${TARGET_DIR}/"

"${TURBOREPO_TESTS_DIR}/helpers/setup_git.sh" ${TARGET_DIR}

# Update package manager if one was provided
if [ "$PACKAGE_MANAGER" != "" ]; then
  # Use jq to write a new file with a .packageManager field set and then
  # Overwrite original package.json. For some reason the command above won't send its output
  # directly to the original file.
  jq --arg pm "$PACKAGE_MANAGER" '.packageManager = $pm' "$TARGET_DIR/package.json" > "$TARGET_DIR/package.json.new"
  mv "$TARGET_DIR/package.json.new" "$TARGET_DIR/package.json"

  # We just created a new file. On Windows, we need to convert it to Unix line endings
  # so the hashes will be stable with what's expected in our test cases.
  if [[ "$OSTYPE" == "msys" ]]; then
    dos2unix --quiet "$TARGET_DIR/package.json"
  fi

  git commit -am "Update package manager" --quiet
fi


# Install dependencies
pushd ${TARGET_DIR} > /dev/null || exit 1
${SCRIPT_DIR}/install_deps.sh "$PACKAGE_MANAGER"
popd > /dev/null || exit 1
