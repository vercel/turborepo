#!/bin/bash
set -eo pipefail

user_provided_flags="$@"
script_provided_flags="\
  --platform \
  -p=turborepo-napi \
  --manifest-path=rust/Cargo.toml \
  --output-dir=native \
  --no-js \
"

for flag in $user_provided_flags; do
  if [[ $flag == --target=* ]]; then
    target=${flag#*=}
    rustup toolchain install nightly-2026-07-03 --target "$target"

    # Cross-compile Linux GNU targets with cargo-zigbuild.
    if [[ $target == x86_64-unknown-linux-gnu || $target == aarch64-unknown-linux-gnu ]]; then
      script_provided_flags+=" --cross-compile"
    fi
  fi
done

node_modules/.bin/napi build $script_provided_flags $user_provided_flags

# Unfortunately, when napi generates a .d.ts file, it doesn't match our formatting rules (it doesn't have semicolons).
# Since there's no way to configure this from napi itself, we need to run prettier on it after generating it.
node_modules/.bin/prettier --write js/index.d.ts

mkdir -p js/dist
cp js/index.{js,d.ts} js/dist/
