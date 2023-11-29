#!/bin/bash

SCRIPT_DIR=$(dirname ${BASH_SOURCE[0]})
TARGET_DIR=$1
FIXTURE_DIR=$2

cp -a ${SCRIPT_DIR}/../_fixtures/find_turbo/$FIXTURE_DIR/. ${TARGET_DIR}/

# We need to symlink: turbo -> .pnpm/turbo@1.0.0/node_modules/turbo
# where `turbo` is the symlink
# and `.pnpm/turbo@1.0.0/node_modules/turbo` is the path to symlink to
# Note: using a nested if so it's easy to find the Windows checks in scripts around the codebase.
if [[ "$OSTYPE" == "msys" ]]; then
   if [[ $FIXTURE_DIR == "linked" ]]; then
    # Delete the existing turbo directory or file, whatever exists there
    rm -rf node_modules/turbo

    # Let's enter the node_modules directory
    # echo "entering node_modules directory"
    pushd node_modules > /dev/null || exit 1

    # Use pnpx to run symlnk-dir because installing globally doesn't work with pnpm.
    pnpx symlink-dir .pnpm/turbo@1.0.0/node_modules/turbo turbo > /dev/null 2>&1

    # Get outta there
    popd > /dev/null || exit 1

    # Debug what we have
    echo "ls -al"
    ls -al

    echo "ls -al node_modules/"
    ls -al node_modules/

    echo "ls -al node_modules/turbo/"
    ls -al node_modules/turbo/


    echo "ls -al node_modules/turbo/../"
    ls -al node_modules/turbo/../

    echo "ls -al node_modules/turbo/../turbo-windows-64"
    ls -al node_modules/turbo/../turbo-windows-64

    echo "ls -al node_modules/turbo/../turbo-windows-64/bin"
    ls -al node_modules/turbo/../turbo-windows-64/bin

    echo "ls -al node_modules/.pnpm/turbo@1.0.0/node_modules/turbo-windows-64/bin"
    ls -al node_modules/.pnpm/turbo@1.0.0/node_modules/turbo-windows-64/bin

    echo "ls -al node_modules/.pnpm/turbo-windows-64@1.0.0/node_modules/turbo-windows-64/bin/"
    ls -al node_modules/.pnpm/turbo-windows-64@1.0.0/node_modules/turbo-windows-64/bin/
  fi
fi
