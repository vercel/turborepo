#!/usr/bin/env bash

dir=$1
pkgManager=$2

# Update package manager
if [ "$pkgManager" != "" ]; then
  # Use jq to write a new file with a .packageManager field set and then
  # overwrite original package.json.
  jq --arg pm "$pkgManager" '.packageManager = $pm' "$dir/package.json" > "$dir/package.json.new"
  mv "$dir/package.json.new" "$dir/package.json"

  # We just created a new file. On Windows, we need to convert it to Unix line endings
  # so the hashes will be stable with what's expected in our test cases.
  if [[ "$OSTYPE" == "msys" ]]; then
    dos2unix --quiet "$dir/package.json"
  fi

  git commit -am "Updated package manager to $pkgManager" --quiet
fi
