#!/usr/bin/env bash

THIS_DIR=$(dirname "${BASH_SOURCE[0]}")
MONOREPO_ROOT_DIR="${THIS_DIR}/../.."

if [[ "${OSTYPE}" == "msys" ]]; then
  EXT=".exe"
else
  EXT=""
fi

# disable the first-run telemetry message
export TURBO_TELEMETRY_MESSAGE_DISABLED=1
export TURBO_GLOBAL_WARNING_DISABLED=1
export TURBO_DOWNLOAD_LOCAL_ENABLED=0
export TURBO_PRINT_VERSION_DISABLED=1
export COREPACK_ENABLE_DOWNLOAD_PROMPT=0
TURBO=${MONOREPO_ROOT_DIR}/target/debug/turbo${EXT}

# Unset GITHUB_ACTIONS to prevent GitHub Actions-specific behavior (e.g. log grouping)
# Tests that need GitHub Actions behavior should set GITHUB_ACTIONS=1 explicitly
unset GITHUB_ACTIONS
