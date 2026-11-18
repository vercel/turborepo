#!/usr/bin/env bash
# TypeScript bindings for the relay protocol, formatted like the rest of the repository.
set -eu

pkg=$(dirname "$0")/..
cd "$pkg"
pnpm exec buf generate
pnpm -w format:files "$PWD/src/gen"
