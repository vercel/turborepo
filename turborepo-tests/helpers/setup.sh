#!/usr/bin/env bash

THIS_DIR=$(dirname "${BASH_SOURCE[0]}")
ROOT_DIR="${THIS_DIR}/../.."

if [[ "${OSTYPE}" == "msys" ]]; then
  EXT=".exe"
else
  EXT=""
fi

TURBO=${ROOT_DIR}/target/debug/turbo${EXT}
TMPDIR=$(mktemp -d)
