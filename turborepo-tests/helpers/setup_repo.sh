#!/bin/bash

THIS_SCRIPT=$(dirname "${BASH_SOURCE[0]}")
TURBOREPO_TESTS_DIR="$THIS_SCRIPT/.."

TURBOREPO_INTEGRATION_TESTS_DIR="${TURBOREPO_TESTS_DIR}/integration/tests"

# Run global setup script. Using source means that it executes the script in the current
# shell instead of a subshell, so env vars are preserved.
source "${TURBOREPO_TESTS_DIR}/helpers/setup.sh"

TARGET_DIR="$PWD"

# Copy over all the files from the fixture into PWD.
# FIXTURE_NAME should already be set.
cp -a "${TURBOREPO_INTEGRATION_TESTS_DIR}/_fixtures/${FIXTURE_NAME}/." "${TARGET_DIR}/"

# Initialize git repo. This also runs npm install for some reason.
"${TURBOREPO_TESTS_DIR}/helpers/setup_git.sh" "${TARGET_DIR}"

# Update package manager
if [ "$3" != "" ]; then
  # Use jq to write a new file with a .packageManager field set and then
  # Overwrite original package.json. For some reason the command above won't send its output
  # directly to the original file.
  jq --arg pm "$3" '.packageManager = $pm' "$TARGET_DIR/package.json" > "$TARGET_DIR/package.json.new"
  mv "$TARGET_DIR/package.json.new" "$TARGET_DIR/package.json"
  git commit -am "Update package manager" --quiet
fi
