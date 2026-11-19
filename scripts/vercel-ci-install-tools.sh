#!/usr/bin/env bash
# Vercel CI install command. This repository has no turbo dependency, so CI
# installs the latest turbo canary, then `turbo setup` installs the Rust
# toolchain and the dependencies. `futureFlags.experimentalCargoWorkspaces`
# makes every turbo command run `cargo metadata`, so turbo needs that toolchain.
set -euo pipefail

runtime="$(dirname "$(pwd)")/.vercel-ci-runtime"
npm install --prefix "$runtime" --no-audit --no-fund turbo@canary

"$runtime/node_modules/.bin/turbo" setup --frozen
