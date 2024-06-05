#!/usr/bin/env bash

if [ -z "$1" ]; then
    echo "Please provide a tag..."
    echo "Usage: ./release.sh v0.1.0"
    exit
fi

branch_name=$(echo "${1}" | sed 's/\.//g')
git checkout -b "release_${branch_name}"

# Update the version
msg="# managed by release.sh"
sed -i '' "s/^version = .* $msg$/version = \"${1#v}\" $msg/" Cargo.toml

# Update the changelog
git-cliff --tag "$1" --unreleased --prepend CHANGELOG.md

# Commit changes
git add CHANGELOG.md Cargo.toml
git commit -m "chore(release): $1"

echo "Done."
echo "Now push the commit to GitHub."
echo "And after review, create a tag and push it to GitHub."
