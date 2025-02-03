#!/bin/bash

user_provided_flags="$@"
script_provided_flags="\
  --platform \
  -p=turborepo-napi \
  --cargo-cwd=../../ \
  --cargo-name=turborepo_napi \
  --zig \
  native \
  --js=false \
"

for flag in $user_provided_flags; do
  if [[ $flag == --target=* ]]; then
    target=${flag#*=}
    rustup target add "$target"
  fi
done

node_modules/.bin/napi build $script_provided_flags $user_provided_flags

# Unfortunately, when napi generates a .d.ts file, it doesn't match our formatting rules (it doesn't have semicolons).
# Since there's now way to configure this from napi itself, so we need to run prettier on it after generating it.
node_modules/.bin/prettier --write js/index.d.ts

mkdir -p js/dist
cp js/index.{js,d.ts} js/dist/
