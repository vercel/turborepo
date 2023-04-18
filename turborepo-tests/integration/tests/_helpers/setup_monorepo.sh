#!/bin/bash

SCRIPT_DIR=$(dirname ${BASH_SOURCE[0]})
TARGET_DIR=$1
FIXTURE="_fixtures/${2-basic_monorepo}"
cp -a ${SCRIPT_DIR}/../$FIXTURE/. ${TARGET_DIR}/
${SCRIPT_DIR}/setup_git.sh ${TARGET_DIR}

# Update package manager
if [ "$3" != "" ]; then
  # Use jq to write a new file with a .packageManager field set and then
  # Overwrite original package.json. For some reason the command above won't send its output
  # directly to the original file.
  jq --arg pm "$3" '.packageManager = $pm' "$TARGET_DIR/package.json" > "$TARGET_DIR/package.json.new"
  mv "$TARGET_DIR/package.json.new" "$TARGET_DIR/package.json"
  git commit -am "Update package manager" --quiet
fi
