#!/usr/bin/env bash

if [[ "$OSTYPE" != "msys" ]]; then
  echo "Skipping build for non-windows platform"
  exit
fi


echo "Building stub turbo.exe for windows platform"
g++ turbo.cpp -o turbo.exe


SCRIPT_DIR=$(dirname "${BASH_SOURCE[0]}")
UP_ONE="$SCRIPT_DIR/.."
ROOT_DIR="$SCRIPT_DIR/../.."
FIND_TURBO_FIXTURES_DIR="${ROOT_DIR}/turborepo-tests/integration/fixtures/find_turbo"

echo "PWD: $PWD"
echo "ROOT_DIR: $ROOT_DIR"
echo "UP_ONE: $UP_ONE"
echo "FIND_TURBO_FIXTURES_DIR: ${FIND_TURBO_FIXTURES_DIR}"

cp turbo.exe "${FIND_TURBO_FIXTURES_DIR}/hoisted/node_modules/turbo-windows-64/bin/"
cp turbo.exe "${FIND_TURBO_FIXTURES_DIR}/hoisted/node_modules/turbo-windows-arm64/bin/"

cp turbo.exe "${FIND_TURBO_FIXTURES_DIR}/linked/node_modules/.pnpm/turbo-windows-64@1.0.0/node_modules/turbo-windows-64/bin/"
cp turbo.exe "${FIND_TURBO_FIXTURES_DIR}/linked/node_modules/.pnpm/turbo-windows-arm64@1.0.0/node_modules/turbo-windows-arm64/bin/"

cp turbo.exe "${FIND_TURBO_FIXTURES_DIR}/nested/node_modules/turbo/node_modules/turbo-windows-64/bin/"
cp turbo.exe "${FIND_TURBO_FIXTURES_DIR}/nested/node_modules/turbo/node_modules/turbo-windows-arm64/bin/"

cp turbo.exe "${FIND_TURBO_FIXTURES_DIR}/self/node_modules/turbo-windows-64/bin/"
cp turbo.exe "${FIND_TURBO_FIXTURES_DIR}/self/node_modules/turbo-windows-arm64/bin/"

cp turbo.exe "${FIND_TURBO_FIXTURES_DIR}/unplugged/.yarn/unplugged/turbo-windows-64-npm-1.0.0-520925a700/node_modules/turbo-windows-64/bin/"
cp turbo.exe "${FIND_TURBO_FIXTURES_DIR}/unplugged/.yarn/unplugged/turbo-windows-arm64-npm-1.0.0-520925a700/node_modules/turbo-windows-arm64/bin/"

cp turbo.exe "${FIND_TURBO_FIXTURES_DIR}/unplugged_env_moved/.moved/unplugged/turbo-windows-64-npm-1.0.0-520925a700/node_modules/turbo-windows-64/bin/"
cp turbo.exe "${FIND_TURBO_FIXTURES_DIR}/unplugged_env_moved/.moved/unplugged/turbo-windows-arm64-npm-1.0.0-520925a700/node_modules/turbo-windows-arm64/bin/"

cp turbo.exe "${FIND_TURBO_FIXTURES_DIR}/unplugged_moved/.moved/unplugged/turbo-windows-64-npm-1.0.0-520925a700/node_modules/turbo-windows-64/bin/"
cp turbo.exe "${FIND_TURBO_FIXTURES_DIR}/unplugged_moved/.moved/unplugged/turbo-windows-arm64-npm-1.0.0-520925a700/node_modules/turbo-windows-arm64/bin/"
