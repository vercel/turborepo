#!/bin/bash

# This script is called from within a prysk test, so pwd is already in the prysk tmp directory.

set -eo pipefail

exampleName=$1
pkgManager=$2

# Copy the example dir over to the test dir that prysk puts you in
SCRIPT_DIR=$(dirname "${BASH_SOURCE[0]}")
MONOREPO_ROOT_DIR="$SCRIPT_DIR/../.."
EXAMPLE_DIR="$MONOREPO_ROOT_DIR/examples/$exampleName"

TARGET_DIR="$(pwd)"

cp -a "$EXAMPLE_DIR/." "${TARGET_DIR}/"

# cleanup lockfiles so we can install from scratch
[ ! -f yarn.lock ] || mv yarn.lock yarn.lock.bak
[ ! -f pnpm-lock.yaml ] || mv pnpm-lock.yaml pnpm-lock.yaml.bak
[ ! -f package-lock.json ] || mv package-lock.json package-lock.json.bak


TURBO_VERSION_FILE="${MONOREPO_ROOT_DIR}/version.txt"
# Change package.json in the example directory to point to @canary if our branch is currently at that version
TURBO_TAG=$(cat "$TURBO_VERSION_FILE" | sed -n '2 p')
if [ "$TURBO_TAG" == "canary" ]; then
  jq --arg version "canary" '.devDependencies.turbo = $version' package.json > package.json.new
  mv package.json.new package.json
fi

# Update package manager
if [ "$3" != "" ]; then
  # Use jq to write a new file with a .packageManager field set and then
  # overwrite original package.json.
  jq --arg pm "$3" '.packageManager = $pm' "$TARGET_DIR/package.json" > "$TARGET_DIR/package.json.new"
  mv "$TARGET_DIR/package.json.new" "$TARGET_DIR/package.json"

  # We just created a new file. On Windows, we need to convert it to Unix line endings
  # so the hashes will be stable with what's expected in our test cases.
  if [[ "$OSTYPE" == "msys" ]]; then
    dos2unix --quiet "$TARGET_DIR/package.json"
  fi
fi

# Enable corepack so that when we set the packageManager in package.json it actually makes a diference.
if [ "$PRYSK_TEMP" == "" ]; then
  COREPACK_INSTALL_DIR_CMD=
else
  COREPACK_INSTALL_DIR="${PRYSK_TEMP}/corepack"
  mkdir -p "${COREPACK_INSTALL_DIR}"
  export PATH=${COREPACK_INSTALL_DIR}:$PATH
  COREPACK_INSTALL_DIR_CMD="--install-directory=${COREPACK_INSTALL_DIR}"
fi
corepack enable "${COREPACK_INSTALL_DIR_CMD}"

# Delete .git directory if it's there, we'll set up a new git repo
[ ! -d .git ] || rm -rf .git

if [ "${OSTYPE}" == "msys" ]; then
  EXT=".exe"
else
  EXT=""
fi
export TURBO_BINARY_PATH=${MONOREPO_ROOT_DIR}/target/debug/turbo${EXT}

"$MONOREPO_ROOT_DIR/turborepo-tests/helpers/setup_git.sh" "${TARGET_DIR}"

# Install dependencies after git is setup
"${SCRIPT_DIR}/install_deps.sh" "$pkgManager"
