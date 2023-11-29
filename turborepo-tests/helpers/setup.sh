#!/usr/bin/env bash

THIS_DIR=$(dirname "${BASH_SOURCE[0]}")

# MONOREPO_ROOT_DIR is used in some tests to use other helper scripts
MONOREPO_ROOT_DIR="${THIS_DIR}/../.."

if [[ "${OSTYPE}" == "msys" ]]; then
  EXT=".exe"
else
  EXT=""
fi

# All integration tests use this variable to call the turbo binary
TURBO=${MONOREPO_ROOT_DIR}/target/debug/turbo${EXT}
