#!/bin/bash

THIS_SCRIPT=$(dirname "${BASH_SOURCE[0]}")
TURBOREPO_TESTS_DIR="$THIS_SCRIPT/../../.."
TURBOREPO_INTEGRATION_TESTS_DIR="${THIS_SCRIPT}/.."

# Run global setup script
"${TURBOREPO_TESTS_DIR}/helpers/setup.sh"

# Copy over all the files from the fixture into PWD.
FIXTURE_NAME="${1-basic_monorepo}"
cp -a "${TURBOREPO_INTEGRATION_TESTS_DIR}/_fixtures/${FIXTURE_NAME}/." "${PWD}/"

# Initialize git repo. This also runs npm install for some reason.
"${TURBOREPO_TESTS_DIR}/helpers/setup_git.sh" "${PWD}"

# Update package manager
if [ "$3" != "" ]; then
  # Use jq to write a new file with a .packageManager field set and then
  # Overwrite original package.json. For some reason the command above won't send its output
  # directly to the original file.
  jq --arg pm "$3" '.packageManager = $pm' "$PWD/package.json" > "$PWD/package.json.new"
  mv "$PWD/package.json.new" "$PWD/package.json"
  git commit -am "Update package manager" --quiet
fi
