#!/usr/bin/env bash
# Runs the runner tests against every supported major of jest and vitest (CI
# runs one of each, the ones in package.json). Needs a built package and npm.
#   scripts/test-runners.sh [jest@29 vitest@0.34 ...]
set -euo pipefail
package=$(cd "$(dirname "$0")/.." && pwd)
specs=${*:-jest@27 jest@28 jest@29 jest@30 vitest@0.34 vitest@1 vitest@2 vitest@3 vitest@4 vitest@5}
for spec in $specs; do
  dir=$(mktemp -d)
  (cd "$dir" && npm init --yes && npm install --no-audit --no-fund "$spec") > /dev/null
  mkdir -p "$dir/node_modules/@turbo" && ln -s "$package" "$dir/node_modules/@turbo/trace"
  echo "== $spec"
  (cd "$package" && TURBO_TRACE_TEST_RUNNERS=$dir pnpm exec jest runners) || failed="${failed:-} $spec"
  rm -rf "$dir"
done
[ -z "${failed:-}" ] || { echo "failed:$failed"; exit 1; }
