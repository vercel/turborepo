#!/bin/bash

export TURBO_DOWNLOAD_LOCAL_ENABLED=0
SCRIPT_DIR=$(dirname ${BASH_SOURCE[0]})
TARGET_DIR=$1
FIXTURE_DIR=$2

cp -a ${SCRIPT_DIR}/../../fixtures/find_turbo/$FIXTURE_DIR/. ${TARGET_DIR}/

# On Windows, git may check out symlinks (mode 120000) as plain text files
# containing the target path instead of actual symlinks/junctions. The linked
# fixture relies on two layers of symlinks that must be real junctions for
# turbo's local binary discovery to work:
#   Level 1: node_modules/turbo -> .pnpm/turbo@1.0.0/node_modules/turbo
#   Level 2: .pnpm/turbo@1.0.0/node_modules/turbo-<platform> -> ../../turbo-<platform>@1.0.0/...
#
# We recreate all of them as NTFS junctions (mklink /J), which work without
# Developer Mode or elevated privileges. Junction targets are resolved relative
# to CWD (not the link's parent), so we use paths from the working directory.
if [[ "$OSTYPE" == "msys" ]]; then
  if [[ $FIXTURE_DIR == "linked" ]]; then
    PNPM_STORE="node_modules/.pnpm"
    PNPM_TURBO_NM="${PNPM_STORE}/turbo@1.0.0/node_modules"

    # Level 1: node_modules/turbo -> .pnpm/turbo@1.0.0/node_modules/turbo
    rm -rf node_modules/turbo
    cmd //c mklink //J \
      "node_modules\\turbo" \
      "${PNPM_TURBO_NM//\//\\}\\turbo" \
      > /dev/null || exit 1

    # Level 2: platform package symlinks inside the pnpm virtual store.
    # Junction targets must be relative to CWD, not the link's parent.
    for platform in darwin-64 darwin-arm64 linux-64 linux-arm64 windows-64 windows-arm64; do
      rm -rf "${PNPM_TURBO_NM}/turbo-${platform}"
      cmd //c mklink //J \
        "${PNPM_TURBO_NM//\//\\}\\turbo-${platform}" \
        "${PNPM_STORE//\//\\}\\turbo-${platform}@1.0.0\\node_modules\\turbo-${platform}" \
        > /dev/null || exit 1
    done
  fi
fi
