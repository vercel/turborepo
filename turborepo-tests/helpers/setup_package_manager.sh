#!/usr/bin/env bash

dir=$1
pkgManager=$2

# Update package manager if one was provided
if [ "$pkgManager" != "" ]; then
  # Use jq to write a new file with a .packageManager field set and then
  # Overwrite original package.json. For some reason the command above won't send its output
  # directly to the original file.
  jq --arg pm "$pkgManager" '.packageManager = $pm' "$dir/package.json" > "$dir/package.json.new"
  mv "$dir/package.json.new" "$dir/package.json"

  # We just created a new file. On Windows, we need to convert it to Unix line endings
  # so the hashes will be stable with what's expected in our test cases.
  if [[ "$OSTYPE" == "msys" ]]; then
    dos2unix --quiet "$dir/package.json"
  fi

  if [[ $(git status --porcelain) ]]; then
    git commit -am "Updated package manager to $pkgManager" --quiet
  fi
fi

# get just the packageManager name, without the version
# We pass the name to corepack enable so that it will work for npm also.
# `corepack enable` with no specified packageManager does not work for npm.
pkgManagerName="${pkgManager%%@*}"

# Set the corepack install directory to a temp directory (either prysk temp or provided dir).
# This will help isolate from the rest of the system, especially when running tests on a dev machine.
COREPACK_INSTALL_DIR="${PRYSK_TEMP:-$dir}/corepack"
if [[ "$OSTYPE" == "msys" ]]; then
  # Ensure it's a POSIX path so that we can use it as a PATH entry (C:\... -> /c/...)
  COREPACK_INSTALL_DIR="$(cygpath -au "$COREPACK_INSTALL_DIR")"
  # Ensure corepack uses lowercase .cmd extensions, consistent with node's bundled npm
  export PATHEXT="$(echo "$PATHEXT" | tr '[:upper:]' '[:lower:]')"
fi
mkdir -p "${COREPACK_INSTALL_DIR}"
export PATH=${COREPACK_INSTALL_DIR}:$PATH


# Enable corepack so that the packageManager setting in package.json is respected.
corepack enable $pkgManagerName "--install-directory=${COREPACK_INSTALL_DIR}"



