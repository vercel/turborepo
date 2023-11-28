#!/bin/bash

SCRIPT_DIR=$(dirname ${BASH_SOURCE[0]})
TARGET_DIR=$1
FIXTURE_DIR=$2

cp -a ${SCRIPT_DIR}/../_fixtures/find_turbo/$FIXTURE_DIR/. ${TARGET_DIR}/

# We need to symlink: turbo -> .pnpm/turbo@1.0.0/node_modules/turbo
# where `turbo` is the symlink
# and `.pnpm/turbo@1.0.0/node_modules/turbo` is the path to symlink to
if [[ "$OSTYPE" == "msys" && $FIXTURE_NAME == "linked" ]]; then
  # Delete the existing turbo directory or file, whatever exists there
  rm -rf node_modules/turbo

  # Let's enter the node_modules directory
  # echo "entering node_modules directory"
  pushd node_modules > /dev/null || exit 1


  # Use pnpx to run symlnk-dir because installing globally doesn't work with pnpm
  # TODO, should we install this as a dependency in this workspace so we can use it or
  # something else to avoid hitting the network in the middle of the test setup?
  pnpx symlink-dir .pnpm/turbo@1.0.0/node_modules/turbo turbo > /dev/null 2>&1

  # Get outta there
  popd > /dev/null || exit 1
fi
