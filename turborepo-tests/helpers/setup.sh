#!/usr/bin/env bash

THIS_DIR=$(dirname "$(realpath "$0")")

if [[ "$OSTYPE" == "msys" ]]; then
    ROOT_DIR="${THIS_DIR}\\..\\.."
    TURBO=${ROOT_DIR}\\target\\debug\\turbo.exe
else
    ROOT_DIR="${THIS_DIR}/../.."
    TURBO=${ROOT_DIR}/target/debug/turbo
fi

echo "------------------------"
echo "BASHSOURCE: ${BASH_SOURCE[0]}"
echo "THIS_DIR: ${THIS_DIR}"
echo "ROOT_DIR: ${ROOT_DIR}"
echo "TURBO: ${TURBO}"
echo "------------------------"

VERSION=${ROOT_DIR}/version.txt
TMPDIR=$(mktemp -d)
