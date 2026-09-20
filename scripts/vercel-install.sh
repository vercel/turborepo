#!/usr/bin/env bash

set -e

package_name="$1"
repo_root=$(cd "$(dirname "$0")/.." && pwd)

cd "$repo_root"
pnpm install --filter="${package_name}..." --frozen-lockfile

if ! pnpm --workspace-root add --save-dev turbo@"$(head -n 1 version.txt)" --config.minimum-release-age=0; then
  pnpm --workspace-root add --save-dev turbo@"$(sed -n '2p' version.txt)" --config.minimum-release-age=0
fi
