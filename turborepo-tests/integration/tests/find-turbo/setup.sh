#!/bin/bash

SCRIPT_DIR=$(dirname ${BASH_SOURCE[0]})
TARGET_DIR=$1
FIXTURE_DIR=$2

cp -a ${SCRIPT_DIR}/../_fixtures/find_turbo/$FIXTURE_DIR/. ${TARGET_DIR}/


# TODO: copy over the stub instead of having a duplicate in each fixture

# # These find_turbo fixtures have a pre-made node_modules directory that stubs out where the local turbo binary
# # would be located for specific package manager setups. For linux and darwin, we just put those binaries
# # into the fixture itself. For Windows platform, the binary itself needs to be a _real_ Windows binary. Instead
# # of maintaining many copies of these binaries, we keep one and move it over to the specific folder in node_modules
# # required by that fixture. This makes the fixture a bit dynamic in nature, but it's easier to maintain.
# ##
# # Note that we only _really_ need to do this when these tests are running on Windows, because that's the
# # only time they get used, but we will do it always, because the folders exist in the fixture and they shuoldn't be empty.
# WINDOWS_BIN="${SCRIPT_DIR}/../_fixtures/find_turbo/-windows-binary/turbostub.exe"

# if [[ "$FIXTURE_DIR" == "hoisted" ]]; then
#   cp "$WINDOWS_BIN"  "${TARGET_DIR}/node_modules/turbo-windows-64/bin/turbo.exe"
#   cp "$WINDOWS_BIN"  "${TARGET_DIR}/node_modules/turbo-windows-arm64/bin/turbo.exe"
# elif [[ "$FIXTURE_DIR" == "linked" ]]; then
#   cp "$WINDOWS_BIN"  "${TARGET_DIR}/node_modules/.pnpm/turbo-windows-64@1.0.0/bin/turbo.exe"
#   cp "$WINDOWS_BIN"  "${TARGET_DIR}/node_modules/.pnpm/turbo-windows-arm64@1.0.0/bin/turbo.exe"
# elif [[ "$FIXTURE_DIR" == "nested" ]]; then
#   cp "$WINDOWS_BIN"  "${TARGET_DIR}/node_modules/turbo/node_modules/turbo-windows-64/bin/turbo.exe"
#   cp "$WINDOWS_BIN"  "${TARGET_DIR}/node_modules/turbo/node_modules/turbo-windows-arm64/bin/turbo.exe"
# elif [[ "$FIXTURE_DIR" == "self" ]]; then
#   cp "$WINDOWS_BIN"  "${TARGET_DIR}/node_modules/turbo-windows-64/bin/turbo.exe"
#   cp "$WINDOWS_BIN"  "${TARGET_DIR}/node_modules/turbo-windows-arm64/bin/turbo.exe"
# elif [[ "$FIXTURE_DIR" == "unplugged" ]]; then
#   cp "$WINDOWS_BIN"  "${TARGET_DIR}/.yarn/unplugged/turbo-windows-64-npm-1.0.0-520925a700/node_modules/turbo-windows-64/bin/turbo.exe"
#   cp "$WINDOWS_BIN"  "${TARGET_DIR}/.yarn/unplugged/turbo-windows-arm64-npm-1.0.0-520925a700/node_modules/turbo-windows-arm64/bin/turbo.exe"
# elif [[ "$FIXTURE_DIR" == "unplugged_env_moved" ]]; then
#   cp "$WINDOWS_BIN"  "${TARGET_DIR}/.moved/unplugged/turbo-windows-64-npm-1.0.0-520925a700/node_modules/turbo-windows-64/bin/turbo.exe"
#   cp "$WINDOWS_BIN"  "${TARGET_DIR}/.moved/unplugged/turbo-windows-arm64-npm-1.0.0-520925a700/node_modules/turbo-windows-arm64/bin/turbo.exe"
# elif [[ "$FIXTURE_DIR" == "unplugged_moved" ]]; then
#   cp "$WINDOWS_BIN"  "${TARGET_DIR}/.moved/unplugged/turbo-windows-64-npm-1.0.0-520925a700/node_modules/turbo-windows-64/bin/turbo.exe"
#   cp "$WINDOWS_BIN"  "${TARGET_DIR}/.moved/unplugged/turbo-windows-arm64-npm-1.0.0-520925a700/node_modules/turbo-windows-arm64/bin/turbo.exe"
# fi
