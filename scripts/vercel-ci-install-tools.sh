#!/usr/bin/env bash
# Vercel CI install command. This repository has no turbo dependency, so CI
# installs the latest turbo canary, then `turbo setup` installs the Rust
# toolchain and the dependencies. `futureFlags.experimentalCargoWorkspaces`
# makes every turbo command run `cargo metadata`, so turbo needs that toolchain.
set -euo pipefail

runtime="$(dirname "$(pwd)")/.vercel-ci-runtime"
npm install --prefix "$runtime" --no-audit --no-fund turbo@canary

# Frozen mode installs exactly what the committed turbo.lock records. It fails
# if turbo.lock no longer matches the tool declarations.
"$runtime/node_modules/.bin/turbo" setup --frozen
