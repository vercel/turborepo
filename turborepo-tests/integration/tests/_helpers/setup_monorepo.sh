#!/bin/bash

SCRIPT_DIR=$(dirname ${BASH_SOURCE[0]})
FIXTURE="_fixtures/${2-basic_monorepo}"
TURBOREPO_TESTS_DIR="$SCRIPT_DIR/../../.."
TURBOREPO_INTEGRATION_TESTS_DIR="${SCRIPT_DIR}/.."

TARGET_DIR=$1
cp -a "${TURBOREPO_INTEGRATION_TESTS_DIR}/$FIXTURE/." "${TARGET_DIR}/"

${TURBOREPO_TESTS_DIR}/helpers/setup_git.sh ${TARGET_DIR}

echo "npm version is:"
npm --version

# Update package manager
if [ "$3" != "" ]; then
  # Use jq to write a new file with a .packageManager field set and then
  # Overwrite original package.json. For some reason the command above won't send its output
  # directly to the original file.
  jq --arg pm "$3" '.packageManager = $pm' "$TARGET_DIR/package.json" > "$TARGET_DIR/package.json.new"
  mv "$TARGET_DIR/package.json.new" "$TARGET_DIR/package.json"

  # We just created a new file. On Windows, we need to convert it to Unix line endings
  # so the hashes will be stable with what's expected in our test cases.
  if [[ "$OSTYPE" == "msys" ]]; then
    dos2unix --quiet "$TARGET_DIR/package.json"
  fi

  git commit -am "Update package manager" --quiet
fi
