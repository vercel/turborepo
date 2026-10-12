#!/bin/sh
# Managed setup journey for an installed standalone turbo, shared by the POSIX
# release smokes. turbo sees an empty PATH (no Node.js or JS package manager)
# and an unreachable proxy; only the loopback fixture mirror answers, through
# TURBO_SETUP_NODE_MIRROR and TURBO_SETUP_NPM_REGISTRY. The journey:
# `turbo setup --tools-only` creates turbo.lock, a frozen clone refuses tampered
# mirror bytes, plain `turbo run` uses managed Node and pnpm in strict and loose
# modes, repeat, CI, and offline setup and `--check` stay offline, and setup
# repairs a damaged installation that run refuses.
#
# Usage: scripts/standalone-setup-journey.sh <turbo> <node-for-the-mirror>
set -eu
umask 022

fail() {
  printf 'standalone setup journey: %s\n' "$1" >&2
  exit 1
}

# Miette may wrap diagnostics across lines with `|` gutters.
contains() {
  case "$(printf '%s' "$1" | tr -d ' |\n')" in
    *"$(printf '%s' "$2" | tr -d ' |\n')"*) return 0 ;;
  esac
  return 1
}

[ "$#" -eq 2 ] || fail 'usage: standalone-setup-journey.sh <turbo> <node-for-the-mirror>'
turbo=$1
scripts=$(cd "$(dirname "$0")" && pwd)
case "$(uname -m)" in
  x86_64) arch=x64 ;;
  aarch64 | arm64) arch=arm64 ;;
  *) fail "unsupported journey host: $(uname -m)" ;;
esac
work=$(mktemp -d "${TMPDIR:-/tmp}/turbo-setup-journey.XXXXXX")
work=$(cd "$work" && pwd -P)
mirror=$work/mirror
mkdir "$mirror" "$work/home" "$work/clone-home" "$work/tmp" "$work/empty-path"
"$2" "$scripts/setup-fixture-mirror.mjs" "$mirror" &
pid=$!
trap 'kill "$pid" 2>/dev/null || true; rm -rf "$work"' 0
tries=0
until [ -s "$mirror/ready" ]; do
  kill -0 "$pid" 2>/dev/null || fail 'the fixture mirror exited'
  tries=$((tries + 1))
  [ "$tries" -le 600 ] || fail 'the fixture mirror did not start'
  sleep 0.1
done
read -r port version <"$mirror/ready"

repo=$work/repo
web=$repo/apps/web
mkdir -p "$web"
printf '%s\n' '{"futureFlags":{"experimentalSetup":true},"agentGuidance":false,"tasks":{"build":{"cache":false}}}' >"$repo/turbo.json"
printf '%s\n' "$version" >"$repo/.nvmrc"
printf '/.turbo/\nnode_modules/\n' >"$repo/.gitignore"
printf '%s\n' '{"name":"journey","private":true,"packageManager":"pnpm@10.0.0"}' >"$repo/package.json"
printf "packages: ['apps/*']\n" >"$repo/pnpm-workspace.yaml"
printf "lockfileVersion: '9.0'\nimporters:\n  .: {}\n  apps/web: {}\n" >"$repo/pnpm-lock.yaml"
printf '%s\n' '{"name":"web","scripts":{"build":"node probe.cjs"}}' >"$web/package.json"
cat >"$web/probe.cjs" <<'EOF'
const first = process.env.PATH.split(require("node:path").delimiter)[0];
const node = require("node:fs").realpathSync(process.execPath);
const secret = process.env.SMOKE_SECRET ?? "filtered";
console.log(`managed-probe ${node}|${process.version}|${process.arch}|${first}|${secret}`);
EOF

