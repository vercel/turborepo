#!/bin/sh
# Release smoke for a packaged Linux or macOS standalone turbo. It installs the
# archive with the POSIX installer from staged release assets while Node.js and
# JavaScript package managers are absent from PATH, then checks that setup
# selects this host's artifacts (GNU ones on a glibc host) and runs the managed
# setup journey (standalone-setup-journey.sh) against a loopback fixture
# mirror. On macOS, with TURBO_SMOKE_SIGNING_IDENTIFIER set, the installed copy
# must still carry that valid signature. On Linux, the same binary must reject
# Alpine (musl) with guidance. turbo itself runs without reachable network
# proxies.
#
# Usage: scripts/standalone-posix-smoke.sh <version> [standalone-artifacts]
# The fixture mirror runs on the runner's Node.js, outside turbo's PATH. The
# Alpine step downloads a pinned minirootfs and needs sudo, unshare, chroot.
set -eu
# Setup rejects group- or world-writable repositories; match a CI checkout.
umask 022

fail() {
  printf 'standalone smoke: %s\n' "$1" >&2
  exit 1
}

# Miette may wrap diagnostics across lines with `|` gutters.
contains() {
  case "$(printf '%s' "$1" | tr -d ' |\n')" in
    *"$(printf '%s' "$2" | tr -d ' |\n')"*) return 0 ;;
  esac
  return 1
}

version=${1:-}
[ -n "$version" ] || fail 'usage: standalone-posix-smoke.sh <version> [standalone-artifacts]'
assets=$(cd "${2:-standalone-artifacts}" && pwd)
installer=$(cd "$(dirname "$0")/.." && pwd)/apps/docs/public/install
arch=$(uname -m)
case "$(uname -s):$arch" in
  Linux:x86_64)
    target=x86_64-unknown-linux-musl selector=linux-x64-gnu node=linux-x64
    alpine=27694aaa55fd7a9e3ef596e0ad4eb66802308bb20172b17030cd5f4d8ae9bac2
    ;;
  Linux:aarch64)
    target=aarch64-unknown-linux-musl selector=linux-arm64-gnu node=linux-arm64
    alpine=821565fa8f3953eefd12497b166b4b50add2f7c57fb312e75862f5867e06fefe
    ;;
  Darwin:arm64) target=aarch64-apple-darwin selector=macos-arm64 node=darwin-arm64 ;;
  Darwin:x86_64) target=x86_64-apple-darwin selector=macos-x64 node=darwin-x64 ;;
  *) fail "unsupported smoke host: $(uname -s)/$arch" ;;
esac
archive=turbo-$version-$target.tar.gz
[ -f "$assets/$archive" ] && [ -f "$assets/SHA256SUMS" ] ||
  fail "missing $archive or SHA256SUMS in $assets"

work=$(mktemp -d "${TMPDIR:-/tmp}/turbo-standalone-smoke.XXXXXX")
cleanup() {
  if [ -d "$work/alpine" ]; then sudo rm -rf "$work/alpine"; fi
  rm -rf "$work"
}
trap cleanup 0

# Only the utilities the installer needs, plus a curl that serves the staged
# assets for the installer's release URLs and rejects every other request.
tools=$work/tools
mkdir "$tools" "$work/home" "$work/tmp"
for utility in sh tar gzip awk grep sed head mktemp mkdir cp chmod mv ln rm tr uname; do
  path=$(command -v "$utility") || fail "runner is missing $utility"
  ln -s "$path" "$tools/$utility"
done
# The installer takes either SHA-256 utility; older macOS has only shasum.
path=$(command -v sha256sum || command -v shasum) || fail 'runner is missing sha256sum or shasum'
ln -s "$path" "$tools/${path##*/}"
cat >"$tools/curl" <<'EOF'
#!/bin/sh
set -eu
output=
url=
while [ "$#" -gt 0 ]; do
  case "$1" in
    --output) output=$2; shift 2 ;;
    --header|--proto|--proto-redir) shift 2 ;;
    -*) shift ;;
    *) url=$1; shift ;;
  esac
