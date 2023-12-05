#!/usr/bin/env bash

THIS_DIR=$(dirname "${BASH_SOURCE[0]}")
MONOREPO_ROOT_DIR="${THIS_DIR}/../.."

if [[ "${OSTYPE}" == "msys" ]]; then
  EXT=".exe"
else
  EXT=""
fi

TURBO=${MONOREPO_ROOT_DIR}/target/debug/turbo${EXT}