base=https://127.0.0.1:$port
closed=http://127.0.0.1:9
home=$work/home ci=
# Sets $output and $code, and $requests to the mirror requests it made.
run() {
  dir=$1
  shift
  before=$(wc -l <"$mirror/requests.log")
  code=0
  output=$(cd "$dir" && env -i PATH="$work/empty-path" HOME="$home" TMPDIR="$work/tmp" \
    TURBO_TELEMETRY_DISABLED=1 DO_NOT_TRACK=1 TURBO_NO_UPDATE_NOTIFIER=1 \
    HTTP_PROXY="$closed" HTTPS_PROXY="$closed" ALL_PROXY="$closed" NO_PROXY=127.0.0.1 \
    SSL_CERT_FILE="$scripts/fixtures/setup-mirror/ca.pem" \
    TURBO_SETUP_NODE_MIRROR="$base/node" TURBO_SETUP_NPM_REGISTRY="$base/npm" \
    SMOKE_SECRET=present ${ci:+CI=1} "$turbo" "$@" 2>&1) || code=$?
  requests=$(($(wc -l <"$mirror/requests.log") - before))
}
succeeds() {
  run "$@"
  [ "$code" -eq 0 ] || fail "turbo $* exited $code: $output"
}
offline() {
  succeeds "$@"
  [ "$requests" -eq 0 ] || fail "turbo $* made $requests mirror requests: $output"
}

# First setup, from a subdirectory, through the mirror only.
succeeds "$web" setup --tools-only
contains "$output" 'Commit turbo.lock' || fail "first setup did not ask for a commit: $output"
[ "$requests" -gt 0 ] || fail 'first setup made no mirror requests'
! grep -q 127.0.0.1 "$repo/turbo.lock" || fail 'turbo.lock records the mirror location'
cp "$repo/turbo.lock" "$work/turbo.lock"
cp "$repo/.turbo/tools/manifest.json" "$work/manifest.json"
unchanged() {
  cmp -s "$repo/turbo.lock" "$work/turbo.lock" &&
    cmp -s "$repo/.turbo/tools/manifest.json" "$work/manifest.json" ||
    fail "turbo $1 changed turbo.lock or the installation"
}

# A clone with the committed lock and an empty download cache refuses tampered
# mirror bytes in frozen CI setup and installs nothing.
cp -R "$repo" "$work/clone"
rm -rf "$work/clone/.turbo"
touch "$mirror/tamper"
home=$work/clone-home ci=1
run "$work/clone" setup --tools-only
home=$work/home ci=
rm "$mirror/tamper"
[ "$code" -ne 0 ] && contains "$output" 'does not match the trusted digest' ||
  fail "frozen setup accepted a tampered artifact: $output"
[ ! -e "$work/clone/.turbo/tools/manifest.json" ] || fail 'a tampered artifact was installed'
cmp -s "$work/clone/turbo.lock" "$work/turbo.lock" || fail 'frozen setup changed turbo.lock'

# Plain turbo run: managed pnpm dispatches, and the task's node is managed
# Node for this architecture, first on PATH, with env-mode filtering.
build() {
  offline "$1" run build "--env-mode=$2"
  contains "$output" 'fixture-pnpm 10.0.0 run build' || fail "managed pnpm did not run: $output"
  probe=$(printf '%s\n' "$output" | sed -n 's/.*managed-probe //p' | head -n 1)
  case "$probe" in
    "$repo/.turbo/tools/"*"|v$version|$arch|$repo/.turbo/tools/"*"|$3") ;;
    *) fail "unexpected $2 task environment: ${probe:-$output}" ;;
  esac
  unchanged run
}
for dir in "$repo" "$web"; do
  build "$dir" strict filtered
  build "$dir" loose present
done

# Repeats reuse the installation without network, including frozen CI.
offline "$repo" setup
contains "$output" 'Dependencies skipped' || fail "setup did not report skipped dependencies: $output"
ci=1
offline "$repo" setup --tools-only
ci=
offline "$repo" setup --tools-only --offline
offline "$repo" setup --check --tools-only --offline
unchanged setup

# Damage: run refuses before any task, --check fails, and setup repairs it.
bin=$(printf '%s\n' "$probe" | cut -d '|' -f 4)
rm "$bin/pnpm"
run "$repo" run build
[ "$code" -ne 0 ] && contains "$output" 'turbo setup' || fail "run accepted a damaged installation: $output"
! contains "$output" 'managed-probe' || fail "a task ran on a damaged installation: $output"
run "$repo" setup --check --tools-only --offline
[ "$code" -ne 0 ] || fail "setup --check accepted a damaged installation: $output"
succeeds "$repo" setup --tools-only
cmp -s "$repo/turbo.lock" "$work/turbo.lock" || fail 'repair changed turbo.lock'
cp "$repo/.turbo/tools/manifest.json" "$work/manifest.json"
build "$repo" strict filtered

printf 'Standalone setup journey passed: managed Node.js %s and pnpm 10.0.0 (%s).\n' "$version" "$arch"
