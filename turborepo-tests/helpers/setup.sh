#!/usr/bin/env bash

REALPATH="$(realpath "$0")"
THIS_DIR=$(dirname "${REALPATH}")

# if [[ "$OSTYPE" == "msys" ]]; then
#     ROOT_DIR="${THIS_DIR}\\..\\.."
#     TURBO=${ROOT_DIR}\\target\\debug\\turbo.exe
# else
#     ROOT_DIR="${THIS_DIR}/../.."
#     TURBO=${ROOT_DIR}/target/debug/turbo
# fi

ROOT_DIR="${THIS_DIR}/../.."
TURBO=${ROOT_DIR}/target/debug/turbo

echo "------------------------"
echo "BASHSOURCE: ${BASH_SOURCE[0]}"
echo "REALPATH: ${REALPATH}"
echo "THIS_DIR: ${THIS_DIR}"
echo "ROOT_DIR: ${ROOT_DIR}"
echo "TURBO: ${TURBO}"
echo "------------------------"

VERSION=${ROOT_DIR}/version.txt
TMPDIR=$(mktemp -d)
