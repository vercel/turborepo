#!/usr/bin/env bash
set -e

export TURBO_LOG_VERBOSITY=trace
export EXPERIMENTAL_RUST_CODEPATH=true

echo "$PWD" # current dir shuold be benchmark/

cd large-monorepo
../../target/debug/turbo build --skip-infer --dry --profile=../profile.json
cd .. # back to benchmark root dir
jq -f src/fold.jq < profile.json > ttft.json
