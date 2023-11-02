#!/bin/bash

SCRIPT_DIR=$(dirname ${BASH_SOURCE[0]})
FIXTURE="_fixtures/${2-basic_monorepo}"
TURBOREPO_TESTS_DIR="$SCRIPT_DIR/../../.."
TURBOREPO_INTEGRATION_TESTS_DIR="${SCRIPT_DIR}/.."

TARGET_DIR=$1
cp -a "${TURBOREPO_INTEGRATION_TESTS_DIR}/$FIXTURE/." "${TARGET_DIR}/"

echo "before git setup"
cat -vet "${TARGET_DIR}/foo.txt"

echo "package.json before npm install"
cat -vet "${TARGET_DIR}/package.json"

${TURBOREPO_TESTS_DIR}/helpers/setup_git.sh ${TARGET_DIR}

echo "package.json after npm install"
cat -vet "${TARGET_DIR}/package.json"

echo "after git setup"
cat -vet "${TARGET_DIR}/foo.txt"

# Update package manager
if [ "$3" != "" ]; then
  # Use jq to write a new file with a .packageManager field set and then
  # Overwrite original package.json. For some reason the command above won't send its output
  # directly to the original file.
  jq --arg pm "$3" '.packageManager = $pm' "$TARGET_DIR/package.json" > "$TARGET_DIR/package.json.new"
  mv "$TARGET_DIR/package.json.new" "$TARGET_DIR/package.json"
  git commit -am "Update package manager" --quiet
fi
