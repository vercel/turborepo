#!/usr/bin/env bash

THIS_DIR=$(dirname "${BASH_SOURCE[0]}")

if [[ "$OSTYPE" == "msys" ]]; then
    ROOT_DIR="${THIS_DIR}\\..\\.."
    TURBO=${ROOT_DIR}\\target\\debug\\turbo.exe
else
    ROOT_DIR="${THIS_DIR}/../.."
    TURBO=${ROOT_DIR}/target/debug/turbo
fi

VERSION=${ROOT_DIR}/version.txt
TMPDIR=$(mktemp -d)
