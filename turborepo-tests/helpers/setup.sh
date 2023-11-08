#!/usr/bin/env bash

THIS_DIR=$(dirname "${BASH_SOURCE[0]}")
ROOT_DIR="${THIS_DIR}/../.."

if [[ "$OSTYPE" == "msys" ]]; then
    TURBO=${ROOT_DIR}/target/debug/turbo.exe
else
    TURBO=${ROOT_DIR}/target/debug/turbo
fi

VERSION=${ROOT_DIR}/version.txt
TMPDIR=$(mktemp -d)
