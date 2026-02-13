#!/bin/bash
# One-time bootstrap: publish empty placeholder packages so that
# pnpm can resolve the optionalDependencies in @turbo/gen's package.json.
#
# Run once before merging the PR:
#   bash packages/turbo-gen/scripts/bootstrap-platform-packages.sh
#
# After the first real release, these placeholders will be replaced
# by actual platform binaries and this script is no longer needed.

set -euo pipefail

VERSION="0.0.0"
PLATFORMS=("darwin-64" "darwin-arm64" "linux-64" "linux-arm64" "windows-64")

for platform in "${PLATFORMS[@]}"; do
  name="@turbo/gen-${platform}"
  dir=$(mktemp -d)

  os="${platform%-*}"
  arch="${platform##*-}"

  if [ "$os" = "darwin" ]; then node_os="darwin"; fi
  if [ "$os" = "linux" ]; then node_os="linux"; fi
  if [ "$os" = "windows" ]; then node_os="win32"; fi

  if [ "$arch" = "64" ]; then node_cpu="x64"; fi
  if [ "$arch" = "arm64" ]; then node_cpu="arm64"; fi

  cat > "${dir}/package.json" <<EOF
{
  "name": "${name}",
  "version": "${VERSION}",
  "description": "Platform binary placeholder for @turbo/gen (${platform})",
  "repository": "https://github.com/vercel/turborepo",
  "license": "MIT",
  "os": ["${node_os}"],
  "cpu": ["${node_cpu}"]
}
EOF

  echo "Publishing ${name}@${VERSION}..."
  npm publish "${dir}" --access public
  rm -rf "${dir}"
done

echo ""
echo "Done. Now run 'pnpm install' to update the lockfile."
