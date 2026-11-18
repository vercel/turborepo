#!/usr/bin/env bash
# Vercel CI install command. This repository has no turbo dependency, so CI
# installs the latest turbo canary, then `turbo setup` installs the Rust
# toolchain and the dependencies. `futureFlags.experimentalCargoWorkspaces`
# makes every turbo command run `cargo metadata`, so turbo needs that toolchain.
#
# turbo goes into Vercel CI's runtime prefix, next to the repository. Its
# `node_modules/.bin` is on the PATH of the planning step and of every task.
# Vercel CI sets TURBO_TOOL_STORE.
set -euo pipefail

runtime="$(dirname "$(pwd)")/.vercel-ci-runtime"
npm install --prefix "$runtime" --no-audit --no-fund turbo@canary

# No turbo.lock is committed, so setup has to resolve one.
"$runtime/node_modules/.bin/turbo" setup --no-frozen