done
name=${url##*/}
if [ -z "$output" ] || [ "$url" != "$SMOKE_RELEASE_URL/$name" ] || [ ! -f "$SMOKE_ASSETS/$name" ]; then
  printf 'unexpected download: %s\n' "$url" >&2
  exit 22
fi
cp "$SMOKE_ASSETS/$name" "$output"
EOF
chmod 0755 "$tools/curl"

closed=http://127.0.0.1:9
hermetic() {
  env -i PATH="$tools" HOME="$work/home" TMPDIR="$work/tmp" TURBO_TELEMETRY_DISABLED=1 DO_NOT_TRACK=1 \
    HTTP_PROXY="$closed" HTTPS_PROXY="$closed" ALL_PROXY="$closed" NO_PROXY= "$@"
}
# shellcheck disable=SC2016 # The hermetic shell expands $tool.
found=$(hermetic /bin/sh -c 'for tool in node npm npx pnpm yarn corepack bun turbo; do
  if command -v "$tool" >/dev/null 2>&1; then printf "%s " "$tool"; fi
done')
[ -z "$found" ] || fail "found $found on the smoke PATH"

install_from() {
  hermetic SMOKE_ASSETS="$1" \
    SMOKE_RELEASE_URL="https://github.com/vercel/turborepo/releases/download/v$version" \
    TURBO_VERSION="$version" TURBO_INSTALL_DIR="$work/install" TURBO_NO_MODIFY_PATH=1 \
    /bin/sh "$installer"
}

# Integrity: the packaged archive with appended bytes must not install.
mkdir "$work/tampered"
cp "$assets/SHA256SUMS" "$work/tampered/"
{ cat "$assets/$archive"; printf 'tampered'; } >"$work/tampered/$archive"
if output=$(install_from "$work/tampered" 2>&1); then
  fail "installed a tampered $archive"
fi
contains "$output" "SHA-256 verification failed for $archive" ||
  fail "unexpected result for a tampered archive: $output"
[ ! -e "$work/install" ] || fail 'a tampered archive changed the install directory'

turbo=$work/install/turbo
output=$(install_from "$assets" 2>&1) || fail "install failed: $output"
contains "$output" "Installed turbo $version at $turbo" || fail "unexpected install output: $output"
reported=$(hermetic "$turbo" --version) || fail 'turbo --version failed'
[ "$reported" = "$version" ] || fail "turbo --version reported $reported, expected $version"

# Packaging and installation keep the signed, notarized bytes intact.
if [ -n "${TURBO_SMOKE_SIGNING_IDENTIFIER:-}" ]; then
  codesign --verify --strict --verbose=2 "$turbo" || fail 'the installed turbo fails codesign verification'
  details=$(codesign -dv "$turbo" 2>&1)
  contains "$details" "Identifier=$TURBO_SMOKE_SIGNING_IDENTIFIER" ||
    fail "the installed turbo is not signed as $TURBO_SMOKE_SIGNING_IDENTIFIER: $details"
fi

# Read-only setup checks need no network. Reaching turbo.lock proves this
# binary qualified the host (a glibc host for the musl-static Linux build); the
# missing installation proves it selected this host's Node artifact.
repo=$work/repo
mkdir "$repo"
printf '%s\n' '{"futureFlags":{"experimentalSetup":true}}' >"$repo/turbo.json"
printf '24.0.0\n' >"$repo/.nvmrc"
printf '/.turbo/\n' >"$repo/.gitignore"
check() {
  if output=$(hermetic "$turbo" setup --check --tools-only --cwd="$repo" 2>&1); then
    fail "setup --check unexpectedly succeeded: $output"
  fi
  contains "$output" "$1" || fail "expected \"$1\" from setup --check: $output"
}
check 'managed activation requires turbo.lock'
zeros=0000000000000000000000000000000000000000000000000000000000000000
cat >"$repo/turbo.lock" <<EOF
{"schemaVersion":0,"tools":{"node":{"adapter":"node","version":"24.0.0",
"declarations":[{"file":".nvmrc","request":"24.0.0"}],
"installation":{"kind":"managed","artifacts":{"$selector":{"distribution":{
"url":"https://nodejs.org/dist/v24.0.0/node-v24.0.0-$node.tar.gz","sha256":"$zeros",
"format":"tar-gz","rootPrefix":"node-v24.0.0-$node","executables":{"node":"bin/node"}}}}}}}}
EOF
check 'managed installation is missing; run turbo setup'
[ ! -e "$repo/.turbo" ] || fail 'setup --check wrote .turbo'

# Managed setup and plain turbo run with this host's artifacts.
mirror_node=$(command -v node) || fail 'the fixture mirror needs Node.js on the runner'
sh "$(dirname "$0")/standalone-setup-journey.sh" "$turbo" "$mirror_node"
if [ "$(uname -s)" = Darwin ]; then
  printf 'Standalone smoke passed for turbo %s (%s).\n' "$version" "$target"
  exit 0
fi

# Alpine: the same binary must refuse managed tools on musl, without network.
release=3.22.6
name=alpine-minirootfs-$release-$arch.tar.gz
curl -fsSL --proto '=https' -o "$work/$name" \
  "https://dl-cdn.alpinelinux.org/alpine/v${release%.*}/releases/$arch/$name"
printf '%s  %s\n' "$alpine" "$work/$name" | sha256sum -c - >/dev/null ||
  fail "$name does not match its pinned SHA-256"
mkdir "$work/alpine"
tar -xzf "$work/$name" -C "$work/alpine"
cp "$turbo" "$work/alpine/turbo"
cp -R "$repo" "$work/alpine/repo"
if output=$(sudo unshare --net chroot "$work/alpine" /usr/bin/env -i PATH=/usr/bin:/bin \
  HOME=/tmp TURBO_TELEMETRY_DISABLED=1 /turbo setup --check --tools-only --cwd=/repo 2>&1); then
  fail "setup --check succeeded on Alpine: $output"
fi
if ! contains "$output" 'managed tools are not supported on musl Linux hosts' ||
  ! contains "$output" 'verify-system'; then
  fail "unexpected Alpine result: $output"
fi

printf 'Standalone smoke passed for turbo %s (%s).\n' "$version" "$target"
