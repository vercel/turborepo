#!/bin/bash

SCRIPT_DIR=$(dirname ${BASH_SOURCE[0]})
TARGET_DIR=$1
cp -a ${SCRIPT_DIR}/my-pkg/. ${TARGET_DIR}/
${SCRIPT_DIR}/../setup_git.sh ${TARGET_DIR}

# If a package manager was passed, set it in package.json
echo "$2"

if [ "$2" != "" ]; then
  cat "$TARGET_DIR/package.json" | jq ".packageManager=\"$2\"" | sponge "$TARGET_DIR/package.json"
  git commit -am "Update package manager" --quiet
fi
