#!/bin/bash

SCRIPT_DIR=$(dirname ${BASH_SOURCE[0]})

TARGET_DIR=$1
FIXTURE_NAME=$2

# readlink should resolve the relative paths to the fixture so we have a canonicalized absolute path
FIXTURE_DIR="${SCRIPT_DIR}/../_fixtures/find_turbo/$FIXTURE_NAME"
# FIXTURE_DIR2="${TESTDIR}/../_fixtures/find_turbo/$FIXTURE_NAME" # TESTDIR should be `turborepo-tests/integration/tests/find-turbo` here

# echo "PWD: $PWD"
# echo "HOME: ${HOME}"
# echo "TMPDIR: $TMPDIR"
# echo "BASH_SOURCE[0]: ${BASH_SOURCE[0]}"
# echo "SCRIPT_DIR: ${SCRIPT_DIR}"
# echo "TESTDIR: ${TESTDIR}"

# echo "FIXTURE_DIR: $FIXTURE_DIR"
# echo "FIXTURE_DIR2: $FIXTURE_DIR2"
# echo "TARGET_DIR: $TARGET_DIR"
# echo "READLINK_FIXTURE_DIR: $(readlink -f "$FIXTURE_DIR")"
# echo "READLINK_FIXTURE_DIR2: $(readlink -f "$FIXTURE_DIR2")"
# echo "READLINK_TARGET_DIR: $(readlink -f "$TARGET_DIR")"
# echo "-----------"

DESTINATION="${TARGET_DIR}"
# echo "cp cmd: cp -a ${FIXTURE_DIR}/. ${DESTINATION}/"
cp -a "${FIXTURE_DIR}/." "${DESTINATION}/"

# We need to symlink: turbo -> .pnpm/turbo@1.0.0/node_modules/turbo
# where `turbo` is the symlink
# and `.pnpm/turbo@1.0.0/node_modules/turbo` is the path to symlink to
if [[ "$OSTYPE" == "msys" && $FIXTURE_NAME == "linked" ]]; then
  # Delete the existing turbo directory or file, whatever exists there
  rm -rf node_modules/turbo

  # Let's enter the node_modules directory
  # echo "entering node_modules directory"
  pushd node_modules > /dev/null || exit 1

  ######## Create the symlink
  # cmd //c mklink turbo "${PWD}\\.pnpm\\turbo@1.0.0\\node_modules\\turbo"
  # echo "running chmod on new symlink turbo"
  # chmod +rwx turbo
  # echo "running icacls on new symlink turbo"
  # cmd //c icacls "turbo /grant Everyone:(F)"

  # Use pnpx to run symlnk-dir because installing globally doesn't work with pnpm
  # TODO, should we install this as a dependency in this workspace so we can use it or
  # something else to avoid hitting the network in the middle of the test setup?
  # echo "pnpx symlink-dir turbo .pnpm/turbo@1.0.0/node_modules/turbo"
  pnpx symlink-dir .pnpm/turbo@1.0.0/node_modules/turbo turbo > /dev/null 2>&1

  # Get outta there
  # echo "leaving node_modules directory"
  popd > /dev/null || exit 1

  # # Make sure we got outta there.
  # echo "PWD now is: $PWD"

  # # Debug what we have
  # echo "ls -al"
  # ls -al

  # echo "ls -al node_modules/"
  # ls -al node_modules/

  # echo "ls -al node_modules/turbo/"
  # ls -al node_modules/turbo/

  # echo "ls -al node_modules/turbo/../"
  # ls -al node_modules/turbo/../

  # echo "ls -al node_modules/turbo/../turbo-windows-64"
  # ls -al node_modules/turbo/../turbo-windows-64

  # echo "ls -al node_modules/turbo/../turbo-windows-64/bin"
  # ls -al node_modules/turbo/../turbo-windows-64/bin

  # echo "REAL PATH: ls -al node_modules/.pnpm/turbo@1.0.0/node_modules/turbo-windows-64/bin"
  # ls -al node_modules/.pnpm/turbo@1.0.0/node_modules/turbo-windows-64/bin

  # echo "ls -al node_modules/.pnpm/turbo-windows-64@1.0.0/node_modules/turbo-windows-64/bin/"
  # ls -al node_modules/.pnpm/turbo-windows-64@1.0.0/node_modules/turbo-windows-64/bin/
fi
